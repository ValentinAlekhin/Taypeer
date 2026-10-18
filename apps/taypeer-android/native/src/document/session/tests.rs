use super::*;
use std::collections::VecDeque;

struct Chunks {
    values: Mutex<VecDeque<Vec<u8>>>,
    requests: Mutex<Vec<u32>>,
}
impl WorkerStreams for Chunks {
    fn read(&self, count: u32) -> Result<Vec<u8>, AndroidError> {
        self.requests.lock().unwrap().push(count);
        self.values
            .lock()
            .unwrap()
            .pop_front()
            .ok_or(AndroidError::Runtime)
    }
    fn write(&self, _: Vec<u8>) -> Result<u32, AndroidError> {
        unreachable!("read-only PUBLIC stream fixture")
    }
}

#[test]
fn private_stream_reads_preserve_exact_chunks_and_reject_oversized_callbacks() {
    let chunks = Arc::new(Chunks {
        values: Mutex::new(VecDeque::from([
            b"PUBLIC".to_vec(),
            vec![b'X'; crate::descriptors::TRANSFER_CHUNK + 1],
            Vec::new(),
        ])),
        requests: Mutex::new(Vec::new()),
    });
    let mut reader = Reader(chunks.clone());
    let mut bytes = [b'_'; 8];
    assert_eq!(reader.read(&mut bytes).unwrap(), 6);
    assert_eq!(&bytes, b"PUBLIC__");
    assert_eq!(reader.read(&mut []).unwrap(), 0);
    let mut large = vec![b'_'; crate::descriptors::TRANSFER_CHUNK + 8];
    assert_eq!(
        reader.read(&mut large).unwrap_err().kind(),
        std::io::ErrorKind::InvalidData
    );
    assert!(large.iter().all(|byte| *byte == b'_'));
    assert_eq!(reader.read(&mut bytes).unwrap(), 0);
    assert_eq!(
        reader.read(&mut bytes).unwrap_err().kind(),
        std::io::ErrorKind::Other
    );
    assert_eq!(
        *chunks.requests.lock().unwrap(),
        vec![8, crate::descriptors::TRANSFER_CHUNK as u32, 8, 8]
    );
}
