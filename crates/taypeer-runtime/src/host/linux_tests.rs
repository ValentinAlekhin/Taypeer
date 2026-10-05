//! Production Linux credential and worker path on public synthetic data only.
use super::*;
use crate::{
    Command,
    platform::{ProcessConnection, ProcessLauncher},
};
use std::io::{BufRead, BufReader, Write};
use std::process::{Command as Process, Stdio};

const MARKER: &str = "PUBLIC Linux worker ready";

#[test]
#[ignore = "Private worker helper invoked by Linux production integration scenarios"]
fn native_linux_child() {
    if std::env::var_os("TAYPEER_PUBLIC_NATIVE_LINUX").is_none() {
        return;
    }
    println!("\n{MARKER}");
    std::io::stdout().flush().unwrap();
    let result = crate::run_worker(std::io::stdin(), std::io::stdout());
    std::process::exit(if result.is_ok() { 0 } else { 70 });
}

struct Launcher;
impl ProcessLauncher for Launcher {
    fn launch(&self) -> Result<ProcessConnection, RuntimeError> {
        if let Some(path) = std::env::var_os("TAYPEER_PUBLIC_LINUX_WORKER") {
            return crate::platform::DesktopLauncher(Path::new(&path)).launch();
        }
        let mut child = Process::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "host::linux_tests::native_linux_child",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("TAYPEER_PUBLIC_NATIVE_LINUX", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| RuntimeError::Transport)?;
        let input = child.stdin.take().unwrap();
        let mut output = BufReader::new(child.stdout.take().unwrap());
        loop {
            let mut line = String::new();
            if output
                .read_line(&mut line)
                .map_err(|_| RuntimeError::Transport)?
                == 0
            {
                return Err(RuntimeError::Transport);
            }
            if line.trim() == MARKER {
                break;
            }
        }
        Ok(ProcessConnection::new(
            Box::new(crate::platform::DesktopProcess(Some(child))),
            Box::new(input),
            Box::new(output),
        ))
    }
}
fn open(
    host: &RuntimeHost,
    path: &Path,
    password: &str,
    create: bool,
) -> Result<Worker, RuntimeError> {
    host.open_with_launcher(
        &Launcher,
        path,
        password.to_owned(),
        create.then(|| taypeer_services::CreateDatabase {
            name: "PUBLIC Linux database".into(),
            description: None,
            policy: Default::default(),
        }),
    )
}
fn info(worker: &mut Worker) -> taypeer_services::DatabaseInfo {
    serde_json::from_value(worker.request(&Command::DatabaseInfo).unwrap()).unwrap()
}

#[test]
fn databases_activate_independently_after_restart_and_survive_lock() {
    let directory = tempfile::tempdir().unwrap();
    let profile = directory.path().join("profile");
    let first = directory.path().join("first.taypeer");
    let second = directory.path().join("second.taypeer");
    let host = RuntimeHost::new(&profile).unwrap();
    assert!(host.network_snapshot().unwrap().databases.is_empty());
    let mut a = open(&host, &first, "PUBLIC Linux first password", true).unwrap();
    let mut b = open(&host, &second, "PUBLIC Linux second password", true).unwrap();
    assert!(info(&mut a).writable && info(&mut b).writable);
    let a_id = a.database_id().clone();
    let b_id = b.database_id().clone();
    assert_ne!(a_id, b_id);
    let snapshot = host.network_snapshot().unwrap();
    assert_eq!(snapshot.databases.len(), 2);
    assert_ne!(
        host.context.for_database(&a_id).unwrap().transport.public(),
        host.context.for_database(&b_id).unwrap().transport.public()
    );
    a.request(&Command::CreateGroup {
        name: "PUBLIC saved group".into(),
        parent: None,
        operation: taypeer_services::new_operation_id().unwrap(),
    })
    .unwrap();
    a.close().unwrap();
    b.close().unwrap();
    drop(a);
    drop(b);
    drop(host);
    let host = RuntimeHost::new(&profile).unwrap();
    assert!(host.network_snapshot().unwrap().databases.is_empty());
    assert!(open(&host, &first, "PUBLIC incorrect password", false).is_err());
    assert!(host.network_snapshot().unwrap().databases.is_empty());
    let mut a = open(&host, &first, "PUBLIC Linux first password", false).unwrap();
    assert_eq!(a.database_id(), &a_id);
    let groups: Vec<taypeer_services::GroupInfo> =
        serde_json::from_value(a.request(&Command::GroupInfo).unwrap()).unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].group.name, "PUBLIC saved group");
    let first_address = host
        .start_network(taypeer_sync::RelaySetting::Disabled)
        .unwrap();
    a.close().unwrap();
    assert_eq!(host.network_address_for(&a_id).unwrap(), first_address);
    assert!(open(&host, &first, "PUBLIC incorrect password", false).is_err());
    assert!(host.compatibility(&a_id).unwrap().admitted);
    assert!(host.compatibility(&b_id).is_err());
    let mut b = open(&host, &second, "PUBLIC Linux second password", false).unwrap();
    assert_eq!(b.database_id(), &b_id);
    assert_ne!(
        host.network_address_for(&a_id).unwrap().id,
        host.network_address_for(&b_id).unwrap().id
    );
    assert_eq!(host.network_snapshot().unwrap().databases.len(), 2);
    b.close().unwrap();
}

#[test]
fn copied_database_without_credentials_is_read_only_and_corruption_is_an_error() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("original.taypeer");
    let original_profile = directory.path().join("original-profile");
    let host = RuntimeHost::new(&original_profile).unwrap();
    let mut worker = open(&host, &path, "PUBLIC Linux copy password", true).unwrap();
    let database = worker.database_id().clone();
    worker.close().unwrap();
    drop(worker);
    drop(host);
    let copy = directory.path().join("copy.taypeer");
    std::fs::copy(&path, &copy).unwrap();
    let read_host = RuntimeHost::new(&directory.path().join("read-profile")).unwrap();
    let mut read = open(&read_host, &copy, "PUBLIC Linux copy password", false).unwrap();
    assert!(!info(&mut read).writable);
    assert!(!read_host.compatibility(&database).unwrap().admitted);
    assert!(
        read_host
            .start_network(taypeer_sync::RelaySetting::Disabled)
            .is_err()
    );
    assert!(!read_host.network_snapshot().unwrap().running);
    assert!(
        read.request(&Command::CreateGroup {
            name: "PUBLIC forbidden".into(),
            parent: None,
            operation: taypeer_services::new_operation_id().unwrap()
        })
        .is_err()
    );
    read.close().unwrap();
    let mut read = open(&read_host, &copy, "PUBLIC Linux copy password", false).unwrap();
    assert!(!info(&mut read).writable);
    read.close().unwrap();
    let credential = original_profile
        .join("databases")
        .join(Digest::of(database.as_str().as_bytes()).to_string())
        .join("credentials");
    let mut bytes = std::fs::read(&credential).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    std::fs::write(&credential, &bytes).unwrap();
    let host = RuntimeHost::new(&original_profile).unwrap();
    assert!(open(&host, &path, "PUBLIC Linux copy password", false).is_err());
    assert_eq!(std::fs::read(&credential).unwrap(), bytes);
    assert!(host.network_snapshot().unwrap().databases.is_empty());
}

#[test]
fn password_rotation_confirms_local_credentials_before_success() {
    let directory = tempfile::tempdir().unwrap();
    let profile = directory.path().join("profile");
    let path = directory.path().join("rotation.taypeer");
    let host = RuntimeHost::new(&profile).unwrap();
    let mut worker = open(&host, &path, "PUBLIC Linux prior password", true).unwrap();
    let database = worker.database_id().clone();
    let mut command = Command::RotatePassword {
        operation: Digest::of(b"PUBLIC Linux rotation operation"),
        password: zeroize::Zeroizing::new(b"PUBLIC Linux rotated password".to_vec()),
        revoke: None,
    };
    let rotated = worker.request(&command);
    command.erase_input();
    rotated.unwrap();
    let context = host.context.for_database(&database).unwrap();
    assert_eq!(
        context
            .profile
            .load_state::<Option<u64>>("credential_prepared")
            .unwrap(),
        Some(None)
    );
    assert!(
        context
            .profile
            .load_state::<u64>("credential_final")
            .unwrap()
            .is_some()
    );
    worker.close().unwrap();
    drop(worker);
    drop(context);
    drop(host);
    let host = RuntimeHost::new(&profile).unwrap();
    assert!(open(&host, &path, "PUBLIC Linux prior password", false).is_err());
    let mut worker = open(&host, &path, "PUBLIC Linux rotated password", false).unwrap();
    assert!(info(&mut worker).writable);
    worker.close().unwrap();
}

#[test]
fn rotation_failures_recover_the_durable_epoch_without_false_success() {
    // Test-only host faults surround the real worker's credential and database commits.
    for fault in [1, 2, 3, 4] {
        let directory = tempfile::tempdir().unwrap();
        let profile = directory.path().join("profile");
        let path = directory.path().join("fault.taypeer");
        let host = RuntimeHost::new(&profile).unwrap();
        let mut worker = open(&host, &path, "PUBLIC Linux original fault password", true).unwrap();
        let database = worker.database_id().clone();
        let context = host.context.for_database(&database).unwrap();
        context
            .credential_fault
            .store(fault, std::sync::atomic::Ordering::Release);
        let mut command = Command::RotatePassword {
            operation: Digest::of(b"PUBLIC Linux fault rotation operation"),
            password: zeroize::Zeroizing::new(b"PUBLIC Linux next fault password".to_vec()),
            revoke: None,
        };
        let result = worker.request(&command);
        command.erase_input();
        assert!(
            result.is_err(),
            "a failed credential transaction cannot confirm success"
        );
        worker.invalidate(crate::session::LockReason::Manual);
        let _report = worker.close_report();
        drop(worker);
        drop(context);
        drop(host);
        let host = RuntimeHost::new(&profile).unwrap();
        let password = if fault <= 2 {
            "PUBLIC Linux original fault password"
        } else {
            "PUBLIC Linux next fault password"
        };
        let mut recovered = open(&host, &path, password, false).unwrap();
        assert!(info(&mut recovered).writable);
        let context = host.context.for_database(&database).unwrap();
        assert_eq!(
            context
                .profile
                .load_state::<Option<u64>>("credential_prepared")
                .unwrap(),
            Some(None)
        );
        recovered.close().unwrap();
    }
}

fn enrollment_executable(directory: &Path) -> PathBuf {
    if let Some(path) = std::env::var_os("TAYPEER_PUBLIC_LINUX_WORKER") {
        return PathBuf::from(path);
    }
    // Strip the Rust harness prefix before forwarding the production private IPC.
    // The child inherits stdin, including EOF. Serial libtest mode avoids its
    // asynchronous 60-second warning corrupting the private stdout protocol.
    let path = directory.join("public-worker");
    let binary = serde_json::to_string(&std::env::current_exe().unwrap()).unwrap();
    let script = format!(
        "#!/usr/bin/env python3\nimport os, subprocess, sys, shutil\nenv = dict(os.environ)\nenv['TAYPEER_PUBLIC_NATIVE_LINUX'] = '1'\nchild = subprocess.Popen([{binary}, '--exact', 'host::linux_tests::native_linux_child', '--ignored', '--nocapture', '--test-threads=1'], stdin=sys.stdin, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, env=env)\nwhile True:\n    line = child.stdout.readline()\n    if not line:\n        sys.exit(70)\n    if line.strip() == b'PUBLIC Linux worker ready':\n        break\nwhile True:\n    chunk = child.stdout.read1(65536)\n    if not chunk:\n        break\n    sys.stdout.buffer.write(chunk)\n    sys.stdout.buffer.flush()\nsys.exit(child.wait())\n"
    );
    std::fs::write(&path, script).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    path
}

#[test]
fn invitation_retries_wrong_password_and_remote_rotation_rewraps_credentials() {
    let directory = tempfile::tempdir().unwrap();
    let executable = enrollment_executable(directory.path());
    let manager = RuntimeHost::new(&directory.path().join("manager")).unwrap();
    let mut writer = open(
        &manager,
        &directory.path().join("original.taypeer"),
        "PUBLIC Linux invitation password",
        true,
    )
    .unwrap();
    let database = writer.database_id().clone();
    writer
        .request(&Command::CreateGroup {
            name: "PUBLIC invited group".into(),
            parent: None,
            operation: taypeer_services::new_operation_id().unwrap(),
        })
        .unwrap();
    manager
        .start_network(taypeer_sync::RelaySetting::Disabled)
        .unwrap();
    let material = writer.request(&Command::CreateInvitation).unwrap();
    let (invitation, secret): (taypeer_trust::Invitation, zeroize::Zeroizing<[u8; 32]>) =
        serde_json::from_value(material).unwrap();
    let request = invitation.id().unwrap();
    let code = crate::InvitationCode {
        invitation,
        secret,
        address: manager.network_address_for(&database).unwrap(),
    };
    let recipient_profile = directory.path().join("recipient");
    let recipient = RuntimeHost::new(&recipient_profile).unwrap();
    let destination = directory.path().join("received.taypeer");
    assert!(matches!(
        recipient
            .join(
                &executable,
                code,
                &destination,
                "PUBLIC wrong initial password".into()
            )
            .unwrap(),
        crate::JoinProgress::Pending(_)
    ));
    writer
        .request(&Command::ApproveInvitation(request))
        .unwrap();
    assert!(
        recipient
            .resume_join(request, "PUBLIC wrong initial password".into())
            .is_err()
    );
    assert!(
        recipient
            .pending_join_summaries()
            .unwrap()
            .contains_key(&request)
    );
    assert!(matches!(
        recipient
            .resume_join(request, "PUBLIC Linux invitation password".into())
            .unwrap(),
        crate::JoinProgress::Received(_)
    ));
    let mut reader = open(
        &recipient,
        &destination,
        "PUBLIC Linux invitation password",
        false,
    )
    .unwrap();
    assert!(info(&mut reader).writable);
    let groups: Vec<taypeer_services::GroupInfo> =
        serde_json::from_value(reader.request(&Command::GroupInfo).unwrap()).unwrap();
    assert_eq!(groups.len(), 1);
    reader.close().unwrap();
    let address = recipient.network_address_for(&database).unwrap();
    let mut rotation = Command::RotatePassword {
        operation: Digest::of(b"PUBLIC remote rotation"),
        password: zeroize::Zeroizing::new(b"PUBLIC Linux remote next password".to_vec()),
        revoke: None,
    };
    let rotated = writer.request(&rotation);
    rotation.erase_input();
    rotated.unwrap();
    manager.exchange(address, database.clone()).unwrap();
    let mut reader = open(
        &recipient,
        &destination,
        "PUBLIC Linux remote next password",
        false,
    )
    .unwrap();
    assert!(info(&mut reader).writable);
    reader.close().unwrap();
    writer.close().unwrap();
    drop(reader);
    drop(recipient);
    let recipient = RuntimeHost::new(&recipient_profile).unwrap();
    let mut reader = open(
        &recipient,
        &destination,
        "PUBLIC Linux remote next password",
        false,
    )
    .unwrap();
    assert!(info(&mut reader).writable);
    reader.close().unwrap();
}

#[test]
fn creation_finalization_failure_recovers_existing_managing_credentials() {
    let directory = tempfile::tempdir().unwrap();
    let profile = directory.path().join("profile");
    let path = directory.path().join("creation-fault.taypeer");
    let host = RuntimeHost::new(&profile).unwrap();
    host.context
        .credential_fault
        .store(3, std::sync::atomic::Ordering::Release);
    let result = open(&host, &path, "PUBLIC Linux creation fault password", true);
    assert!(matches!(
        result,
        Err(RuntimeError::Service(
            taypeer_services::ServiceError::Storage(taypeer_storage::Error::CommitUncertain)
        ))
    ));
    assert!(
        path.is_file(),
        "the database commit preceded marker failure"
    );
    drop(host);

    let host = RuntimeHost::new(&profile).unwrap();
    let mut recovered = open(&host, &path, "PUBLIC Linux creation fault password", false).unwrap();
    let database = recovered.database_id().clone();
    assert!(info(&mut recovered).writable);
    let context = host.context.for_database(&database).unwrap();
    assert_eq!(
        context
            .profile
            .load_state::<u64>("credential_final")
            .unwrap(),
        Some(0)
    );
    recovered.close().unwrap();
}
