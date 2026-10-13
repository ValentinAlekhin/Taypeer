//! Actual PUBLIC kernel pipes exercise ownership and refusals without OS or key mocks.
use super::*;
use filedescriptor::Pipe;
use std::{
    io::{Read, Seek, Write},
    os::unix::net::UnixStream,
};

fn pipe_with(bytes: &[u8]) -> FileDescriptor {
    let Pipe { read, mut write } = Pipe::new().unwrap();
    write.write_all(bytes).unwrap();
    drop(write);
    read
}

#[test]
fn owned_public_pipe_survives_original_close_and_preserves_exact_eof() {
    let Pipe { read, mut write } = Pipe::new().unwrap();
    let imported = ImportedKeyInput::duplicate(read.as_raw_fd()).unwrap();
    assert!(
        rustix::io::fcntl_getfd(&imported.0)
            .unwrap()
            .contains(rustix::io::FdFlags::CLOEXEC)
    );
    drop(read);
    write.write_all(&PUBLIC_PATTERN).unwrap();
    drop(write);
    assert!(imported.verify_public().unwrap());
}

#[test]
fn public_probe_accepts_only_its_fixed_pattern_without_returning_a_key() {
    for (pattern, expected) in [(PUBLIC_PATTERN, true), ([0x51; 32], false)] {
        let read = pipe_with(&pattern);
        assert_eq!(verify_public(read.as_raw_fd()).unwrap(), expected);
        // Probe dropped only its owned duplicate; the held original still observes EOF.
        let mut read = read;
        assert_eq!(read.read(&mut [0u8; 1]).unwrap(), 0);
    }
}

#[test]
fn public_pipe_refuses_truncated_and_extra_bytes_before_ack() {
    for count in [0, 1, 31, 33, 256] {
        let read = pipe_with(&vec![0x50; count]);
        assert!(matches!(
            verify_public(read.as_raw_fd()),
            Err(AndroidError::InvalidFile)
        ));
    }
}

#[test]
fn private_import_refuses_file_socket_and_writer_without_consuming_originals() {
    let mut file = tempfile::tempfile().unwrap();
    file.write_all(&PUBLIC_PATTERN).unwrap();
    assert!(matches!(
        ImportedKeyInput::duplicate(file.as_raw_fd()),
        Err(AndroidError::InvalidFile)
    ));
    assert_eq!(file.stream_position().unwrap(), 32);

    let (mut socket, mut peer) = UnixStream::pair().unwrap();
    peer.write_all(&PUBLIC_PATTERN).unwrap();
    assert!(matches!(
        ImportedKeyInput::duplicate(socket.as_raw_fd()),
        Err(AndroidError::InvalidFile)
    ));
    let mut received = [0; 32];
    socket.read_exact(&mut received).unwrap();
    assert_eq!(received, PUBLIC_PATTERN);

    let Pipe { read, mut write } = Pipe::new().unwrap();
    assert!(matches!(
        ImportedKeyInput::duplicate(write.as_raw_fd()),
        Err(AndroidError::InvalidFile)
    ));
    write.write_all(&PUBLIC_PATTERN).unwrap();
    drop(write);
    assert!(verify_public(read.as_raw_fd()).unwrap());
}

#[test]
fn private_import_refuses_nonblocking_and_invalid_numbers_without_closing_owner() {
    let read = pipe_with(&PUBLIC_PATTERN);
    let mut read = read;
    read.set_non_blocking(true).unwrap();
    assert!(matches!(
        ImportedKeyInput::duplicate(read.as_raw_fd()),
        Err(AndroidError::InvalidFile)
    ));
    read.set_non_blocking(false).unwrap();
    assert!(verify_public(read.as_raw_fd()).unwrap());
    for fd in [-1, i32::MIN, i32::MAX] {
        assert!(ImportedKeyInput::duplicate(fd).is_err());
    }
}
