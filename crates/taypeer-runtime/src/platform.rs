//! Platform process boundary. The supervisor never performs command or Binder I/O.
use crate::RuntimeError;
use std::{
    io::{Read, Write},
    path::Path,
    process::{Child, Command, Stdio},
};

/// Nonblocking control of exactly one process generation.
/// Implementations must only inspect local state or enqueue termination here:
/// never wait for command locks, remote Binder calls, disk, or process exit.
/// A successful termination request is not proof of exit; `has_exited` supplies that proof.
pub trait ProcessHandle: Send {
    /// Observe OS-confirmed exit without blocking. An error is not an exit acknowledgement.
    fn has_exited(&mut self) -> Result<bool, RuntimeError>;
    /// Request forced termination without waiting for a command or graceful shutdown.
    fn terminate(&mut self) -> Result<(), RuntimeError>;
}

/// Private byte streams and independent process control for one worker.
/// The platform must stop the worker when the host dies, even during a blocked command.
pub struct ProcessConnection {
    pub(crate) control: Box<dyn ProcessHandle>,
    pub(crate) input: Box<dyn Write + Send>,
    pub(crate) output: Box<dyn Read + Send>,
}
impl ProcessConnection {
    /// Transfer ownership of an already isolated worker and its private streams.
    pub fn new(
        control: Box<dyn ProcessHandle>,
        input: Box<dyn Write + Send>,
        output: Box<dyn Read + Send>,
    ) -> Self {
        Self {
            control,
            input,
            output,
        }
    }
}

/// Launch one isolated worker. Binding/launching happens off the UI thread.
/// Returned streams must speak the private runtime protocol, not a public JSON API.
pub trait ProcessLauncher {
    /// Launch a fresh worker; never reuse a process that held an earlier document.
    fn launch(&self) -> Result<ProcessConnection, RuntimeError>;
}

/// Existing desktop executable adapter; launches the private `__worker` entry point.
pub struct DesktopLauncher<'a>(pub &'a Path);
impl ProcessLauncher for DesktopLauncher<'_> {
    fn launch(&self) -> Result<ProcessConnection, RuntimeError> {
        let child = Command::new(self.0)
            .arg("__worker")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| RuntimeError::Transport)?;
        let mut process = DesktopProcess(Some(child));
        let input = process
            .0
            .as_mut()
            .ok_or(RuntimeError::Transport)?
            .stdin
            .take()
            .ok_or(RuntimeError::Transport)?;
        let output = process
            .0
            .as_mut()
            .ok_or(RuntimeError::Transport)?
            .stdout
            .take()
            .ok_or(RuntimeError::Transport)?;
        Ok(ProcessConnection::new(
            Box::new(process),
            Box::new(input),
            Box::new(output),
        ))
    }
}
pub(crate) struct DesktopProcess(pub Option<Child>);
impl ProcessHandle for DesktopProcess {
    fn has_exited(&mut self) -> Result<bool, RuntimeError> {
        self.0
            .as_mut()
            .ok_or(RuntimeError::Transport)?
            .try_wait()
            .map(|status| status.is_some())
            .map_err(|_| RuntimeError::Transport)
    }
    fn terminate(&mut self) -> Result<(), RuntimeError> {
        self.0
            .as_mut()
            .ok_or(RuntimeError::Transport)?
            .kill()
            .map_err(|_| RuntimeError::Transport)
    }
}
impl Drop for DesktopProcess {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take()
            && !matches!(child.try_wait(), Ok(Some(_)))
        {
            // Setup failures also terminate and reap; never block the supervisor/UI.
            let _ = child.kill();
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
    }
}
