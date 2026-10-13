//! PUBLIC kernel descriptors prove exact ownership and terminal lifecycle fences.
use super::*;
use filedescriptor::{FileDescriptor, Pipe};
use std::{
    io::{Seek, Write},
    os::{fd::AsRawFd, unix::net::UnixStream},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};
use taypeer_runtime::platform_worker::{biometry::BiometricInputPurpose, read_private_key_input};
use taypeer_services::ManagedKeyScope;

const PUBLIC: [u8; 32] = [0x50; 32];
const GATE_TIMEOUT: Duration = Duration::from_secs(5);
fn digest(value: u8) -> Digest {
    Digest::from_bytes([value; 32])
}
fn lease(attempt: u8) -> BiometricInputLease {
    BiometricInputLease {
        scope: BiometricAttemptScope {
            namespace: digest(1),
            generation: 1,
            attempt: digest(attempt),
            managed: ManagedKeyScope {
                database: DatabaseId::new("PUBLIC input database"),
                working_copy: digest(2),
                root: digest(3),
                epoch: 1,
                control: digest(4),
                snapshot: digest(5),
                checkpoint: digest(6),
            },
        },
        input: digest(7),
        purpose: BiometricInputPurpose::EnrollmentProof,
    }
}
fn public_pipe(bytes: &[u8]) -> FileDescriptor {
    let Pipe { read, mut write } = Pipe::new().unwrap();
    write.write_all(bytes).unwrap();
    drop(write);
    read
}
fn require_public(reader: KeyReader) {
    let key = read_private_key_input(reader).unwrap();
    assert!(key.secret_bytes() == &PUBLIC);
}
fn protocol<T>(result: Result<T, RuntimeError>) {
    assert!(matches!(result, Err(RuntimeError::Protocol)));
}
fn closed<T>(result: Result<T, RuntimeError>) {
    assert!(matches!(result, Err(RuntimeError::Closed)));
}

#[test]
fn install_owns_duplicate_once_original_close_preserves_exact_private_eof() {
    let owner = PrivateBiometricInputs::new();
    let lease = lease(10);
    let original = public_pipe(&PUBLIC);
    assert!(matches!(
        owner.install(&lease, original.as_raw_fd()),
        Err(AndroidError::InvalidOptions)
    ));
    owner.arm(&lease).unwrap();
    owner.check(&lease).unwrap();
    owner.install(&lease, original.as_raw_fd()).unwrap();
    rustix::fs::fstat(&original).unwrap();
    assert!(matches!(
        owner.install(&lease, original.as_raw_fd()),
        Err(AndroidError::InvalidOptions)
    ));
    drop(original);
    let reader = owner.consume(&lease).unwrap();
    protocol(owner.consume(&lease));
    protocol(owner.arm(&lease));
    owner.check(&lease).unwrap();
    require_public(reader);
    owner.retire(&lease);
    closed(owner.check(&lease));
    closed(owner.consume(&lease));
    protocol(owner.arm(&lease));
}

#[test]
fn cancellation_after_consume_fences_proof_even_while_worker_owns_reader() {
    let owner = PrivateBiometricInputs::new();
    let lease = lease(10);
    let original = public_pipe(&PUBLIC);
    owner.arm(&lease).unwrap();
    owner.install(&lease, original.as_raw_fd()).unwrap();
    let reader = owner.consume(&lease).unwrap();
    owner.retire(&lease);
    owner.retire(&lease);
    closed(owner.check(&lease));
    protocol(owner.arm(&lease));
    // Already-consumed ownership remains solely in Rust, but it conveys no
    // eligibility for authentication/publication after the independent fence.
    drop(original);
    require_public(reader);
    closed(owner.check(&lease));
}

#[test]
fn exact_lease_guards_precede_dup_and_foreign_cancel_keeps_current_input() {
    let owner = PrivateBiometricInputs::new();
    let expected = lease(10);
    owner.arm(&expected).unwrap();
    let mut mismatches = Vec::new();
    let mut other = expected.clone();
    other.input = digest(8);
    mismatches.push(other);
    let mut other = expected.clone();
    other.purpose = BiometricInputPurpose::KeyedBoot;
    mismatches.push(other);
    let mut other = expected.clone();
    other.scope.generation = 2;
    mismatches.push(other);
    let mut other = expected.clone();
    other.scope.attempt = digest(11);
    mismatches.push(other);
    let mut other = expected.clone();
    other.scope.namespace = digest(8);
    mismatches.push(other);
    let mut other = expected.clone();
    other.scope.managed.database = DatabaseId::new("PUBLIC other");
    mismatches.push(other);
    let mut other = expected.clone();
    other.scope.managed.working_copy = digest(8);
    mismatches.push(other);
    let mut other = expected.clone();
    other.scope.managed.root = digest(8);
    mismatches.push(other);
    let mut other = expected.clone();
    other.scope.managed.epoch = 2;
    mismatches.push(other);
    let mut other = expected.clone();
    other.scope.managed.control = digest(8);
    mismatches.push(other);
    let mut other = expected.clone();
    other.scope.managed.snapshot = digest(8);
    mismatches.push(other);
    let mut other = expected.clone();
    other.scope.managed.checkpoint = digest(8);
    mismatches.push(other);
    let imports = AtomicUsize::new(0);
    for other in mismatches {
        assert!(matches!(
            owner.install_with(&other, -1, |_| {
                imports.fetch_add(1, Ordering::SeqCst);
                key_fd::import_private(-1)
            }),
            Err(AndroidError::InvalidOptions)
        ));
        protocol(owner.check(&other));
        protocol(owner.consume(&other));
        owner.retire(&other);
        owner.check(&expected).unwrap();
    }
    assert_eq!(imports.load(Ordering::SeqCst), 0);
    let original = public_pipe(&PUBLIC);
    owner.install(&expected, original.as_raw_fd()).unwrap();
    require_public(owner.consume(&expected).unwrap());
}

#[test]
fn terminal_before_arrival_and_all_sixty_four_attempts_are_never_evicted() {
    let owner = PrivateBiometricInputs::new();
    let cancelled = lease(10);
    owner.retire(&cancelled);
    protocol(owner.arm(&cancelled));
    for attempt in 11..=73 {
        let lease = lease(attempt);
        owner.arm(&lease).unwrap();
        owner.retire(&lease);
    }
    assert_eq!(owner.state.lock().unwrap().attempts.len(), 64);
    for attempt in 10..=73 {
        protocol(owner.arm(&lease(attempt)));
    }
    protocol(owner.arm(&lease(74)));
    assert_eq!(owner.state.lock().unwrap().attempts.len(), 64);
    // Replacement, not tombstone eviction, permits an explicit fresh worker.
    let fresh = PrivateBiometricInputs::new();
    let mut new_worker = lease(74);
    new_worker.scope.generation = 2;
    fresh.arm(&new_worker).unwrap();
}

#[test]
fn first_valid_arm_pins_process_and_copy_but_current_snapshot_is_per_attempt() {
    let owner = PrivateBiometricInputs::new();
    let mut invalid = lease(10);
    invalid.scope.generation = 0;
    protocol(owner.arm(&invalid));
    invalid.scope.generation = 1;
    invalid.scope.managed.database = DatabaseId::new("");
    protocol(owner.arm(&invalid));
    let first = lease(10);
    owner.arm(&first).unwrap();
    owner.retire(&first);
    let mut changed = lease(11);
    changed.scope.managed.snapshot = digest(9);
    changed.scope.managed.control = digest(9);
    changed.scope.managed.checkpoint = digest(9);
    owner.arm(&changed).unwrap();
    owner.retire(&changed);
    let mut other_generation = lease(12);
    other_generation.scope.generation = 2;
    protocol(owner.arm(&other_generation));
    let mut other_namespace = lease(12);
    other_namespace.scope.namespace = digest(9);
    protocol(owner.arm(&other_namespace));
    let mut other_copy = lease(12);
    other_copy.scope.managed.working_copy = digest(9);
    protocol(owner.arm(&other_copy));
    let mut other_database = lease(12);
    other_database.scope.managed.database = DatabaseId::new("PUBLIC other");
    protocol(owner.arm(&other_database));
    let mut other_root = lease(12);
    other_root.scope.managed.root = digest(9);
    protocol(owner.arm(&other_root));
    assert_eq!(owner.state.lock().unwrap().attempts.len(), 2);
}

struct CountedReader {
    reader: KeyReader,
    drops: Arc<AtomicUsize>,
}
impl Read for CountedReader {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        self.reader.read(bytes)
    }
}
impl Drop for CountedReader {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn cancellation_between_dup_and_install_closes_duplicate_and_preserves_original() {
    let owner = Arc::new(PrivateBiometricInputs::new());
    let lease = lease(10);
    owner.arm(&lease).unwrap();
    let mut original = public_pipe(&PUBLIC);
    let held_fd = original.as_raw_fd();
    let (ready, imported) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let drops = Arc::new(AtomicUsize::new(0));
    let thread = {
        let owner = Arc::clone(&owner);
        let lease = lease.clone();
        let drops = Arc::clone(&drops);
        thread::spawn(move || {
            owner.install_with(&lease, held_fd, |fd| {
                let reader = key_fd::import_private(fd)?;
                ready.send(()).unwrap();
                resume.recv_timeout(GATE_TIMEOUT).unwrap();
                Ok(Box::new(CountedReader { reader, drops }))
            })
        })
    };
    imported.recv_timeout(GATE_TIMEOUT).unwrap();
    owner.retire(&lease);
    release.send(()).unwrap();
    assert!(matches!(thread.join().unwrap(), Err(AndroidError::Runtime)));
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    rustix::fs::fstat(&original).unwrap();
    let mut actual = [0; 32];
    original.read_exact(&mut actual).unwrap();
    assert_eq!(actual, PUBLIC);
    closed(owner.consume(&lease));
    protocol(owner.arm(&lease));
}

#[test]
fn concurrent_install_accepts_one_owned_reader_and_drops_the_late_duplicate() {
    let owner = Arc::new(PrivateBiometricInputs::new());
    let lease = lease(10);
    owner.arm(&lease).unwrap();
    let late = public_pipe(&[0x51; 32]);
    let winner = public_pipe(&PUBLIC);
    let held_fd = late.as_raw_fd();
    let (ready, imported) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let drops = Arc::new(AtomicUsize::new(0));
    let thread = {
        let owner = Arc::clone(&owner);
        let lease = lease.clone();
        let drops = Arc::clone(&drops);
        thread::spawn(move || {
            owner.install_with(&lease, held_fd, |fd| {
                let reader = key_fd::import_private(fd)?;
                ready.send(()).unwrap();
                resume.recv_timeout(GATE_TIMEOUT).unwrap();
                Ok(Box::new(CountedReader { reader, drops }))
            })
        })
    };
    imported.recv_timeout(GATE_TIMEOUT).unwrap();
    owner.install(&lease, winner.as_raw_fd()).unwrap();
    release.send(()).unwrap();
    assert!(matches!(
        thread.join().unwrap(),
        Err(AndroidError::InvalidOptions)
    ));
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    rustix::fs::fstat(&late).unwrap();
    rustix::fs::fstat(&winner).unwrap();
    drop(winner);
    require_public(owner.consume(&lease).unwrap());
}

#[test]
fn rejected_kernel_transports_do_not_consume_or_close_originals_or_fill_slot() {
    let owner = PrivateBiometricInputs::new();
    let lease = lease(10);
    owner.arm(&lease).unwrap();
    let mut file = tempfile::tempfile().unwrap();
    file.write_all(&PUBLIC).unwrap();
    assert!(matches!(
        owner.install(&lease, file.as_raw_fd()),
        Err(AndroidError::InvalidFile)
    ));
    assert_eq!(file.stream_position().unwrap(), 32);
    rustix::fs::fstat(&file).unwrap();
    let (mut socket, mut peer) = UnixStream::pair().unwrap();
    peer.write_all(&PUBLIC).unwrap();
    assert!(matches!(
        owner.install(&lease, socket.as_raw_fd()),
        Err(AndroidError::InvalidFile)
    ));
    let mut received = [0; 32];
    socket.read_exact(&mut received).unwrap();
    assert_eq!(received, PUBLIC);
    let Pipe {
        mut read,
        mut write,
    } = Pipe::new().unwrap();
    assert!(matches!(
        owner.install(&lease, write.as_raw_fd()),
        Err(AndroidError::InvalidFile)
    ));
    read.set_non_blocking(true).unwrap();
    assert!(matches!(
        owner.install(&lease, read.as_raw_fd()),
        Err(AndroidError::InvalidFile)
    ));
    rustix::fs::fstat(&write).unwrap();
    rustix::fs::fstat(&read).unwrap();
    read.set_non_blocking(false).unwrap();
    assert!(matches!(
        owner.install(&lease, -1),
        Err(AndroidError::InvalidFile)
    ));
    owner.install(&lease, read.as_raw_fd()).unwrap();
    write.write_all(&PUBLIC).unwrap();
    drop(write);
    drop(read);
    require_public(owner.consume(&lease).unwrap());
}

#[test]
fn accepted_fd_is_not_a_key_proof_and_private_reader_refuses_short_or_extra_input() {
    for count in [0, 31, 33] {
        let owner = PrivateBiometricInputs::new();
        let lease = lease(10);
        let original = public_pipe(&vec![0x50; count]);
        owner.arm(&lease).unwrap();
        owner.install(&lease, original.as_raw_fd()).unwrap();
        rustix::fs::fstat(&original).unwrap();
        protocol(read_private_key_input(owner.consume(&lease).unwrap()));
        owner.retire(&lease);
        closed(owner.check(&lease));
        protocol(owner.arm(&lease));
    }
}
