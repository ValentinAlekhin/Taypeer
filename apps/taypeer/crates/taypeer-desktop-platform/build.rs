//! Compile the windowless macOS lifecycle and pasteboard adapter.
fn main() {
    println!("cargo:rerun-if-changed=native/Platform.swift");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    let directory = std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("Cargo OUT_DIR"));
    let helper = directory.join("taypeer-platform");
    let status = std::process::Command::new("xcrun")
        .args(["swiftc", "-O", "native/Platform.swift", "-o"])
        .arg(&helper)
        .status()
        .expect("macOS Xcode command line tools");
    assert!(
        status.success(),
        "could not compile the macOS platform helper"
    );
    println!(
        "cargo:rustc-env=TAYPEER_PLATFORM_HELPER={}",
        helper.display()
    );
}
