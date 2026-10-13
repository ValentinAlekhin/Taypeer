//! Private key-FD decoding belongs to the isolated native worker entry.
use crate::RuntimeError;
use std::io::{ErrorKind, Read};
use taypeer_storage::ReadKey;
use zeroize::Zeroizing;

/// Consume a private OS crypto adapter's one-shot key input inside the isolated worker.
/// The stream must contain exactly 32 bytes followed by EOF. Short/extra data is a
/// protocol refusal; other read failures remain transport errors. The fixed scratch
/// is zeroized on every return, and only the worker-owned `ReadKey` survives success.
///
/// This is a native worker-entry helper, not a host/UI getter. Never route this input
/// through `WorkerStreams`, selected-file capabilities, JSON or `CredentialPort`.
/// Its FD owner must close/cancel the input independently of a blocked OS operation.
pub fn read_private_key_input(mut input: impl Read) -> Result<ReadKey, RuntimeError> {
    let mut bytes = Zeroizing::new([0u8; 32]);
    input
        .read_exact(&mut *bytes)
        .map_err(|error| match error.kind() {
            ErrorKind::UnexpectedEof => RuntimeError::Protocol,
            _ => RuntimeError::Transport,
        })?;
    let mut trailing = Zeroizing::new([0u8; 1]);
    loop {
        match input.read(&mut *trailing) {
            Ok(0) => return Ok(ReadKey::from_secret(&bytes)),
            Ok(_) => return Err(RuntimeError::Protocol),
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(_) => return Err(RuntimeError::Transport),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn private_key_input_accepts_exact_public_fixture_and_eof() {
        let key = read_private_key_input(Cursor::new([42u8; 32])).unwrap();
        assert_eq!(key.secret_bytes(), &[42u8; 32]);
    }

    #[test]
    fn private_key_input_rejects_empty_short_extra_and_unbounded_input() {
        for length in [0, 1, 31, 33, 256] {
            assert!(matches!(
                read_private_key_input(Cursor::new(vec![42u8; length])),
                Err(RuntimeError::Protocol)
            ));
        }
        assert!(matches!(
            read_private_key_input(std::io::repeat(42)),
            Err(RuntimeError::Protocol)
        ));
    }

    struct InterruptedInput {
        bytes: Cursor<[u8; 32]>,
        interrupted: bool,
    }
    impl Read for InterruptedInput {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            if !self.interrupted && self.bytes.position() == 32 {
                self.interrupted = true;
                return Err(ErrorKind::Interrupted.into());
            }
            self.bytes.read(buffer)
        }
    }

    #[test]
    fn private_key_input_retries_interrupted_eof_check_without_reusing_scratch() {
        let input = InterruptedInput {
            bytes: Cursor::new([42u8; 32]),
            interrupted: false,
        };
        assert!(read_private_key_input(input).is_ok());
    }

    struct FailedInput {
        remaining: usize,
    }
    impl Read for FailedInput {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            if self.remaining == 0 {
                return Err(ErrorKind::PermissionDenied.into());
            }
            let length = buffer.len().min(self.remaining);
            buffer[..length].fill(42);
            self.remaining -= length;
            Ok(length)
        }
    }

    #[test]
    fn private_key_input_preserves_transport_failure_before_and_after_32_bytes() {
        for remaining in [0, 12, 32] {
            assert!(matches!(
                read_private_key_input(FailedInput { remaining }),
                Err(RuntimeError::Transport)
            ));
        }
    }
}
