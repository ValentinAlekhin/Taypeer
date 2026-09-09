//! The only stdout data is lifecycle status, never document/key/draft content.
use taypeer_encrypted_sync_spike::{
    Fault, Id, MAX_BYTES,
    container::{Draft, Store},
};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
};
use zeroize::Zeroizing;
fn read_secret(limit: usize) -> std::io::Result<Zeroizing<Vec<u8>>> {
    let mut size = [0; 4];
    std::io::stdin().read_exact(&mut size)?;
    let size = u32::from_le_bytes(size) as usize;
    if size > limit {
        return Err(std::io::ErrorKind::InvalidData.into());
    }
    let mut bytes = Zeroizing::new(vec![0; size]);
    std::io::stdin().read_exact(&mut bytes)?;
    Ok(bytes)
}
fn run() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let a: Vec<_> = std::env::args().collect();
    let root: Id = serde_json::from_slice(&fs::read(&a[2])?)?;
    let password = read_secret(65536)?;
    let store = Store::at(a[1].clone().into(), root);
    let state = store.load()?.unlock(&password)?;
    drop(password);
    let document = state.doc()?;
    println!("unlocked");
    std::io::stdout().flush()?;
    let draft = read_secret(MAX_BYTES / 8)?;
    let saved = Draft::save(
        &state,
        &draft,
        Path::new(&a[3]),
        if a[4] == "fail" {
            Fault::BeforeRename
        } else {
            Fault::None
        },
    )
    .is_ok();
    drop(draft);
    drop(document);
    drop(state);
    println!("{}", if saved { "saved" } else { "draft-failed" });
    std::io::stdout().flush()?;
    Ok(())
}
fn main() {
    if run().is_err() {
        std::process::exit(1);
    }
}
