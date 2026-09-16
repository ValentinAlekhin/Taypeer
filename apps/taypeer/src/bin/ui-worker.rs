//! Private worker for public synthetic UI fixtures only.
fn main() {
    if !std::env::args().skip(1).eq(["__worker"]) {
        std::process::exit(2);
    }
    let result = taypeer_runtime::run_test_worker(std::io::stdin(), std::io::stdout());
    std::process::exit(if result.is_ok() { 0 } else { 1 });
}
