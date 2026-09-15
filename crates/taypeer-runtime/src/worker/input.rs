//! Read independently of service work so parent EOF can end even a stuck plaintext command.
#[cfg(test)]
mod tests;
use crate::{RuntimeError, protocol::MAX_MESSAGE, session::CLOSE_GRACE};
use std::{
    io::{self, Cursor, Read},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};
use zeroize::Zeroizing;

pub(super) struct Lifetime(Arc<AtomicBool>);
impl Drop for Lifetime {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}
pub(super) struct Incoming {
    receive: mpsc::Receiver<Result<Zeroizing<Vec<u8>>, RuntimeError>>,
    current: Cursor<Zeroizing<Vec<u8>>>,
}
impl Incoming {
    pub fn start(mut reader: impl Read + Send + 'static) -> (Self, Lifetime) {
        let (send, receive) = mpsc::sync_channel(2);
        let finished = Arc::new(AtomicBool::new(false));
        let ended = Arc::clone(&finished);
        std::thread::spawn(move || {
            loop {
                let frame = frame(&mut reader);
                let failed = frame.is_err();
                // The private protocol permits one outstanding command and one I/O reply.
                // A full queue is a protocol failure, not permission to retain unlimited input.
                if send.try_send(frame).is_err() || failed {
                    break;
                }
            }
            std::thread::sleep(CLOSE_GRACE);
            if !ended.load(Ordering::Acquire) {
                // This entry point runs only in a dedicated plaintext child. Destructors
                // cannot clean a stuck Automerge graph; process exit releases its address space.
                std::process::exit(70);
            }
        });
        (
            Self {
                receive,
                current: Cursor::new(Zeroizing::new(Vec::new())),
            },
            Lifetime(finished),
        )
    }
}
impl Read for Incoming {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        if self.current.position() as usize == self.current.get_ref().len() {
            self.current = Cursor::new(
                self.receive
                    .recv()
                    .ok()
                    .and_then(Result::ok)
                    .ok_or_else(|| io::Error::from(io::ErrorKind::UnexpectedEof))?,
            );
        }
        self.current.read(buffer)
    }
}
fn frame(reader: &mut impl Read) -> Result<Zeroizing<Vec<u8>>, RuntimeError> {
    let mut header = [0; 4];
    reader
        .read_exact(&mut header)
        .map_err(|_| RuntimeError::Transport)?;
    let length = u32::from_le_bytes(header) as usize;
    if length > MAX_MESSAGE {
        return Err(RuntimeError::TooLarge);
    }
    let mut bytes = Zeroizing::new(vec![0; length + 4]);
    bytes[..4].copy_from_slice(&header);
    reader
        .read_exact(&mut bytes[4..])
        .map_err(|_| RuntimeError::Transport)?;
    // Syntax validation does not retain another decoded copy of secret strings.
    serde_json::from_slice::<serde::de::IgnoredAny>(&bytes[4..])
        .map_err(|_| RuntimeError::Protocol)?;
    Ok(bytes)
}
