//! The pause controller: the test kit's end of the pause protocol (ARCHITECTURE.md, Test kit: "The pause protocol").
//!
//! A maitred process reaching a point named in `MAITRED_TEST_PAUSE_AT` connects to the socket named in
//! `MAITRED_TEST_PAUSE`, writes `at <point> pid <pid> ticket <id|->`, and blocks until it reads `go`. The runner
//! takes each paused process with [`Controller::next`], does its fault, and lets it go. Each paused process holds its
//! own connection, so several can wait at once; one dropped without `go` reads end of file and continues.

use std::fs;
use std::io::{self, BufRead, BufReader, ErrorKind, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use maitred_scenario::PausePoint;

use crate::timing::pause_accept_retry;

/// The longest socket path every platform takes: macOS's `sun_path` is 104 bytes with its terminating NUL.
const MAX_SOCKET_PATH: usize = 103;

static NEXT: AtomicU32 = AtomicU32::new(0);

/// Listens for paused processes on a socket of its own, in a directory of its own that goes when it does.
#[derive(Debug)]
pub struct Controller {
    listener: UnixListener,
    dir: PathBuf,
    path: PathBuf,
}

/// One process held at a pause point. Dropping it without [`Paused::go`] closes its connection, and the process
/// continues as it would with the controller gone.
#[derive(Debug)]
pub struct Paused {
    pub point: PausePoint,
    pub pid: u32,
    /// `None` when the process paused before its ticket existed.
    pub ticket: Option<String>,
    stream: UnixStream,
}

impl Paused {
    /// Lets the process continue.
    ///
    /// # Errors
    ///
    /// If the process has gone, so the reply can't be written.
    pub fn go(mut self) -> io::Result<()> {
        self.stream.write_all(b"go\n")
    }
}

impl Controller {
    /// A controller under the system's temporary directory.
    ///
    /// # Errors
    ///
    /// As [`Controller::listen_in`].
    pub fn listen() -> io::Result<Self> {
        Self::listen_in(&std::env::temp_dir())
    }

    /// A controller in a new directory under `parent`.
    ///
    /// # Errors
    ///
    /// `InvalidInput` if the socket's path would be too long for some platform to bind, before anything is made;
    /// otherwise any error making the directory or binding the socket.
    pub fn listen_in(parent: &Path) -> io::Result<Self> {
        let dir = parent.join(format!(
            "mtp-{}-{}",
            process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        let path = dir.join("s");
        if path.as_os_str().len() > MAX_SOCKET_PATH {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                format!(
                    "{} is over {MAX_SOCKET_PATH} bytes; listen under a shorter directory",
                    path.display()
                ),
            ));
        }
        fs::create_dir(&dir)?;
        let listener = UnixListener::bind(&path)?;
        listener.set_nonblocking(true)?;
        Ok(Self {
            listener,
            dir,
            path,
        })
    }

    /// The socket's path, for `MAITRED_TEST_PAUSE`.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The two variables a maitred process needs to pause at `points`, in the order given.
    #[must_use]
    pub fn env(&self, points: &[PausePoint]) -> [(String, String); 2] {
        let names: Vec<&str> = points.iter().map(|point| point.name()).collect();
        [
            (
                "MAITRED_TEST_PAUSE".into(),
                self.path.to_string_lossy().into_owned(),
            ),
            ("MAITRED_TEST_PAUSE_AT".into(), names.join(",")),
        ]
    }

    /// The next process to pause, waiting up to `timeout` for it to connect and say where it is.
    ///
    /// # Errors
    ///
    /// `TimedOut` if none does in time; `InvalidData` if its line isn't the protocol's, or names a point the doc
    /// doesn't list.
    pub fn next(&self, timeout: Duration) -> io::Result<Paused> {
        let deadline = Instant::now() + timeout;
        let stream = loop {
            match self.listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        return Err(ErrorKind::TimedOut.into());
                    }
                    thread::sleep(pause_accept_retry());
                }
                Err(error) => return Err(error),
            }
        };
        stream.set_nonblocking(false)?;
        stream.set_read_timeout(Some(
            deadline
                .saturating_duration_since(Instant::now())
                .max(pause_accept_retry()),
        ))?;
        let mut line = String::new();
        BufReader::new(stream.try_clone()?).read_line(&mut line)?;
        let (point, pid, ticket) = parse(&line).ok_or_else(|| {
            io::Error::new(
                ErrorKind::InvalidData,
                format!("not a pause line: {line:?}"),
            )
        })?;
        Ok(Paused {
            point,
            pid,
            ticket,
            stream,
        })
    }
}

impl Drop for Controller {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
        let _ = fs::remove_dir(&self.dir);
    }
}

/// `at <point> pid <pid> ticket <id|->`, with its newline.
fn parse(line: &str) -> Option<(PausePoint, u32, Option<String>)> {
    let words: Vec<&str> = line.strip_suffix('\n')?.split(' ').collect();
    let ["at", point, "pid", pid, "ticket", ticket] = words.as_slice() else {
        return None;
    };
    let ticket = (*ticket != "-").then(|| (*ticket).to_string());
    Some((PausePoint::named(point)?, pid.parse().ok()?, ticket))
}
