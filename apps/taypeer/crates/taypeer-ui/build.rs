//! Track the shared translation resources used by the localization macro.
fn main() {
    // The shared locale directory lives outside this crate, so Cargo must track it explicitly.
    println!("cargo:rerun-if-changed=../../locales");
}
