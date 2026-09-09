use taypeer_encrypted_sync_spike::kdf::calibrate;
fn main() {
    for target in [500, 1000, 5000] {
        match calibrate(target) {
            Ok(result) => println!("{}", serde_json::to_string(&result).unwrap()),
            Err(_) => {
                eprintln!("calibration failed");
                std::process::exit(1);
            }
        }
    }
}
