//! Real isolated workers and signed ciphertext; all inputs are synthetic PUBLIC data.
use super::*;
use crate::platform::{ProcessConnection, ProcessLauncher};
use std::{
    collections::BTreeSet,
    io::{BufRead, BufReader, Read, Write},
    path::PathBuf,
    process::{Command as Process, Stdio},
    sync::Barrier,
    time::Instant,
};
use taypeer_services::{DatabaseService, GroupInfo, SessionToken};
use taypeer_storage::{ArchiveSnapshot, PreparedCommit};
use taypeer_sync::{Backend, Command as NetworkCommand, Coordinator, EndpointAddr};
use taypeer_trust::{Digest, Identity, JoinProof, ObjectKind};

const PASSWORD: &str = "PUBLIC automation synchronization password";
const MARKER: &str = "PUBLIC automation synchronization worker";

#[test]
#[ignore = "Private subprocess helper for synthetic synchronization scenarios"]
fn child() {
    if std::env::var_os("TAYPEER_PUBLIC_AUTOMATION_SYNC").is_none() {
        return;
    }
    println!("\n{MARKER}");
    std::io::stdout().flush().unwrap();
    let result = run_test_worker(std::io::stdin(), std::io::stdout());
    std::process::exit(if result.is_ok() { 0 } else { 70 });
}

#[derive(Default)]
struct Launcher {
    after_boot: Option<Arc<dyn Fn() + Send + Sync>>,
}
impl ProcessLauncher for Launcher {
    fn launch(&self) -> Result<ProcessConnection, RuntimeError> {
        let mut child = Process::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "automation_sync_tests::child",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("TAYPEER_PUBLIC_AUTOMATION_SYNC", "1")
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
            Box::new(AfterBoot {
                output,
                incoming: Vec::new(),
                action: self.after_boot.clone(),
            }),
        ))
    }
}
// Deliver ciphertext after the child's startup apply, before its boot response
// reaches Worker::open. This reproduces the old subscription window exactly.
struct AfterBoot<R> {
    output: R,
    incoming: Vec<u8>,
    action: Option<Arc<dyn Fn() + Send + Sync>>,
}
impl<R: Read> Read for AfterBoot<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let count = self.output.read(buffer)?;
        if self.action.is_some() {
            self.incoming.extend_from_slice(&buffer[..count]);
            while self.incoming.len() >= 4 {
                let size = u32::from_le_bytes(self.incoming[..4].try_into().unwrap()) as usize;
                if self.incoming.len() < size + 4 {
                    break;
                }
                let response =
                    serde_json::from_slice::<Value>(&self.incoming[4..size + 4]).unwrap();
                self.incoming.drain(..size + 4);
                if response.get("Response").is_some() {
                    self.action.take().unwrap()();
                    self.incoming.clear();
                    break;
                }
            }
        }
        Ok(count)
    }
}

fn host(path: &Path) -> RuntimeHost {
    RuntimeHost::with_test_sessions(path, SessionController::new(Default::default())).unwrap()
}
struct Receiver {
    host: RuntimeHost,
    path: PathBuf,
}
struct Fixture {
    _directory: tempfile::TempDir,
    manager: RuntimeHost,
    service: DatabaseService,
    session: SessionToken,
    receivers: Vec<Receiver>,
}
impl Fixture {
    fn new(receivers: usize) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let manager = host(&directory.path().join("manager-profile"));
        let path = directory.path().join("manager.taypeer");
        let mut service = DatabaseService::new();
        let session = manager
            .open_local(
                &mut service,
                &path,
                PASSWORD.as_bytes(),
                Some("PUBLIC database".into()),
            )
            .unwrap();
        let mut targets = Vec::new();
        for index in 0..receivers {
            let receiver = host(&directory.path().join(format!("receiver-{index}-profile")));
            let author = receiver.profile().author().unwrap();
            let identity =
                Identity::new(author.public(), receiver.context.transport.public()).unwrap();
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();
            let (invitation, secret) = service.create_invitation(&session, now).unwrap();
            let request = invitation.id().unwrap();
            let proof = JoinProof::sign(&invitation, identity, &author).unwrap();
            manager
                .coordinator()
                .command(
                    receiver.context.transport.public(),
                    NetworkCommand::Join {
                        invitation: Box::new(invitation),
                        secret: zeroize::Zeroizing::new(*secret.expose()),
                        proof: Box::new(proof),
                        address: EndpointAddr::new(
                            receiver
                                .context
                                .transport
                                .public()
                                .to_string()
                                .parse()
                                .unwrap(),
                        ),
                    },
                )
                .unwrap();
            service
                .approve_invitation(&session, request, now + 1)
                .unwrap();
            targets.push(Receiver {
                host: receiver,
                path: directory.path().join(format!("receiver-{index}.taypeer")),
            });
        }
        let snapshot = manager.coordinator().snapshot(&session.database).unwrap();
        for target in &targets {
            snapshot
                .candidate()
                .export(snapshot.metadata().clone(), &target.path)
                .unwrap();
            target
                .host
                .context
                .attach(&target.path.canonicalize().unwrap())
                .unwrap();
        }
        Self {
            _directory: directory,
            manager,
            service,
            session,
            receivers: targets,
        }
    }
    fn group(&mut self, name: &str) -> Digest {
        let before = self
            .manager
            .coordinator()
            .snapshot(&self.session.database)
            .unwrap();
        self.service
            .create_group(
                &self.session,
                name.into(),
                None,
                &taypeer_services::new_operation_id().unwrap(),
            )
            .unwrap();
        let after = self
            .manager
            .coordinator()
            .snapshot(&self.session.database)
            .unwrap();
        let ids: Vec<_> = after
            .metadata()
            .manifest
            .body
            .objects
            .keys()
            .filter(|id| !before.metadata().manifest.body.objects.contains_key(id))
            .filter(|id| after.object(**id).unwrap().envelope().kind == ObjectKind::Change)
            .copied()
            .collect();
        assert_eq!(ids.len(), 1);
        ids[0]
    }
    fn worker(&self, index: usize, launcher: &Launcher) -> Worker {
        let target = &self.receivers[index];
        target
            .host
            .open_with_launcher(launcher, &target.path, PASSWORD.into(), None)
            .unwrap()
    }
    fn snapshot(&self) -> ArchiveSnapshot {
        self.manager
            .coordinator()
            .snapshot(&self.session.database)
            .unwrap()
    }
    fn deliver(&self, index: usize, object: Digest) {
        deliver(
            &self.snapshot(),
            self.manager.context.transport.public(),
            self.receivers[index].host.coordinator(),
            object,
        );
    }
}
fn deliver(
    snapshot: &ArchiveSnapshot,
    peer: taypeer_trust::PublicKey,
    to: &Coordinator,
    id: Digest,
) {
    to.command(
        peer,
        NetworkCommand::Offer(Box::new(snapshot.metadata().clone())),
    )
    .unwrap();
    let mut staged = tempfile::NamedTempFile::new().unwrap();
    std::io::copy(&mut snapshot.reader(id).unwrap(), &mut staged).unwrap();
    to.receive(
        peer,
        &snapshot.chain().head().database,
        snapshot.metadata().manifest.body.objects[&id].clone(),
        staged,
    )
    .unwrap();
}
fn wait(mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        if predicate() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "synchronization did not reach the expected public state"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}
fn groups(worker: &mut Worker) -> Vec<GroupInfo> {
    serde_json::from_value(worker.request(&Command::GroupInfo).unwrap()).unwrap()
}
fn flood(coordinator: &Coordinator, database: &DatabaseId) {
    for _ in 0..260 {
        let snapshot = coordinator.snapshot(database).unwrap();
        coordinator
            .commit(
                database,
                PreparedCommit {
                    expected: snapshot.fingerprint(),
                    control: snapshot.chain().head_hash().unwrap(),
                    controls: snapshot.chain().records().to_vec(),
                    objects: Vec::new(),
                    remove: BTreeSet::new(),
                    checkpoint: snapshot.metadata().manifest.body.checkpoint,
                    baseline: snapshot.metadata().manifest.body.baseline,
                    journal: snapshot.metadata().journal.clone(),
                },
            )
            .unwrap();
    }
}
// Pause only async event consumers. IPC, the supervisor and ciphertext writes
// remain active on their independent OS threads, as in a delayed UI host.
struct PausedConsumers(Option<Arc<Barrier>>);
impl PausedConsumers {
    fn resume(mut self) {
        self.0.take().unwrap().wait();
    }
}
impl Drop for PausedConsumers {
    fn drop(&mut self) {
        if let Some(release) = self.0.take() {
            release.wait();
        }
    }
}
fn pause_consumers(host: &RuntimeHost) -> PausedConsumers {
    let count = host.runtime.metrics().num_workers();
    let release = Arc::new(Barrier::new(count + 1));
    for _ in 0..count {
        let (ready, entered) = std::sync::mpsc::channel();
        let release = Arc::clone(&release);
        host.runtime.spawn(async move {
            ready.send(()).unwrap();
            release.wait();
        });
        entered.recv_timeout(Duration::from_secs(3)).unwrap();
    }
    PausedConsumers(Some(release))
}

#[test]
fn delivery_in_the_boot_subscription_window_is_applied_before_open_returns() {
    let mut f = Fixture::new(1);
    let object = f.group("PUBLIC gap group");
    let snapshot = f.snapshot();
    let peer = f.manager.context.transport.public();
    let coordinator = Arc::clone(f.receivers[0].host.coordinator());
    let launcher = Launcher {
        after_boot: Some(Arc::new(move || {
            deliver(&snapshot, peer, &coordinator, object)
        })),
    };
    let mut worker = f.worker(0, &launcher);
    assert_eq!(groups(&mut worker)[0].group.name, "PUBLIC gap group");
    worker.close().unwrap();
}

#[test]
fn locked_receipt_is_applied_on_unlock_and_remains_confirmed_after_host_restart() {
    let mut f = Fixture::new(1);
    let mut worker = f.worker(0, &Launcher::default());
    worker.close().unwrap();
    let object = f.group("PUBLIC locked receipt");
    let before = f.receivers[0]
        .host
        .coordinator()
        .snapshot(&f.session.database)
        .unwrap();
    f.deliver(0, object);
    let received = f.receivers[0]
        .host
        .coordinator()
        .snapshot(&f.session.database)
        .unwrap();
    assert_eq!(
        received.metadata().manifest.body.checkpoint,
        before.metadata().manifest.body.checkpoint
    );
    assert!(
        received
            .metadata()
            .manifest
            .body
            .objects
            .contains_key(&object)
    );
    drop(worker);
    let target = f.receivers.remove(0);
    let profile = target.host.profile().directory().to_owned();
    let path = target.path.clone();
    drop(target);
    let restarted = host(&profile);
    let mut worker = restarted
        .open_with_launcher(&Launcher::default(), &path, PASSWORD.into(), None)
        .unwrap();
    assert_eq!(groups(&mut worker)[0].group.name, "PUBLIC locked receipt");
    worker.close().unwrap();
}

#[test]
fn dependency_delivery_permutations_complete_automatically_without_extra_history() {
    let mut f = Fixture::new(6);
    let objects = [
        f.group("PUBLIC one"),
        f.group("PUBLIC two"),
        f.group("PUBLIC three"),
    ];
    let permutations = [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ];
    for (index, permutation) in permutations.into_iter().enumerate() {
        let mut worker = f.worker(index, &Launcher::default());
        for (position, next) in permutation.into_iter().enumerate() {
            f.deliver(index, objects[next]);
            if position == 0 && next != 0 {
                wait(|| {
                    worker.application_progress().is_some_and(|result| {
                        result.is_ok_and(|value| {
                            serde_json::from_value::<taypeer_services::ApplyReport>(value)
                                .unwrap()
                                .pending
                                .iter()
                                .any(|packet| {
                                    packet.reason == taypeer_services::PendingReason::Dependency
                                })
                        })
                    })
                });
                assert!(groups(&mut worker).is_empty());
            }
        }
        wait(|| groups(&mut worker).len() == 3);
        let mut names: Vec<_> = groups(&mut worker)
            .iter()
            .map(|row| row.group.name.clone())
            .collect();
        names.sort();
        assert_eq!(names, ["PUBLIC one", "PUBLIC three", "PUBLIC two"]);
        for row in groups(&mut worker) {
            let history = worker
                .request(&Command::GroupHistory(row.group.id))
                .unwrap();
            assert_eq!(history.as_array().unwrap().len(), 1);
        }
        for object in objects {
            f.deliver(index, object);
        }
        for row in groups(&mut worker) {
            assert_eq!(
                worker
                    .request(&Command::GroupHistory(row.group.id))
                    .unwrap()
                    .as_array()
                    .unwrap()
                    .len(),
                1
            );
        }
        worker.close().unwrap();
    }
}

#[test]
fn lag_reconciles_received_data_without_revoking_a_healthy_worker() {
    let mut f = Fixture::new(1);
    let mut worker = f.worker(0, &Launcher::default());
    let object = f.group("PUBLIC lag receipt");
    let release = pause_consumers(&f.receivers[0].host);
    f.deliver(0, object);
    flood(f.receivers[0].host.coordinator(), &f.session.database);
    release.resume();
    wait(|| groups(&mut worker).len() == 1);
    assert!(worker.is_open());
    worker.close().unwrap();
}

#[test]
fn lag_cannot_hide_a_password_epoch_rotation() {
    let f = Fixture::new(1);
    let mut worker = f.worker(0, &Launcher::default());
    let release = pause_consumers(&f.receivers[0].host);
    let mut f = f;
    f.service
        .rotate_password(
            &f.session,
            Digest::of(b"PUBLIC missed rotation"),
            b"PUBLIC next password",
            None,
        )
        .unwrap();
    f.receivers[0]
        .host
        .coordinator()
        .command(
            f.manager.context.transport.public(),
            NetworkCommand::Offer(Box::new(f.snapshot().metadata().clone())),
        )
        .unwrap();
    let rotated = f.snapshot();
    for id in rotated.metadata().manifest.body.objects.keys() {
        f.deliver(0, *id);
    }
    let prior = f.receivers[0]
        .host
        .coordinator()
        .snapshot(&f.session.database)
        .unwrap();
    f.receivers[0]
        .host
        .coordinator()
        .commit(
            &f.session.database,
            PreparedCommit {
                expected: prior.fingerprint(),
                control: prior.chain().head_hash().unwrap(),
                controls: prior.chain().records().to_vec(),
                objects: Vec::new(),
                remove: BTreeSet::new(),
                checkpoint: rotated.metadata().manifest.body.checkpoint,
                baseline: rotated.metadata().manifest.body.baseline,
                journal: prior.metadata().journal.clone(),
            },
        )
        .unwrap();
    flood(f.receivers[0].host.coordinator(), &f.session.database);
    release.resume();
    wait(|| !worker.is_open());
    assert!(worker.application_progress().is_none());
    assert!(worker.request(&Command::GroupInfo).is_err());
    let _closed = worker.close_report();
}

#[test]
fn application_retries_storage_failure_without_another_network_event() {
    for (suffix, expected) in [
        ("fail-before", taypeer_storage::Error::Io),
        ("fail-after", taypeer_storage::Error::CommitUncertain),
    ] {
        let mut f = Fixture::new(1);
        let mut worker = f.worker(0, &Launcher::default());
        let object = f.group("PUBLIC retried source");
        let marker = f.receivers[0].path.with_extension(suffix);
        std::fs::write(&marker, b"PUBLIC test fault").unwrap();
        f.deliver(0, object);
        wait(|| {
            matches!(worker.application_progress(),
            Some(Err(RuntimeError::Service(taypeer_services::ServiceError::Storage(error)))) if error == expected)
        });
        std::fs::remove_file(marker).unwrap();
        wait(|| groups(&mut worker).len() == 1);
        wait(|| matches!(worker.application_progress(), Some(Ok(_))));
        let rows = groups(&mut worker);
        assert_eq!(
            worker
                .request(&Command::GroupHistory(rows[0].group.id.clone()))
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            1
        );
        worker.close().unwrap();
    }
}

#[test]
fn malformed_decrypted_packet_is_quarantined_while_an_independent_source_is_saved() {
    let mut f = Fixture::new(1);
    let mut worker = f.worker(0, &Launcher::default());
    let eligible = f.group("PUBLIC eligible despite quarantine");
    let snapshot = f.snapshot();
    let checkpoint = snapshot
        .object(snapshot.metadata().manifest.body.checkpoint)
        .unwrap();
    let key = checkpoint.unlock_key(PASSWORD.as_bytes()).unwrap();
    let malformed = taypeer_storage::EncryptedObject::seal(
        snapshot.chain(),
        &f.manager.profile().author().unwrap(),
        ObjectKind::Change,
        &checkpoint.password_header().unwrap(),
        &key,
        b"PUBLIC invalid decoded source".as_slice(),
        29,
    )
    .unwrap();
    let malformed_id = malformed.descriptor().digest;
    f.manager
        .coordinator()
        .commit(
            &f.session.database,
            PreparedCommit {
                expected: snapshot.fingerprint(),
                control: snapshot.chain().head_hash().unwrap(),
                controls: snapshot.chain().records().to_vec(),
                objects: vec![malformed],
                remove: BTreeSet::new(),
                checkpoint: snapshot.metadata().manifest.body.checkpoint,
                baseline: snapshot.metadata().manifest.body.baseline,
                journal: snapshot.metadata().journal.clone(),
            },
        )
        .unwrap();
    f.deliver(0, malformed_id);
    f.deliver(0, eligible);
    wait(|| groups(&mut worker).len() == 1);
    wait(|| {
        worker.application_progress().is_some_and(|result| {
            result.is_ok_and(|value| {
                serde_json::from_value::<taypeer_services::ApplyReport>(value)
                    .unwrap()
                    .pending
                    .iter()
                    .any(|packet| {
                        packet.object == malformed_id
                            && packet.reason == taypeer_services::PendingReason::Invalid
                    })
            })
        })
    });
    assert!(worker.is_open());
    assert_eq!(
        groups(&mut worker)[0].group.name,
        "PUBLIC eligible despite quarantine"
    );
    worker.close().unwrap();
}

#[test]
fn revoked_and_dependent_sources_do_not_block_automatic_independent_application() {
    let mut f = Fixture::new(3);
    let mut revoked = f.worker(1, &Launcher::default());
    let mut forwarding = f.worker(2, &Launcher::default());
    let create = |worker: &mut Worker, name: &str| {
        worker
            .request(&Command::CreateGroup {
                name: name.into(),
                parent: None,
                operation: taypeer_services::new_operation_id().unwrap(),
            })
            .unwrap();
    };
    create(&mut revoked, "PUBLIC revoked source");
    create(&mut forwarding, "PUBLIC independent source");
    let from_revoked = f.receivers[1]
        .host
        .coordinator()
        .snapshot(&f.session.database)
        .unwrap();
    for object in from_revoked.metadata().manifest.body.objects.keys() {
        deliver(
            &from_revoked,
            f.receivers[1].host.context.transport.public(),
            f.receivers[2].host.coordinator(),
            *object,
        );
    }
    wait(|| groups(&mut forwarding).len() == 2);
    create(&mut forwarding, "PUBLIC dependent source");
    let wrapped = f.receivers[2]
        .host
        .coordinator()
        .snapshot(&f.session.database)
        .unwrap();
    let excluded = f.receivers[1].host.profile().author().unwrap().device_id();
    f.service
        .rotate_password(
            &f.session,
            Digest::of(b"PUBLIC automatic quarantine revocation"),
            b"PUBLIC quarantine next password",
            Some(excluded),
        )
        .unwrap();
    let rotation = f.snapshot();
    for object in rotation.metadata().manifest.body.objects.keys() {
        f.deliver(0, *object);
    }
    for object in wrapped.metadata().manifest.body.objects.keys() {
        deliver(
            &wrapped,
            f.receivers[2].host.context.transport.public(),
            f.receivers[0].host.coordinator(),
            *object,
        );
    }
    let target = &f.receivers[0];
    let mut worker = target
        .host
        .open_with_launcher(
            &Launcher::default(),
            &target.path,
            "PUBLIC quarantine next password".into(),
            None,
        )
        .unwrap();
    let current = groups(&mut worker);
    assert_eq!(current.len(), 1);
    assert_eq!(current[0].group.name, "PUBLIC independent source");
    let report: taypeer_services::ApplyReport =
        serde_json::from_value(worker.application_progress().unwrap().unwrap()).unwrap();
    assert!(
        report
            .pending
            .iter()
            .any(|packet| packet.reason == taypeer_services::PendingReason::RevokedAuthor)
    );
    assert!(
        report
            .pending
            .iter()
            .any(|packet| packet.reason == taypeer_services::PendingReason::Dependency)
    );
    assert_eq!(
        worker
            .request(&Command::GroupHistory(current[0].group.id.clone()))
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        1
    );
    worker.close().unwrap();
    forwarding.close().unwrap();
    revoked.close().unwrap();
}
