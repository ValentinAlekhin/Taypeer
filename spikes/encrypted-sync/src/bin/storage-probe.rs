//! Child process used only with synthetic fixtures by the crash/restart tests.
use taypeer_encrypted_sync_spike::{Fault, Id, container::Store};
fn main() {
    let args: Vec<_> = std::env::args().collect();
    let root: Id = serde_json::from_slice(&std::fs::read(&args[2]).unwrap()).unwrap();
    let store = Store::at(args[1].clone().into(), root);
    let fault = match args[3].as_str() {
        "before" => Fault::CrashBeforeRename,
        "after" => Fault::CrashAfterRename,
        _ => Fault::None,
    };
    if store
        .apply(
            b"PUBLIC synthetic spike password",
            &ed25519_dalek::SigningKey::from_bytes(&[1; 32]),
            fault,
        )
        .is_err()
    {
        std::process::exit(1);
    }
}
