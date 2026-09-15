//! Private command IPC and independent lifetimes for plaintext workers and ciphertext writers.
mod cipher_ipc;
mod host;
mod network;
mod protocol;
mod worker;
pub use network::{InvitationCode, JoinProgress, PeerProgress, PendingJoin};
pub mod profile;

pub use host::{RegisteredCompatibility, RuntimeHost};
pub use protocol::{Command, RuntimeError, erase_view};
pub use worker::run_worker;

use cipher_ipc::{IoReply, WorkerMessage};
use host::{Callbacks, HostContext};
use protocol::{Boot, read_frame, write_frame};
use serde_json::Value;
use std::{
    io::BufReader,
    path::Path,
    process::{Child, ChildStdin, ChildStdout, Command as ProcessCommand, Stdio},
    sync::{Arc, Mutex},
};
use taypeer_core::DatabaseId;
use taypeer_sync::CoordinatorEvent;
use zeroize::Zeroize;

/// One child process and private pipes, with an authority listener that closes stale workers
/// even while the CLI is waiting for user input. The host retains ciphertext storage separately.
pub struct Worker {
    process: Arc<Mutex<WorkerProcess>>,
    database: DatabaseId,
    watch: tokio::task::JoinHandle<()>,
}
struct WorkerProcess {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    callbacks: Callbacks,
    _spool: tempfile::TempDir,
    open: bool,
    closure_error: Option<RuntimeError>,
    application: Option<Result<Value, RuntimeError>>,
}
impl Worker {
    pub(crate) fn enroll(
        executable: &Path,
        profile: &profile::NativeProfile,
        invitation: taypeer_trust::Invitation,
    ) -> Result<taypeer_trust::JoinProof, RuntimeError> {
        let mut child = ProcessCommand::new(executable)
            .arg("__worker")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| RuntimeError::Transport)?;
        let result = (|| {
            let mut input = child.stdin.take().ok_or(RuntimeError::Transport)?;
            let mut output = child.stdout.take().ok_or(RuntimeError::Transport)?;
            let boot = Boot {
                path: profile.directory().to_owned(),
                password: String::new(),
                create_name: None,
                profile: profile.directory().to_owned(),
                spool: profile.directory().to_owned(),
                invitation: Some(invitation),
            };
            write_frame(&mut input, &boot)?;
            let WorkerMessage::Response(response) = read_frame(&mut output)? else {
                return Err(RuntimeError::Protocol);
            };
            serde_json::from_value(response.into_result()?).map_err(|_| RuntimeError::Protocol)
        })();
        terminate(&mut child);
        result
    }
    pub(crate) fn open(
        executable: &Path,
        path: &Path,
        password: String,
        create_name: Option<String>,
        context: Arc<HostContext>,
        runtime: &tokio::runtime::Handle,
    ) -> Result<Self, RuntimeError> {
        let spool = tempfile::tempdir().map_err(|_| RuntimeError::Transport)?;
        let directory = spool
            .path()
            .canonicalize()
            .map_err(|_| RuntimeError::Transport)?;
        let mut boot = Boot {
            path: path.to_owned(),
            password,
            create_name,
            profile: context.profile.directory().to_owned(),
            spool: directory.clone(),
            invitation: None,
        };
        let mut events = context.coordinator.subscribe();
        let mut child = ProcessCommand::new(executable)
            .arg("__worker")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| RuntimeError::Transport)?;
        let channels = match (child.stdin.take(), child.stdout.take()) {
            (Some(input), Some(output)) => (input, BufReader::new(output)),
            _ => {
                terminate(&mut child);
                return Err(RuntimeError::Transport);
            }
        };
        let mut process = WorkerProcess {
            child,
            input: channels.0,
            output: channels.1,
            callbacks: Callbacks::new(context, path.to_owned(), directory),
            _spool: spool,
            open: true,
            closure_error: None,
            application: None,
        };
        let setup = (|| {
            write_frame(&mut process.input, &boot)?;
            boot.password.zeroize();
            let value = process.response()?;
            serde_json::from_value::<DatabaseId>(value).map_err(|_| RuntimeError::Protocol)
        })();
        boot.password.zeroize();
        let database = match setup {
            Ok(id) => id,
            Err(error) => {
                terminate(&mut process.child);
                return Err(error);
            }
        };
        let process = Arc::new(Mutex::new(process));
        let weak = Arc::downgrade(&process);
        let watched_database = database.clone();
        let watch = runtime.spawn(async move {
            loop {
                let (invalidate, apply) = match events.recv().await {
                    Ok(
                        CoordinatorEvent::ControlChanged {
                            database,
                            lock: true,
                            ..
                        }
                        | CoordinatorEvent::Frozen(database),
                    ) => (database == watched_database, false),
                    Ok(
                        CoordinatorEvent::Received { database, .. }
                        | CoordinatorEvent::ControlChanged {
                            database,
                            lock: false,
                            ..
                        },
                    ) => (false, database == watched_database),
                    Ok(_) => (false, false),
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => (true, false),
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                };
                if invalidate {
                    if let Some(process) = weak.upgrade() {
                        // Pipe and filesystem work belongs outside the async executor.
                        let _ = tokio::task::spawn_blocking(move || {
                            if let Ok(mut process) = process.lock() {
                                let _ = process.close();
                            }
                        })
                        .await;
                    }
                    break;
                }
                if apply && let Some(process) = weak.upgrade() {
                    let _ = tokio::task::spawn_blocking(move || {
                        if let Ok(mut process) = process.lock()
                            && process.open
                        {
                            let result = process.request(&Command::ApplyReceived);
                            process.application = Some(result);
                        }
                    })
                    .await;
                }
            }
        });
        Ok(Self {
            process,
            database,
            watch,
        })
    }
    /// Stable logical database identity authenticated by the opened document.
    pub fn database_id(&self) -> &DatabaseId {
        &self.database
    }
    /// Whether the authority listener has kept this plaintext process current.
    pub fn is_open(&self) -> bool {
        self.process.lock().is_ok_and(|process| process.open)
    }
    /// Most recent automatic application attempt, distinct from network receipt counts.
    pub fn application_progress(&self) -> Option<Result<Value, RuntimeError>> {
        self.process
            .lock()
            .ok()
            .and_then(|process| process.application.clone())
    }
    /// Execute one limited service command. A broken IPC channel permanently closes the worker.
    pub fn request(&mut self, command: &Command) -> Result<Value, RuntimeError> {
        self.process
            .lock()
            .map_err(|_| RuntimeError::Closed)?
            .request(command)
    }
    /// Save the interrupted editor, invalidate the session and reap the child, even on failure.
    pub fn close(&mut self) -> Result<(), RuntimeError> {
        self.watch.abort();
        self.process
            .lock()
            .map_err(|_| RuntimeError::Closed)?
            .close()
    }
}
impl WorkerProcess {
    fn response(&mut self) -> Result<Value, RuntimeError> {
        loop {
            match read_frame::<WorkerMessage>(&mut self.output)? {
                WorkerMessage::Response(response) => return response.into_result(),
                WorkerMessage::Io(request) => {
                    let result = self.callbacks.handle(*request);
                    write_frame(&mut self.input, &IoReply { result })?;
                }
            }
        }
    }
    fn request(&mut self, command: &Command) -> Result<Value, RuntimeError> {
        if !self.open {
            return Err(RuntimeError::Closed);
        }
        let result = (|| {
            write_frame(&mut self.input, command)?;
            self.response()
        })();
        if matches!(
            result,
            Err(RuntimeError::Transport | RuntimeError::Protocol | RuntimeError::TooLarge)
        ) {
            self.open = false;
            terminate(&mut self.child);
        }
        result
    }
    fn close(&mut self) -> Result<(), RuntimeError> {
        if !self.open {
            return self.closure_error.take().map_or(Ok(()), Err);
        }
        let result = self.request(&Command::Lock).map(|_| ());
        self.open = false;
        let ended = self.child.wait().map_err(|_| RuntimeError::Transport);
        let result = result.and(ended.and_then(|status| {
            if status.success() {
                Ok(())
            } else {
                Err(RuntimeError::Transport)
            }
        }));
        self.closure_error = result.as_ref().err().copied();
        result
    }
}
fn terminate(child: &mut Child) {
    // A broken peer cannot acknowledge a saved draft. Reap it without exposing OS diagnostics.
    let _ = child.kill();
    let _ = child.wait();
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.watch.abort();
        if let Ok(mut process) = self.process.lock() {
            terminate(&mut process.child);
        }
    }
}
