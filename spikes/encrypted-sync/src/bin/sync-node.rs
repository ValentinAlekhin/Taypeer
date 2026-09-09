fn main() {
    std::process::exit(taypeer_encrypted_sync_spike::network::main_entry(
        &std::env::args().collect::<Vec<_>>(),
    ));
}
