//! Blocking pipe actor. The supervisor owns Child independently of this actor's I/O.
#[cfg(test)]
pub(crate) mod tests;
use crate::{
    Command, RuntimeError,
    cipher_ipc::{IoReply, IoRequest, WorkerMessage},
    host::Callbacks,
    protocol::{Response, read_frame, write_frame},
    session::{LockReason, POLL, ProcessControl},
};
use serde_json::Value;
use std::{
    io::{BufReader, Write},
    process::{ChildStdin, ChildStdout},
    sync::{Arc, mpsc},
};
use zeroize::Zeroizing;

pub(crate) enum Job {
    Request {
        bytes: Zeroizing<Vec<u8>>,
        opening: bool,
        reply: mpsc::SyncSender<Response>,
    },
    Close,
}
pub(crate) struct Client {
    pub control: Arc<ProcessControl>,
    pub jobs: mpsc::SyncSender<Job>,
}
impl Client {
    pub fn spawn(
        executable: &std::path::Path,
        sessions: &crate::session::SessionController,
        callbacks: Box<dyn CallbackHandler>,
        spool: tempfile::TempDir,
    ) -> Result<Arc<Self>, RuntimeError> {
        use std::process::{Command, Stdio};
        let child = Command::new(executable)
            .arg("__worker")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| RuntimeError::Transport)?;
        let (jobs, receive) = mpsc::sync_channel(1);
        // Child ownership is guarded before any fallible setup step.
        let control = Arc::new(ProcessControl::new(
            child,
            jobs.clone(),
            sessions.activity(),
        ));
        control.set_generation(sessions.register(&control)?);
        let (input, output) = control.take_pipes()?;
        let actor = PipeActor {
            input,
            output: BufReader::new(output),
            callbacks,
            spool,
        };
        let running = Arc::clone(&control);
        std::thread::spawn(move || actor.run(receive, running));
        Ok(Arc::new(Self { control, jobs }))
    }
    pub fn request(
        &self,
        data: &impl serde::Serialize,
        opening: bool,
    ) -> Result<Value, RuntimeError> {
        self.control.check(opening)?;
        let bytes = Zeroizing::new(serde_json::to_vec(data).map_err(|_| RuntimeError::Protocol)?);
        if bytes.len() > crate::protocol::MAX_MESSAGE {
            return Err(RuntimeError::TooLarge);
        }
        let (reply, receive) = mpsc::sync_channel(1);
        let mut job = Job::Request {
            bytes,
            opening,
            reply,
        };
        loop {
            self.control.check(opening)?;
            match self.jobs.try_send(job) {
                Ok(()) => break,
                Err(mpsc::TrySendError::Full(pending)) => {
                    job = pending;
                    std::thread::sleep(POLL);
                }
                Err(mpsc::TrySendError::Disconnected(_)) => return Err(RuntimeError::Transport),
            }
        }
        loop {
            self.control.check(opening).map_err(interrupted)?;
            match receive.recv_timeout(POLL) {
                Ok(response) => {
                    self.control.check(opening).map_err(interrupted)?;
                    return response.into_result();
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    self.control.invalidate(LockReason::Transport);
                    return Err(RuntimeError::Transport);
                }
            }
        }
    }
}
pub(crate) struct PipeActor {
    pub input: ChildStdin,
    pub output: BufReader<ChildStdout>,
    pub callbacks: Box<dyn CallbackHandler>,
    pub spool: tempfile::TempDir,
}
pub(crate) trait CallbackHandler: Send {
    fn handle(&mut self, request: IoRequest) -> Result<crate::cipher_ipc::IoValue, RuntimeError>;
}
pub(crate) struct EnrollmentCallbacks;
impl CallbackHandler for EnrollmentCallbacks {
    fn handle(&mut self, _: IoRequest) -> Result<crate::cipher_ipc::IoValue, RuntimeError> {
        Err(RuntimeError::Protocol)
    }
}
impl CallbackHandler for Callbacks {
    fn handle(&mut self, request: IoRequest) -> Result<crate::cipher_ipc::IoValue, RuntimeError> {
        self.handle(request)
    }
}
impl PipeActor {
    pub fn run(mut self, jobs: mpsc::Receiver<Job>, control: Arc<ProcessControl>) {
        // Retained until all ciphertext callbacks have finished, even after forceful child exit.
        let _spool = &self.spool;
        loop {
            let job = match jobs.recv_timeout(POLL) {
                Ok(job) => job,
                Err(mpsc::RecvTimeoutError::Timeout) if !control.finished() => continue,
                Err(_) => break,
            };
            match job {
                Job::Request {
                    bytes,
                    opening,
                    reply,
                } => {
                    let sent = control.check(opening).and_then(|()| self.send(&bytes));
                    // No plaintext request buffer survives a blocked ciphertext callback.
                    drop(bytes);
                    let result = sent.and_then(|()| self.response(&control, false));
                    if matches!(
                        result,
                        Err(RuntimeError::Transport
                            | RuntimeError::Protocol
                            | RuntimeError::TooLarge)
                    ) {
                        control.invalidate(LockReason::Transport);
                    }
                    // Response also erases a queued result if its caller was cancelled.
                    let _ = reply.send(Response { result });
                }
                Job::Close => {
                    let result = write_frame(&mut self.input, &Command::Lock)
                        .and_then(|()| self.response(&control, true))
                        .map(|mut value| crate::erase_view(&mut value));
                    control.acknowledge(result);
                    break;
                }
            }
        }
        if !control.finished() {
            control.invalidate(LockReason::HostExited);
        }
    }
    fn send(&mut self, bytes: &[u8]) -> Result<(), RuntimeError> {
        self.input
            .write_all(&(bytes.len() as u32).to_le_bytes())
            .map_err(|_| RuntimeError::Transport)?;
        self.input
            .write_all(bytes)
            .map_err(|_| RuntimeError::Transport)?;
        self.input.flush().map_err(|_| RuntimeError::Transport)
    }
    fn response(&mut self, control: &ProcessControl, closing: bool) -> Result<Value, RuntimeError> {
        loop {
            match read_frame::<WorkerMessage>(&mut self.output)? {
                WorkerMessage::Response(response) => return response.into_result(),
                WorkerMessage::Io(request) => {
                    let permission = if closing {
                        matches!(
                            *request,
                            IoRequest::Snapshot { .. }
                                | IoRequest::SaveDraft(_)
                                | IoRequest::DiscardDraft
                        )
                    } else {
                        control.check(true).is_ok()
                    };
                    let result = if permission {
                        self.callbacks.handle(*request)
                    } else {
                        Err(RuntimeError::Closed)
                    };
                    write_frame(&mut self.input, &IoReply { result })?;
                }
            }
        }
    }
}

fn interrupted(error: RuntimeError) -> RuntimeError {
    match error {
        RuntimeError::SessionClosed(reason) => RuntimeError::OperationInterrupted(reason),
        error => error,
    }
}
