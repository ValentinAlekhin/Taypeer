//! Bounded private IPC and a process owner for one unlocked database.
//!
//! The parent receives limited views, never the full document or a read key. A host
//! must explicitly close a worker to observe draft errors; Drop only terminates it.

mod protocol;
mod worker;

pub use protocol::{Command, RuntimeError, erase_view};
pub use worker::run_worker;

use protocol::{Boot, Response, read_frame, write_frame};
use serde_json::Value;
use std::{
    io::BufReader,
    path::Path,
    process::{Child, ChildStdin, ChildStdout, Command as ProcessCommand, Stdio},
};
use taypeer_core::DatabaseId;
use zeroize::Zeroize;

/// One child process and its private pipes. It is the sole owner of its file writer.
pub struct Worker {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    database: DatabaseId,
    open: bool,
}

impl Worker {
    /// Start a worker through the host executable's hidden `__worker` entry point.
    /// Credentials travel over an inherited private pipe, never argv or environment.
    pub fn open(
        executable: &Path,
        path: &Path,
        password: String,
        create_name: Option<String>,
    ) -> Result<Self, RuntimeError> {
        let mut boot = Boot {
            path: path.to_owned(),
            password,
            create_name,
        };
        let mut child = ProcessCommand::new(executable)
            .arg("__worker")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // Parser/panic diagnostics are not a user-data channel.
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| RuntimeError::Transport)?;
        let setup = (|| {
            let mut input = child.stdin.take().ok_or(RuntimeError::Transport)?;
            let mut output = BufReader::new(child.stdout.take().ok_or(RuntimeError::Transport)?);
            write_frame(&mut input, &boot)?;
            boot.password.zeroize();
            let response: Response = read_frame(&mut output)?;
            let value = response.into_result()?;
            let database = serde_json::from_value(value).map_err(|_| RuntimeError::Protocol)?;
            Ok((input, output, database))
        })();
        boot.password.zeroize();
        match setup {
            Ok((input, output, database)) => Ok(Self {
                child,
                input,
                output,
                database,
                open: true,
            }),
            Err(error) => {
                terminate(&mut child);
                Err(error)
            }
        }
    }

    /// Stable database identity returned by the authenticated document.
    pub fn database_id(&self) -> &DatabaseId {
        &self.database
    }

    /// Execute one command. Transport failure invalidates this worker permanently.
    pub fn request(&mut self, command: &Command) -> Result<Value, RuntimeError> {
        if !self.open {
            return Err(RuntimeError::Closed);
        }
        let response = (|| {
            write_frame(&mut self.input, command)?;
            read_frame::<Response>(&mut self.output)
        })();
        match response {
            Ok(response) => response.into_result(),
            Err(error) => {
                self.open = false;
                terminate(&mut self.child);
                Err(error)
            }
        }
    }

    /// Invalidate access, save an interrupted draft, and wait for process termination.
    /// The process also terminates when draft persistence fails.
    pub fn close(&mut self) -> Result<(), RuntimeError> {
        if !self.open {
            return Ok(());
        }
        let result = self.request(&Command::Lock).map(|_| ());
        self.open = false;
        let ended = self.child.wait().map_err(|_| RuntimeError::Transport);
        result.and(ended.and_then(|status| {
            if status.success() {
                Ok(())
            } else {
                Err(RuntimeError::Transport)
            }
        }))
    }
}

fn terminate(child: &mut Child) {
    // A dead/broken peer cannot acknowledge draft persistence. Reap it regardless
    // of whether it has already exited; the caller reports the original error.
    let _ = child.kill();
    let _ = child.wait();
}

impl Drop for Worker {
    fn drop(&mut self) {
        terminate(&mut self.child);
    }
}
