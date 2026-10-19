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

pub(crate) struct Link {
    outbox: SyncSender<FromProgram>,
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

        let (outbox, pending) = mpsc::sync_channel::<FromProgram>(OUTBOX);
        let writer_lost = Arc::clone(&alive);
        thread::Builder::new()
            .name("manivelle-protocol-write".to_owned())
            .spawn(move || {
                // Ends with the first failed write: the senders then see a closed channel.
                for message in pending {
                    if write_line(&mut stream, &message).is_err() {
                        break;
                    }
                }
                writer_lost.store(false, Ordering::Release);
            })?;

        Ok(Self {
            outbox,
            commands,
            waiting,
            alive,
        })
    }

    /// Hands one message to the writing thread; `Err` once the tool cannot be reached
    /// any more. A full outbox drops the message and is not an error.
    pub(crate) fn send(&self, message: FromProgram) -> Result<(), ()> {
        match self.outbox.try_send(message) {
            Ok(()) | Err(TrySendError::Full(_)) => Ok(()),
            Err(TrySendError::Disconnected(_)) => Err(()),
        }
    }

    /// Whether the tool is still connected.
    pub(crate) fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Acquire)
    }

    /// A handle that sends like [`send`](Self::send), for code that outlives a borrow of
    /// the link, such as the `console` tap.
    pub(crate) fn outbox(&self) -> SyncSender<FromProgram> {
        self.outbox.clone()
    }

    /// The commands received since the last call; a single atomic read when none came.
    pub(crate) fn take_commands(&self) -> Vec<Command> {
        if !self.waiting.swap(false, Ordering::AcqRel) {
            return Vec::new();
        }
        self.commands.try_iter().collect()
    }
}

fn write_line(stream: &mut TcpStream, message: &FromProgram) -> io::Result<()> {
    let mut line = serde_json::to_vec(message).map_err(io::Error::other)?;
    line.push(b'\n');
    stream.write_all(&line)?;
    stream.flush()
}
