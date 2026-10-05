//! Synthetic selected streams verify bounded transfers and cleanup without Android callbacks.
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

struct InputCleanup {
    host: Arc<SelectedTransfersHost>,
    closed: Arc<AtomicUsize>,
}
impl SelectedInput for InputCleanup {
    fn length(&self) -> Result<u64, AndroidError> {
        Ok(1)
    }
    fn read(&self, _: u64, _: u32) -> Result<Vec<u8>, AndroidError> {
        panic!("Unbound capability must not read")
    }
    fn close(&self) {
        assert!(
            self.host.entries.try_lock().is_ok(),
            "Close ran with the registry locked"
        );
        self.closed.fetch_add(1, Ordering::Relaxed);
    }
}
#[test]
fn unbound_capability_cannot_read_and_cleanup_runs_outside_the_registry_lock() {
    let host = Arc::new(SelectedTransfersHost::default());
    let closed = Arc::new(AtomicUsize::new(0));
    host.entries.lock().unwrap().insert(
        1,
        Arc::new(Selection::Input {
            stream: Arc::new(InputCleanup {
                host: Arc::clone(&host),
                closed: Arc::clone(&closed),
            }),
            length: 1,
        }),
    );
    assert!(matches!(host.read(1, 0, 1), Err(AndroidError::Runtime)));
    drop(SelectionLease {
        host: Arc::clone(&host),
        id: 1,
    });
    assert_eq!(closed.load(Ordering::Relaxed), 1);
    assert!(host.entries.lock().unwrap().is_empty());
}

#[derive(Default)]
struct Remote {
    calls: Mutex<Vec<(u64, usize)>>,
    oversized: bool,
    uncertain: bool,
}
impl SelectedTransfersRemote for Remote {
    fn read(&self, _: u64, offset: u64, count: u32) -> Result<Vec<u8>, AndroidError> {
        assert!(count as usize <= crate::descriptors::TRANSFER_CHUNK);
        self.calls.lock().unwrap().push((offset, count as usize));
        Ok(vec![7; count as usize + usize::from(self.oversized)])
    }
    fn write(&self, _: u64, offset: u64, bytes: Vec<u8>) -> Result<u32, AndroidError> {
        assert!(bytes.len() <= crate::descriptors::TRANSFER_CHUNK);
        self.calls.lock().unwrap().push((offset, bytes.len()));
        Ok(bytes.len() as u32)
    }
    fn finish(&self, _: u64) -> Result<(), AndroidError> {
        if self.uncertain {
            Err(AndroidError::CommitUncertain)
        } else {
            Ok(())
        }
    }
}
#[test]
fn selected_input_is_bounded_and_keeps_independent_offsets() {
    let remote = Arc::new(Remote::default());
    let port: Arc<dyn SelectedTransfersRemote> = remote.clone();
    let mut first = RemoteInput {
        port: Arc::clone(&port),
        id: 1,
        offset: 0,
    };
    let mut second = RemoteInput {
        port,
        id: 2,
        offset: 0,
    };
    let mut bytes = vec![0; crate::descriptors::TRANSFER_CHUNK * 2];
    let chunk = first.read(&mut bytes).unwrap();
    assert_eq!(chunk, crate::descriptors::TRANSFER_CHUNK);
    assert_eq!(first.read(&mut bytes[..3]).unwrap(), 3);
    assert_eq!(second.read(&mut bytes[..2]).unwrap(), 2);
    assert_eq!(
        *remote.calls.lock().unwrap(),
        vec![(0, chunk), (chunk as u64, 3), (0, 2)]
    );
}
#[test]
fn oversized_provider_reply_is_rejected_without_advancing_the_input() {
    let mut source = RemoteInput {
        port: Arc::new(Remote {
            oversized: true,
            ..Default::default()
        }),
        id: 1,
        offset: 0,
    };
    let mut bytes = [0; 2];
    assert_eq!(
        source.read(&mut bytes).unwrap_err().kind(),
        std::io::ErrorKind::InvalidData
    );
    assert_eq!(source.offset, 0);
    assert_eq!(bytes, [0; 2]);
}
#[test]
fn selected_output_is_bounded_and_uncertain_finish_is_never_success() {
    use taypeer_runtime::platform_worker::SelectedOutput;
    let remote = Arc::new(Remote {
        uncertain: true,
        ..Default::default()
    });
    let mut output = RemoteOutput {
        port: remote.clone(),
        id: 1,
        offset: 0,
    };
    assert_eq!(
        output
            .write(&vec![7; crate::descriptors::TRANSFER_CHUNK * 2])
            .unwrap(),
        crate::descriptors::TRANSFER_CHUNK
    );
    output.write_all(&[7, 7]).unwrap();
    assert_eq!(
        *remote.calls.lock().unwrap(),
        vec![
            (0, crate::descriptors::TRANSFER_CHUNK),
            (crate::descriptors::TRANSFER_CHUNK as u64, 2)
        ]
    );
    assert!(matches!(
        output.finish(),
        Err(taypeer_runtime::RuntimeError::Service(
            taypeer_services::ServiceError::Storage(taypeer_storage::Error::CommitUncertain)
        ))
    ));
}
