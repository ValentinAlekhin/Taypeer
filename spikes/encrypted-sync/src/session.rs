//! Experimental process boundary for secret state. Locked parent holds only a Child handle.
use crate::Result;
use std::{
    io::{BufRead, BufReader, Write},
    path::Path,
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
};
pub struct SecretSession {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
}
impl SecretSession {
    pub fn open(
        worker: &Path,
        database: &Path,
        root_file: &Path,
        draft_file: &Path,
        password: &[u8],
        fail_draft: bool,
    ) -> Result<Self> {
        if password.len() > 65536 {
            return Err("password size");
        }
        let mut child = Command::new(worker)
            .args([database, root_file, draft_file])
            .arg(if fail_draft { "fail" } else { "save" })
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| "worker spawn")?;
        let input = child.stdin.take().ok_or("worker input")?;
        let output = BufReader::new(child.stdout.take().ok_or("worker output")?);
        let mut session = Self {
            child,
            input,
            output,
        };
        session
            .input
            .write_all(&(password.len() as u32).to_le_bytes())
            .and_then(|_| session.input.write_all(password))
            .map_err(|_| "worker password")?;
        let mut ready = String::new();
        session
            .output
            .read_line(&mut ready)
            .map_err(|_| "worker ready")?;
        if ready != "unlocked\n" {
            return Err("worker unlock");
        }
        Ok(session)
    }
    pub fn pid(&self) -> u32 {
        self.child.id()
    }
    /// Always reaps the worker before returning; failure to save a draft does not cancel lock.
    pub fn lock(mut self, draft: &[u8]) -> Result<bool> {
        if draft.len() > crate::MAX_BYTES / 8 {
            return Err("draft size");
        }
        self.input
            .write_all(&(draft.len() as u32).to_le_bytes())
            .and_then(|_| self.input.write_all(draft))
            .map_err(|_| "worker draft")?;
        let mut status = String::new();
        self.output
            .read_line(&mut status)
            .map_err(|_| "worker lock")?;
        let exited = self.child.wait().map_err(|_| "worker wait")?;
        if !exited.success() || !matches!(status.as_str(), "saved\n" | "draft-failed\n") {
            return Err("worker stopped without confirmation");
        }
        Ok(status == "saved\n")
    }
}
impl Drop for SecretSession {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
