use super::*;
use std::{
    io::Write,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[test]
#[ignore = "subprocess helper; invoked only by busy_child_exits_after_parent_eof"]
fn busy_child() {
    let Some(ready) = std::env::var_os("TAYPEER_PUBLIC_EOF_TEST") else {
        return;
    };
    let (mut input, _lifetime) = Incoming::start(std::io::stdin());
    let mut frame = [0; 6];
    input.read_exact(&mut frame).unwrap();
    std::fs::write(ready, b"PUBLIC ready").unwrap();
    std::thread::sleep(Duration::from_secs(60));
}

#[test]
fn busy_child_exits_after_parent_eof() {
    let directory = tempfile::tempdir().unwrap();
    let ready = directory.path().join("PUBLIC ready");
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "worker::input::tests::busy_child", "--ignored"])
        .env("TAYPEER_PUBLIC_EOF_TEST", &ready)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    input.write_all(&[2, 0, 0, 0, b'{', b'}']).unwrap();
    let until = Instant::now() + Duration::from_secs(5);
    while !ready.exists() {
        if Instant::now() >= until {
            let _ = child.kill();
            panic!("child did not start");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    drop(input); // OS closes this same pipe when the real parent dies.
    let until = Instant::now() + Duration::from_secs(4);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert_eq!(status.code(), Some(70));
            break;
        }
        if Instant::now() >= until {
            let _ = child.kill();
            panic!("busy child survived EOF");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
