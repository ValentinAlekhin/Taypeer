//! Content-free diagnostic for the pinned standard library on an Android shell.
//! Pass a fresh public-fixture path; the probe never overwrites an existing file.
fn main() -> std::io::Result<()> {
    let path = std::env::args_os()
        .nth(1)
        .ok_or_else(|| std::io::Error::other("fixture path required"))?;
    let file = std::fs::File::create_new(&path)?;
    match file.try_lock() {
        Ok(()) => println!("supported"),
        Err(std::fs::TryLockError::WouldBlock) => println!("would_block"),
        Err(std::fs::TryLockError::Error(error)) => {
            println!("unsupported_or_failed: {:?}", error.kind())
        }
    }
    drop(file);
    std::fs::remove_file(path)
}
