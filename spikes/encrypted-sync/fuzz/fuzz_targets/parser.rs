#![no_main]
use libfuzzer_sys::fuzz_target;
use taypeer_encrypted_sync_spike::{container::Container,MAX_BYTES};
fuzz_target!(|data:&[u8]| {
    if data.len()>MAX_BYTES{return;}
    let pinned = serde_json::from_slice::<Container>(data).map(|c| c.chain.root.hash()).unwrap_or([0;32]);
    let _=Container::parse(data,pinned);
    let _=automerge::Change::from_bytes(data.to_vec());
    let _=automerge::Automerge::load(data);
});
