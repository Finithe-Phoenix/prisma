//! Owned lifecycle for a Windows-style console child process.
//!
//! A session always pipes all three standard streams. Background readers drain
//! stdout and stderr so a verbose guest cannot deadlock while the caller is
//! waiting for it. The session remains the owner of every pipe and reader
//! thread until [`ConsoleSession::wait`], [`ConsoleSession::stop`], or `Drop`
//! performs deterministic cleanup.

use std::ffi::OsStr;
use std::fmt;
use std::io::{self, Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

/// Builder for a console session.
#[derive(Debug)]
pub struct ConsoleSessionBuilder {
    command: Command,
}

impl ConsoleSessionBuilder {
    /// Creates a builder for `program`.
    pub fn new(program: impl AsRef<OsStr>) -> Self {
        Self {
            command: Command::new(program),
        }
    }

    /// Appends one command-line argument.
    pub fn arg(&mut self, arg: impl AsRef<OsStr>) -> &mut Self {
        self.command.arg(arg);
        self
    }

    /// Appends command-line arguments.
    pub fn args<I, S>(&mut self, args: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.command.args(args);
        self
    }

    /// Sets the child working directory.
    pub fn current_dir(&mut self, dir: impl AsRef<Path>) -> &mut Self {
        self.command.current_dir(dir);
        self
    }

    /// Sets one child environment variable.
    pub fn env(&mut self, key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> &mut Self {
        self.command.env(key, value);
        self
    }

    /// Sets child environment variables.
    pub fn envs<I, K, V>(&mut self, vars: I) -> &mut Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        self.command.envs(vars);
        self
    }

    /// Removes one inherited child environment variable.
    pub fn env_remove(&mut self, key: impl AsRef<OsStr>) -> &mut Self {
        self.command.env_remove(key);
        self
    }

    /// Prevents the child from inheriting the parent environment.
    pub fn env_clear(&mut self) -> &mut Self {
        self.command.env_clear();
        self
    }

    /// Spawns the configured process with owned standard-stream pipes.
    pub fn spawn(&mut self) -> io::Result<ConsoleSession> {
        self.command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = self.command.spawn()?;
        let (Some(stdin), Some(stdout), Some(stderr)) =
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            cleanup_failed_child(&mut child);
            return Err(io::Error::other(
                "spawned console process did not provide all three standard-stream pipes",
            ));
        };

        let stdout = match PipeCapture::spawn(stdout, "prisma-console-stdout") {
            Ok(capture) => capture,
            Err(error) => {
                cleanup_failed_spawn(&mut child, stdin);
                return Err(error);
            }
        };
        let stderr = match PipeCapture::spawn(stderr, "prisma-console-stderr") {
            Ok(capture) => capture,
            Err(error) => {
                cleanup_failed_spawn(&mut child, stdin);
                let mut stdout = stdout;
                let _ = stdout.finish();
                return Err(error);
            }
        };

        Ok(ConsoleSession {
            child: Some(child),
            stdin: Some(stdin),
            stdout,
            stderr,
            status: None,
        })
    }
}

/// A running or completed console child and all resources associated with it.
#[derive(Debug)]
pub struct ConsoleSession {
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    stdout: PipeCapture,
    stderr: PipeCapture,
    status: Option<ExitStatus>,
}

/// Failure returned when a console capability is unavailable for this transport.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConsoleResizeError {
    /// Ordinary redirected pipes do not expose a pseudo-terminal to resize.
    Unsupported,
}

impl fmt::Display for ConsoleResizeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported => formatter.write_str(
                "console resize is unsupported for redirected pipes; a PTY backend is required",
            ),
        }
    }
}

impl std::error::Error for ConsoleResizeError {}

impl ConsoleSession {
    /// Creates a builder for a console process.
    pub fn builder(program: impl AsRef<OsStr>) -> ConsoleSessionBuilder {
        ConsoleSessionBuilder::new(program)
    }

    /// Spawns `program` without arguments.
    pub fn spawn(program: impl AsRef<OsStr>) -> io::Result<Self> {
        Self::builder(program).spawn()
    }

    /// Returns the operating-system process identifier while the child handle is live.
    #[must_use]
    pub fn id(&self) -> Option<u32> {
        self.child.as_ref().map(Child::id)
    }

    /// Writes bytes to the child stdin and flushes the pipe.
    pub fn write_stdin(&mut self, bytes: impl AsRef<[u8]>) -> io::Result<()> {
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "console stdin is closed"))?;
        stdin.write_all(bytes.as_ref())?;
        stdin.flush()
    }

    /// Closes the child stdin pipe, allowing readers to observe end-of-file.
    pub fn close_stdin(&mut self) {
        drop(self.stdin.take());
    }

    /// Returns all stdout bytes captured so far.
    #[must_use]
    pub fn stdout_snapshot(&self) -> Vec<u8> {
        self.stdout.snapshot()
    }

    /// Returns all stderr bytes captured so far.
    #[must_use]
    pub fn stderr_snapshot(&self) -> Vec<u8> {
        self.stderr.snapshot()
    }

    /// Checks whether the child has exited, reaping it and its pipes when complete.
    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        if let Some(status) = self.status {
            return Ok(Some(status));
        }

        let Some(child) = self.child.as_mut() else {
            return Ok(self.status);
        };
        let Some(status) = child.try_wait()? else {
            return Ok(None);
        };

        self.close_stdin();
        self.child.take();
        self.status = Some(status);
        self.finish_readers()?;
        Ok(Some(status))
    }

    /// Waits for normal process exit and deterministically reaps all resources.
    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        self.finish(false)
    }

    /// Terminates a running child, waits for it, and closes every owned resource.
    pub fn stop(&mut self) -> io::Result<ExitStatus> {
        self.finish(true)
    }

    /// Requests a terminal viewport resize.
    ///
    /// This pipe-backed session deliberately reports [`ConsoleResizeError::Unsupported`]
    /// because changing dimensions requires a real ConPTY/PTY transport.
    pub const fn resize(&mut self, _columns: u16, _rows: u16) -> Result<(), ConsoleResizeError> {
        Err(ConsoleResizeError::Unsupported)
    }

    /// Returns the final status after `wait`, `stop`, or a completed `try_wait`.
    #[must_use]
    pub const fn status(&self) -> Option<ExitStatus> {
        self.status
    }

    fn finish(&mut self, terminate: bool) -> io::Result<ExitStatus> {
        if let Some(status) = self.status {
            return Ok(status);
        }

        let mut child = self.child.take().ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "console child is not available")
        })?;

        let status = if terminate {
            if let Ok(Some(status)) = child.try_wait() {
                status
            } else {
                let _ = child.kill();
                self.close_stdin();
                match child.wait() {
                    Ok(status) => status,
                    Err(error) => {
                        self.child = Some(child);
                        return Err(error);
                    }
                }
            }
        } else {
            self.close_stdin();
            match child.wait() {
                Ok(status) => status,
                Err(error) => {
                    self.child = Some(child);
                    return Err(error);
                }
            }
        };

        self.close_stdin();
        self.status = Some(status);
        self.finish_readers()?;
        Ok(status)
    }

    fn finish_readers(&mut self) -> io::Result<()> {
        let stdout_result = self.stdout.finish();
        let stderr_result = self.stderr.finish();
        stdout_result.and(stderr_result)
    }
}

impl Drop for ConsoleSession {
    fn drop(&mut self) {
        if self.status.is_none() {
            let _ = self.stop();
        } else {
            self.close_stdin();
            let _ = self.finish_readers();
        }
    }
}

#[derive(Debug)]
struct PipeCapture {
    bytes: Arc<Mutex<Vec<u8>>>,
    error: Arc<Mutex<Option<(io::ErrorKind, String)>>>,
    reader: Option<JoinHandle<()>>,
}

impl PipeCapture {
    fn spawn(mut pipe: impl Read + Send + 'static, thread_name: &'static str) -> io::Result<Self> {
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let error = Arc::new(Mutex::new(None));
        let reader_bytes = Arc::clone(&bytes);
        let reader_error = Arc::clone(&error);
        let reader = thread::Builder::new()
            .name(thread_name.to_owned())
            .spawn(move || {
                let mut chunk = [0_u8; 8192];
                loop {
                    match pipe.read(&mut chunk) {
                        Ok(0) => break,
                        Ok(read) => lock(&reader_bytes).extend_from_slice(&chunk[..read]),
                        Err(read_error) if read_error.kind() == io::ErrorKind::Interrupted => {}
                        Err(read_error) => {
                            *lock(&reader_error) =
                                Some((read_error.kind(), read_error.to_string()));
                            break;
                        }
                    }
                }
            })?;

        Ok(Self {
            bytes,
            error,
            reader: Some(reader),
        })
    }

    fn snapshot(&self) -> Vec<u8> {
        lock(&self.bytes).clone()
    }

    fn finish(&mut self) -> io::Result<()> {
        if let Some(reader) = self.reader.take() {
            reader
                .join()
                .map_err(|_| io::Error::other("console pipe reader thread panicked"))?;
        }
        let read_error = lock(&self.error).take();
        if let Some((kind, message)) = read_error {
            return Err(io::Error::new(kind, message));
        }
        Ok(())
    }
}

fn cleanup_failed_spawn(child: &mut Child, stdin: ChildStdin) {
    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();
}

fn cleanup_failed_child(child: &mut Child) {
    drop(child.stdin.take());
    drop(child.stdout.take());
    drop(child.stderr.take());
    let _ = child.kill();
    let _ = child.wait();
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
