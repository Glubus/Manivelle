//! The program's end of the connection to the tool.
//!
//! Two threads serve it and neither touches the engine: one reads the tool's requests and
//! queues them, one writes what the engine thread hands it. The engine thread answers the
//! requests in `Engine::poll_protocol` and never waits on the network.

use std::io::{self, BufRead, BufReader, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::thread;
use std::time::Duration;

use super::wire::{Command, FromProgram};

/// How long a connection or a write to the tool may take before the program gives up on
/// the tool rather than stall.
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

/// A cheap handle that queues messages for the tool without ever waiting, for code that
/// outlives a borrow of the link, such as the `console` tap.
#[derive(Clone)]
pub(crate) struct Outbox(SyncSender<Outbound>);

impl Outbox {
    /// Queues `message`; `Err` once the tool cannot be reached any more. A full queue drops
    /// the message and is not an error.
    pub(crate) fn send(&self, message: FromProgram) -> Result<(), ()> {
        match self.0.try_send(Outbound::Message(message)) {
            Ok(()) | Err(TrySendError::Full(_)) => Ok(()),
            Err(TrySendError::Disconnected(_)) => Err(()),
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
}

impl Link {
    /// Connects to the tool at `address` and shows it `token`.
    pub(crate) fn connect(address: &str, token: &str) -> io::Result<Self> {
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

        let reader = BufReader::new(stream.try_clone()?);
        let (queue, commands) = mpsc::channel();
        let waiting = Arc::new(AtomicBool::new(false));
        let raise = Arc::clone(&waiting);
        let alive = Arc::new(AtomicBool::new(true));
        let reader_lost = Arc::clone(&alive);
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
                reader_lost.store(false, Ordering::Release);
            })?;

        let (queue_out, pending) = mpsc::sync_channel::<Outbound>(OUTBOX);
        let writer_lost = Arc::clone(&alive);
        thread::Builder::new()
            .name("manivelle-protocol-write".to_owned())
            .spawn(move || {
                // Ends with the first failed write: the senders then see a closed channel.
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
                writer_lost.store(false, Ordering::Release);
            })?;

        Ok(Self {
            outbox: Outbox(queue_out),
            commands,
            waiting,
            alive,
        })
    }

    /// Hands one message to the writing thread; `Err` once the tool cannot be reached
    /// any more. A full outbox drops the message and is not an error.
    pub(crate) fn send(&self, message: FromProgram) -> Result<(), ()> {
        self.outbox.send(message)
    }

    /// Whether the tool is still connected.
    pub(crate) fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Acquire)
    }

    /// A handle that queues messages like [`send`](Self::send).
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
    /// `console` lines of a program that ends right after them reach the tool. A program
    /// that ends without dropping its engine (`std::process::exit`) still loses them.
    fn drop(&mut self) {
        if !self.is_alive() {
            return;
        }
        let (done, flushed) = mpsc::channel();
        if self.outbox.0.try_send(Outbound::Flush(done)).is_ok() {
            let _ = flushed.recv_timeout(FLUSH);
        }
    }
}

fn write_line(stream: &mut TcpStream, message: &FromProgram) -> io::Result<()> {
    let mut line = serde_json::to_vec(message).map_err(io::Error::other)?;
    line.push(b'\n');
    stream.write_all(&line)?;
    stream.flush()
}
