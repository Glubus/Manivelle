//! The program's end of the connection to the tool.
//!
//! Two threads serve it and neither touches the engine: one reads the tool's requests and
//! queues them, one writes what the engine thread hands it. The engine thread answers the
//! requests in `Engine::poll_protocol` and never waits on the network.

use std::io::{self, BufRead, BufReader, Write};
use std::net::{Shutdown, TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::thread;
use std::time::Duration;

use super::wire::{Command, FromProgram, FromTool};

/// How long a connection, the tool's welcome or a write to the tool may take before the
/// program gives up on the tool rather than stall.
const TIMEOUT: Duration = Duration::from_secs(5);

/// How many messages wait for the tool to read them. A tool that reads too slowly loses
/// the messages beyond it, which are mostly `console` lines.
const OUTBOX: usize = 4096;

/// How long dropping the link waits for the queued messages to be written.
const FLUSH: Duration = Duration::from_secs(2);

/// What the writing thread is handed.
enum Outbound {
    Message(FromProgram),
    /// Answered once every message before it is written.
    Flush(mpsc::Sender<()>),
}

/// The tool cannot be reached any more.
#[derive(Debug)]
pub(crate) struct Disconnected;

/// A cheap handle that queues messages for the tool, for code that outlives a borrow of
/// the link, such as the `console` tap.
#[derive(Clone)]
pub(crate) struct Outbox(SyncSender<Outbound>);

impl Outbox {
    /// Queues `message`, waiting for room when the queue is full; `Err` once the tool
    /// cannot be reached any more. The wait ends because a tool that stops reading makes
    /// the writing thread fail within [`TIMEOUT`], which closes the queue.
    pub(crate) fn send(&self, message: FromProgram) -> Result<(), Disconnected> {
        self.0
            .send(Outbound::Message(message))
            .map_err(|_| Disconnected)
    }

    /// Queues `message` without ever waiting: a full queue drops it and is not an error.
    /// Only for `console` lines; every other message goes through [`send`](Self::send).
    pub(crate) fn send_or_drop(&self, message: FromProgram) -> Result<(), Disconnected> {
        match self.0.try_send(Outbound::Message(message)) {
            Ok(()) | Err(TrySendError::Full(_)) => Ok(()),
            Err(TrySendError::Disconnected(_)) => Err(Disconnected),
        }
    }
}

pub(crate) struct Link {
    outbox: Outbox,
    commands: Receiver<Command>,
    /// Raised by the reading thread when a command is queued.
    waiting: Arc<AtomicBool>,
    /// Lowered by either thread when the connection ends.
    alive: Arc<AtomicBool>,
    /// A handle on the socket, to close it in order when the link drops.
    socket: TcpStream,
}

impl Link {
    /// Connects to the tool at `address`, shows it `token` and waits for its welcome.
    pub(crate) fn connect(address: &str, token: &str) -> io::Result<Self> {
        let (stream, reader) = handshake(address, token)?;
        let (queue, commands) = mpsc::channel();
        let waiting = Arc::new(AtomicBool::new(false));
        let alive = Arc::new(AtomicBool::new(true));
        spawn_reader(reader, queue, Arc::clone(&waiting), Arc::clone(&alive))?;
        let (queue_out, pending) = mpsc::sync_channel::<Outbound>(OUTBOX);
        let socket = stream.try_clone()?;
        spawn_writer(stream, pending, Arc::clone(&alive))?;
        Ok(Self {
            outbox: Outbox(queue_out),
            commands,
            waiting,
            alive,
            socket,
        })
    }

    /// Hands one message to the writing thread, waiting for room when the outbox is full;
    /// `Err` once the tool cannot be reached any more.
    pub(crate) fn send(&self, message: FromProgram) -> Result<(), Disconnected> {
        self.outbox.send(message)
    }

    /// Whether the tool is still connected.
    pub(crate) fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Acquire)
    }

    /// A handle that queues messages like [`send_or_drop`](Outbox::send_or_drop) and
    /// [`send`](Outbox::send).
    pub(crate) fn outbox(&self) -> Outbox {
        self.outbox.clone()
    }

    /// The commands received since the last call; a single atomic read when none came.
    pub(crate) fn take_commands(&self) -> Vec<Command> {
        // A plain read while idle: the read-modify-write below is what costs.
        if !self.waiting.load(Ordering::Acquire) || !self.waiting.swap(false, Ordering::AcqRel) {
            return Vec::new();
        }
        self.commands.try_iter().collect()
    }
}

impl Drop for Link {
    /// Gives the writing thread a moment to write what is queued, so that the last
    /// `console` lines of a program that ends right after them reach the tool, then closes
    /// the sending side: on Windows a program that ends with its socket still open resets
    /// the connection, and the tool loses what it had not read yet. A program that ends
    /// without dropping its engine (`std::process::exit`) still loses them. A full queue
    /// is waited on, within [`FLUSH`] in all.
    fn drop(&mut self) {
        if self.is_alive() {
            let (done, flushed) = mpsc::channel();
            let outbox = self.outbox.clone();
            // A thread, because the standard queue has no send with a timeout: it waits
            // for room, and ends when the flush is queued or the writing thread is gone.
            let queued = thread::Builder::new()
                .name("manivelle-protocol-flush".to_owned())
                .spawn(move || {
                    let _ = outbox.0.send(Outbound::Flush(done));
                });
            if queued.is_ok() {
                let _ = flushed.recv_timeout(FLUSH);
            }
        }
        let _ = self.socket.shutdown(Shutdown::Write);
    }
}

fn write_line(stream: &mut TcpStream, message: &FromProgram) -> io::Result<()> {
    let mut line = serde_json::to_vec(message).map_err(io::Error::other)?;
    line.push(b'\n');
    stream.write_all(&line)?;
    stream.flush()
}

/// Opens the connection, shows the tool `token` and waits for its welcome, returning the
/// reader that holds whatever the tool sent after it. The program goes on only once the
/// tool has read the greeting: a program that ended before, on Windows, would reset a
/// connection the tool had not read yet, and the tool would lose the greeting.
fn handshake(address: &str, token: &str) -> io::Result<(TcpStream, BufReader<TcpStream>)> {
    let target = address
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| io::Error::other(format!("no address for {address}")))?;
    let mut stream = TcpStream::connect_timeout(&target, TIMEOUT)?;
    stream.set_write_timeout(Some(TIMEOUT))?;
    stream.set_nodelay(true)?;
    write_line(
        &mut stream,
        &FromProgram::Hello {
            token: token.to_owned(),
            pid: std::process::id(),
            manivelle: env!("CARGO_PKG_VERSION").to_owned(),
        },
    )?;
    let mut reader = BufReader::new(stream.try_clone()?);
    await_welcome(&stream, &mut reader)?;
    Ok((stream, reader))
}

/// Reads the tool's first line, which must be its welcome, within [`TIMEOUT`]. The tool
/// closing the connection instead means it did not accept the greeting.
fn await_welcome(stream: &TcpStream, reader: &mut BufReader<TcpStream>) -> io::Result<()> {
    stream.set_read_timeout(Some(TIMEOUT))?;
    let mut line = String::new();
    let read = reader
        .read_line(&mut line)
        .map_err(|error| match error.kind() {
            // A read that timed out is `WouldBlock` on Unix and `TimedOut` on Windows.
            io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut => io::Error::new(
                io::ErrorKind::TimedOut,
                "the tool did not welcome the program in time",
            ),
            _ => error,
        })?;
    if read == 0 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "the tool closed the connection without welcoming the program: is the secret wrong?",
        ));
    }
    match serde_json::from_str(&line) {
        Ok(FromTool::Welcome) => {}
        Err(error) => return Err(io::Error::new(io::ErrorKind::InvalidData, error)),
    }
    // The reading thread waits for requests with no time limit.
    stream.set_read_timeout(None)
}

/// Reads the tool's requests and queues them. Raises `raise` with each one, and lowers
/// `alive` when the connection ends.
fn spawn_reader(
    reader: BufReader<TcpStream>,
    queue: mpsc::Sender<Command>,
    raise: Arc<AtomicBool>,
    alive: Arc<AtomicBool>,
) -> io::Result<()> {
    thread::Builder::new()
        .name("manivelle-protocol-read".to_owned())
        .spawn(move || {
            for line in reader.lines().map_while(Result::ok) {
                let Ok(command) = serde_json::from_str::<Command>(&line) else {
                    continue;
                };
                if queue.send(command).is_err() {
                    break;
                }
                raise.store(true, Ordering::Release);
            }
            alive.store(false, Ordering::Release);
        })?;
    Ok(())
}

/// Writes the queued messages in order. Lowers `alive` at the first failed write; the
/// senders then see a closed channel.
fn spawn_writer(
    mut stream: TcpStream,
    pending: Receiver<Outbound>,
    alive: Arc<AtomicBool>,
) -> io::Result<()> {
    thread::Builder::new()
        .name("manivelle-protocol-write".to_owned())
        .spawn(move || {
            for item in pending {
                match item {
                    Outbound::Message(message) => {
                        if write_line(&mut stream, &message).is_err() {
                            break;
                        }
                    }
                    Outbound::Flush(done) => {
                        let _ = done.send(());
                    }
                }
            }
            alive.store(false, Ordering::Release);
        })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;

    use super::super::wire::Reply;
    use super::*;

    fn console(n: usize) -> FromProgram {
        FromProgram::Console {
            level: "log".to_owned(),
            script: "mod".to_owned(),
            message: n.to_string(),
        }
    }

    /// The tool's end of a handshake: accepts the program and reads its greeting, and holds
    /// the connection until it chooses to answer.
    fn accept_greeting(listener: &TcpListener) -> TcpStream {
        let (stream, _) = listener.accept().expect("accept");
        let mut reader = BufReader::new(stream.try_clone().expect("clone"));
        let mut line = String::new();
        reader.read_line(&mut line).expect("read the greeting");
        assert!(matches!(
            serde_json::from_str(&line),
            Ok(FromProgram::Hello { .. })
        ));
        stream
    }

    #[test]
    fn connect_returns_only_once_the_tool_welcomed_the_program() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listen");
        let address = listener.local_addr().expect("address").to_string();
        let (returned, connecting) = mpsc::channel();
        let program = thread::spawn(move || {
            let link = Link::connect(&address, "secret");
            returned.send(()).expect("test is waiting");
            link
        });

        // A slow tool: it has read the greeting and answers later. Until it does, the
        // program must not go on, or it could end and reset the connection before the tool
        // had read anything: on Windows the tool then loses the greeting.
        let mut stream = accept_greeting(&listener);
        assert!(
            connecting.recv_timeout(Duration::from_millis(300)).is_err(),
            "connect returned before the tool welcomed the program"
        );

        let welcome = serde_json::to_string(&FromTool::Welcome).expect("encode the welcome");
        writeln!(stream, "{welcome}").expect("welcome");
        connecting
            .recv_timeout(Duration::from_secs(5))
            .expect("connect returns once welcomed");
        assert!(program.join().expect("program thread").is_ok());
    }

    #[test]
    fn connect_fails_when_the_tool_closes_without_a_welcome() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listen");
        let address = listener.local_addr().expect("address").to_string();
        let program = thread::spawn(move || Link::connect(&address, "wrong").map(|_| ()));

        drop(accept_greeting(&listener));

        let error = program
            .join()
            .expect("program thread")
            .expect_err("a tool that drops the program has not welcomed it");
        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn a_reply_waits_for_room_while_console_lines_are_dropped() {
        let (queue, pending) = mpsc::sync_channel(2);
        let outbox = Outbox(queue);
        for n in 0..5 {
            outbox.send_or_drop(console(n)).expect("connected");
        }

        // The queue holds two console lines and the other three are gone. The reply is
        // not dropped: it waits until the writing side takes a line. The writing side
        // here is slow, as a tool that reads late: it starts once the sender has begun.
        let sender = outbox.clone();
        let (begun, sending) = mpsc::channel();
        let reply = thread::spawn(move || {
            begun.send(()).expect("test is waiting");
            sender.send(FromProgram::Reply {
                id: 7,
                reply: Reply::Done,
            })
        });
        drop(outbox);
        sending.recv().expect("sender began");
        thread::sleep(Duration::from_millis(100));
        let received: Vec<Outbound> = pending.iter().collect();

        assert!(reply.join().expect("sender thread").is_ok());
        assert!(
            matches!(
                received.as_slice(),
                [
                    Outbound::Message(FromProgram::Console { .. }),
                    Outbound::Message(FromProgram::Console { .. }),
                    Outbound::Message(FromProgram::Reply { id: 7, .. }),
                ]
            ),
            "{} messages",
            received.len()
        );
    }

    #[test]
    fn dropping_the_link_waits_for_room_to_flush_a_full_queue() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listen");
        let socket = TcpStream::connect(listener.local_addr().expect("address")).expect("connect");
        let (_tool, _) = listener.accept().expect("accept");
        let (queue, pending) = mpsc::sync_channel(1);
        let (_requests, commands) = mpsc::channel();
        let link = Link {
            outbox: Outbox(queue),
            commands,
            waiting: Arc::new(AtomicBool::new(false)),
            alive: Arc::new(AtomicBool::new(true)),
            socket,
        };
        link.outbox().send_or_drop(console(0)).expect("connected");

        // A slow writing thread: the queue is full when the link drops, and has room
        // a little later. The flush must wait for it, and its answer ends the drop.
        let writer = thread::spawn(move || {
            thread::sleep(Duration::from_millis(100));
            let line = matches!(pending.recv(), Ok(Outbound::Message(_)));
            let flush = match pending.recv() {
                Ok(Outbound::Flush(done)) => done.send(()).is_ok(),
                _ => false,
            };
            (line, flush)
        });
        drop(link);

        assert_eq!(writer.join().expect("writer thread"), (true, true));
    }
}
