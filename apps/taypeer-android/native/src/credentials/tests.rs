//! PUBLIC protected metadata preserves precise platform failure categories across FFI.
use super::*;
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Default)]
struct Port {
    value: Arc<Mutex<Option<Vec<u8>>>>,
    fault: Arc<AtomicU8>,
}
impl CredentialPort for Port {
    fn get(&self, _: String, _: String) -> Result<Option<Vec<u8>>, AndroidError> {
        match self.fault.load(Ordering::Relaxed) {
            3 => Err(AndroidError::ProfileInvalid),
            4 => Err(AndroidError::Credentials),
            _ => Ok(self.value.lock().unwrap().clone()),
        }
    }
    fn set(&self, _: String, _: String, bytes: Vec<u8>) -> Result<(), AndroidError> {
        if self.fault.load(Ordering::Relaxed) == 1 {
            return Err(AndroidError::Credentials);
        }
        *self.value.lock().unwrap() = Some(bytes);
        if self.fault.load(Ordering::Relaxed) == 2 {
            return Err(AndroidError::CommitUncertain);
        }
        Ok(())
    }
}
#[test]
fn metadata_callback_unknown_publication_is_not_flattened_into_credentials() {
    let port = Port::default();
    let credentials = Credentials(Box::new(port.clone()));
    assert!(
        credentials
            .get("PUBLIC service", "PUBLIC entry")
            .unwrap()
            .is_none()
    );
    port.fault.store(1, Ordering::Relaxed);
    assert_eq!(
        credentials
            .set("PUBLIC service", "PUBLIC entry", b"PUBLIC metadata")
            .unwrap_err(),
        ProfileError::Credentials
    );
    assert!(port.value.lock().unwrap().is_none());
    port.fault.store(2, Ordering::Relaxed);
    let error = credentials
        .set("PUBLIC service", "PUBLIC entry", b"PUBLIC metadata")
        .unwrap_err();
    assert_eq!(error, ProfileError::CommitUncertain);
    assert!(matches!(
        host_error(error.into()),
        AndroidError::CommitUncertain
    ));
    assert_eq!(
        port.value.lock().unwrap().as_deref(),
        Some(b"PUBLIC metadata".as_slice())
    );
    port.fault.store(0, Ordering::Relaxed);
    credentials
        .set("PUBLIC service", "PUBLIC entry", b"PUBLIC metadata")
        .unwrap();
    port.fault.store(3, Ordering::Relaxed);
    let error = credentials
        .get("PUBLIC service", "PUBLIC entry")
        .unwrap_err();
    assert_eq!(error, ProfileError::Invalid);
    assert!(matches!(
        host_error(error.into()),
        AndroidError::ProfileInvalid
    ));
    port.fault.store(4, Ordering::Relaxed);
    assert_eq!(
        credentials
            .get("PUBLIC service", "PUBLIC entry")
            .unwrap_err(),
        ProfileError::Credentials
    );
}
