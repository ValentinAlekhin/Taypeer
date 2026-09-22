//! Exclusive ownership of an OS file description, including Android's flock adapter.
use std::fs::{File, TryLockError};

/// Acquire an exclusive advisory lock without waiting for another owner.
/// Keep the file open for the entire protected operation; closing its last handle
/// releases the lock, including on process death. Duplicating a handle does not
/// create an independent owner. Contention is distinct from unsupported/I/O errors.
pub fn try_lock_exclusive(file: &File) -> Result<(), TryLockError> {
    #[cfg(target_os = "android")]
    {
        use rustix::fs::{FlockOperation, flock};
        flock(file, FlockOperation::NonBlockingLockExclusive).map_err(|error| {
            if error == rustix::io::Errno::WOULDBLOCK {
                TryLockError::WouldBlock
            } else {
                TryLockError::Error(error.into())
            }
        })
    }
    #[cfg(not(target_os = "android"))]
    file.try_lock()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn independent_owner_is_rejected_until_the_last_handle_closes() {
        let path = tempfile::NamedTempFile::new().unwrap();
        let first = path.reopen().unwrap();
        let second = path.reopen().unwrap();
        try_lock_exclusive(&first).unwrap();
        assert!(matches!(
            try_lock_exclusive(&second),
            Err(TryLockError::WouldBlock)
        ));
        let duplicate = first.try_clone().unwrap();
        drop(first);
        assert!(matches!(
            try_lock_exclusive(&second),
            Err(TryLockError::WouldBlock)
        ));
        drop(duplicate);
        try_lock_exclusive(&second).unwrap();
    }
}
