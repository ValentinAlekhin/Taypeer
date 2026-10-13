//! PUBLIC kernel descriptors verify the opaque Native target/fence, not OS auth.
use super::*;
use crate::{CipherWriter, SelectedTransfersHost};
use std::{
    fs::File,
    io::{Read, Write},
    os::fd::AsRawFd,
    sync::atomic::AtomicUsize,
};
use taypeer_runtime::{
    RuntimeError,
    platform_worker::biometry::{BiometricAttemptScope, BiometricInputPurpose, BiometricInputs},
};
use taypeer_services::ManagedKeyScope;
use taypeer_trust::Digest;

struct Process {
    inputs: Arc<crate::PrivateBiometricInputs>,
    installs: AtomicUsize,
    revocations: AtomicUsize,
}
impl DocumentProcess for Process {
    fn start(
        &self,
        _: Arc<CipherWriter>,
        _: Arc<SelectedTransfersHost>,
    ) -> Result<(), AndroidError> {
        Err(AndroidError::Runtime)
    }
    fn read(&self, _: u32) -> Result<Vec<u8>, AndroidError> {
        Err(AndroidError::Runtime)
    }
    fn write(&self, _: Vec<u8>) -> Result<u32, AndroidError> {
        Err(AndroidError::Runtime)
    }
    fn exited(&self) -> Result<bool, AndroidError> {
        Ok(true)
    }
    fn terminate(&self) -> Result<(), AndroidError> {
        Ok(())
    }
    fn install_biometric_input(
        &self,
        spec: BiometricInputSpec,
        fd: i32,
    ) -> Result<(), AndroidError> {
        self.installs.fetch_add(1, Ordering::Relaxed);
        self.inputs.install_input(spec, fd)
    }
    fn revoke_biometric_input(&self, spec: BiometricInputSpec) {
        self.revocations.fetch_add(1, Ordering::Relaxed);
        self.inputs.retire_input(spec).unwrap();
    }
}
#[derive(Default)]
struct Port {
    retired: AtomicUsize,
}
impl OsBiometryPort for Port {
    fn availability(&self) -> Result<BiometricAvailability, AndroidError> {
        Ok(BiometricAvailability::NoHardware)
    }
    fn create_key(
        &self,
        _: BiometricBindingView,
        _: String,
    ) -> Result<OsBiometricKeyLease, AndroidError> {
        Err(AndroidError::Runtime)
    }
    fn resolve_key(
        &self,
        _: BiometricBindingView,
        _: String,
        _: String,
        _: String,
        _: String,
    ) -> Result<OsBiometricKeyLease, AndroidError> {
        Err(AndroidError::Runtime)
    }
    fn run_prompt(
        &self,
        _: Arc<BiometricPromptCall>,
    ) -> Result<BiometricPromptOutcome, AndroidError> {
        // Deliberately dishonest success without any FD. It must be refused.
        Ok(BiometricPromptOutcome::Written)
    }
    fn retire_lease(&self, _: String) {
        self.retired.fetch_add(1, Ordering::Relaxed);
    }
    fn delete_key(
        &self,
        _: BiometricBindingView,
        _: String,
        _: String,
    ) -> Result<(), AndroidError> {
        Ok(())
    }
}
fn fixture() -> (
    Arc<Process>,
    Arc<Port>,
    Arc<AttemptFence>,
    Arc<BiometricPromptCall>,
) {
    let lease = BiometricInputLease {
        scope: BiometricAttemptScope {
            namespace: Digest::of(b"PUBLIC profile"),
            managed: ManagedKeyScope {
                database: taypeer_core::DatabaseId::new("PUBLIC DB"),
                working_copy: Digest::of(b"PUBLIC copy"),
                root: Digest::of(b"PUBLIC root"),
                epoch: 1,
                control: Digest::of(b"PUBLIC control"),
                snapshot: Digest::of(b"PUBLIC snapshot"),
                checkpoint: Digest::of(b"PUBLIC checkpoint"),
            },
            generation: 4,
            attempt: Digest::of(b"PUBLIC attempt"),
        },
        input: Digest::of(b"PUBLIC input"),
        purpose: BiometricInputPurpose::EnrollmentProof,
    };
    let inputs = crate::PrivateBiometricInputs::bootstrap();
    inputs.arm(&lease).unwrap();
    let process = Arc::new(Process {
        inputs,
        installs: AtomicUsize::new(0),
        revocations: AtomicUsize::new(0),
    });
    let port = Arc::new(Port::default());
    let fence = AttemptFence::owned(lease, process.clone(), port.clone(), None, None);
    let id = Digest::of(b"PUBLIC OS lease").to_string();
    fence.bind_os(id.clone()).unwrap();
    let call = BiometricPromptCall::new(Arc::clone(&fence), id, [0x50; 256]);
    (process, port, fence, call)
}
fn pipe() -> (File, File) {
    let (reader, writer) = rustix::pipe::pipe().unwrap();
    (File::from(reader), File::from(writer))
}
#[test]
fn opaque_install_owns_duplicate_and_never_consumes_or_closes_the_original() {
    let (process, _, fence, call) = fixture();
    let (original, mut writer) = pipe();
    call.install_input(original.as_raw_fd()).unwrap();
    rustix::fs::fstat(&original).unwrap();
    assert!(matches!(
        call.install_input(original.as_raw_fd()),
        Err(AndroidError::Biometry {
            failure: BiometryFailure::Stale
        })
    ));
    drop(original);
    writer.write_all(&[0x50; 32]).unwrap();
    drop(writer);
    let mut reader = process.inputs.consume(fence.lease()).unwrap();
    let mut public = Vec::new();
    reader.read_to_end(&mut public).unwrap();
    assert_eq!(public, [0x50; 32]);
    call.retire_input();
    assert!(matches!(
        process.inputs.check(fence.lease()),
        Err(RuntimeError::Closed)
    ));
}
#[test]
fn cancelled_call_refuses_fd_before_callback_and_late_os_lease_is_retired() {
    let (process, port, fence, call) = fixture();
    let (original, writer) = pipe();
    call.retire_input();
    assert!(matches!(
        call.install_input(original.as_raw_fd()),
        Err(AndroidError::Biometry {
            failure: BiometryFailure::Cancelled
        })
    ));
    rustix::fs::fstat(&original).unwrap();
    assert_eq!(process.installs.load(Ordering::Relaxed), 0);
    assert_eq!(process.revocations.load(Ordering::Relaxed), 1);
    assert_eq!(port.retired.load(Ordering::Relaxed), 1);
    assert!(
        fence
            .bind_os(Digest::of(b"PUBLIC second forbidden lease").to_string())
            .is_err()
    );
    drop(writer);
}
#[test]
fn dishonest_os_written_without_accepted_input_cannot_complete_the_native_call() {
    let (_, port, _, call) = fixture();
    assert!(matches!(
        call.run(port.as_ref()),
        Err(AndroidError::Biometry {
            failure: BiometryFailure::Stale
        })
    ));
    call.retire_input();
    assert!(matches!(
        call.run(port.as_ref()),
        Err(AndroidError::Biometry {
            failure: BiometryFailure::Cancelled
        })
    ));
}
#[test]
fn typed_projection_roundtrip_refuses_noncanonical_foreign_scope_before_handoff() {
    let (_, _, fence, call) = fixture();
    assert_eq!(call.scope().checked().unwrap(), *fence.lease());
    let mut malformed = call.scope();
    malformed.input = malformed.input.to_uppercase();
    assert!(matches!(
        malformed.checked(),
        Err(AndroidError::InvalidOptions)
    ));
    malformed = call.scope();
    malformed.generation = 0;
    assert!(matches!(
        malformed.checked(),
        Err(AndroidError::InvalidOptions)
    ));
    malformed = call.scope();
    malformed.database = "😀".repeat(33);
    assert!(matches!(
        malformed.checked(),
        Err(AndroidError::InvalidOptions)
    ));
}
