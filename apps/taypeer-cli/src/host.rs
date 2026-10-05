use crate::{
    args::{Action, EntryCommand, GenerateCommand, GroupCommand},
    input::Input,
    output::CliError,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
use taypeer_core::{AttributeId, EntryId, GroupId, OperationId};
use taypeer_runtime::{Command, RuntimeHost, Worker};
use taypeer_services::{GroupMove, ObjectId};
mod databases;
mod drafts;
mod history;
mod p2p;

struct Database {
    path: PathBuf,
    worker: Option<Worker>,
    closure: Option<taypeer_runtime::session::LockOutcome>,
}

pub(crate) struct Host {
    pub sessions: taypeer_runtime::session::SessionController,
    executable: PathBuf,
    databases: BTreeMap<String, Database>,
    selected: Option<String>,
    pub input: Input,
    runtime: Option<RuntimeHost>,
    profile: PathBuf,
    #[cfg(feature = "ui-test-support")]
    public_fixture_profile: bool,
}

impl Host {
    /// Inspect a standalone file without credentials, registration or a plaintext worker.
    pub fn file_compatibility(path: &Path) -> Result<Value, CliError> {
        let (database, format) = RuntimeHost::inspect_compatibility(path)?;
        Ok(json!({
            "database": database,
            "locked": true,
            "admitted": null,
            "format": format
        }))
    }
    pub fn new(
        mut input: Input,
        profile: Option<PathBuf>,
        #[cfg(feature = "ui-test-support")] public_fixture_profile: bool,
    ) -> Result<Self, CliError> {
        let profile = match profile {
            Some(path) => path,
            None => RuntimeHost::default_profile_path()?,
        };
        let sessions = taypeer_runtime::session::SessionController::new(
            taypeer_runtime::session::SessionSettings::load(&profile)?,
        );
        input.activity = Some(sessions.activity());
        Ok(Self {
            sessions,
            executable: std::env::current_exe().map_err(|_| CliError::Io)?,
            databases: BTreeMap::new(),
            selected: None,
            input,
            runtime: None,
            profile,
            #[cfg(feature = "ui-test-support")]
            public_fixture_profile,
        })
    }

    pub fn open(&mut self, path: &Path) -> Result<Value, CliError> {
        let path = path.canonicalize().map_err(|_| CliError::Input)?;
        let password = self.input.password(false)?;
        self.ensure_runtime()?;
        let runtime = self.runtime.as_ref().ok_or(CliError::Io)?;
        let copy = runtime
            .working_copies()?
            .into_iter()
            .find(|copy| copy.path == path);
        let worker = if copy.is_some() {
            runtime.open(&self.executable, &path, password.to_string(), None)?
        } else {
            runtime.open_external(&self.executable, &path, password.to_string())?
        };
        self.remember_open(worker)
    }

    fn remember_open(&mut self, mut worker: Worker) -> Result<Value, CliError> {
        let id = worker.database_id().as_str().to_owned();
        if self.databases.contains_key(&id) {
            worker.close()?;
            return Err(CliError::AlreadyOpen);
        }
        let path = self
            .runtime
            .as_ref()
            .ok_or(CliError::Io)?
            .working_copies()?
            .into_iter()
            .find(|copy| copy.database.as_str() == id)
            .ok_or(CliError::UnknownDatabase)?
            .path;
        self.databases.insert(
            id.clone(),
            Database {
                path: path.clone(),
                worker: Some(worker),
                closure: None,
            },
        );
        self.selected = Some(id.clone());
        Ok(json!({"database": id, "file": path, "locked": false}))
    }

    fn selected(&mut self) -> Result<&mut Database, CliError> {
        let id = self.selected.as_ref().ok_or(CliError::NoDatabase)?;
        let database = self
            .databases
            .get_mut(id)
            .ok_or(CliError::UnknownDatabase)?;
        if database
            .worker
            .as_ref()
            .is_some_and(|worker| !worker.is_open())
            && let Some(mut worker) = database.worker.take()
        {
            database.closure = Some(worker.close_report()?);
        }
        Ok(database)
    }
    fn ensure_runtime(&mut self) -> Result<(), CliError> {
        if self.runtime.is_none() {
            #[cfg(feature = "ui-test-support")]
            if self.public_fixture_profile {
                self.runtime = Some(RuntimeHost::with_test_sessions(
                    &self.profile,
                    self.sessions.clone(),
                )?);
                return Ok(());
            }
            self.runtime = Some(RuntimeHost::with_sessions(
                &self.profile,
                self.sessions.clone(),
            )?);
        }
        Ok(())
    }

    fn request(&mut self, mut command: Command) -> Result<Value, CliError> {
        let result = (|| {
            let worker = self
                .selected()?
                .worker
                .as_mut()
                .ok_or(taypeer_runtime::RuntimeError::Closed)?;
            worker.request(&command).map_err(CliError::from)
        })();
        command.erase_input();
        result
    }

    pub fn execute(&mut self, action: Action) -> Result<Value, CliError> {
        self.sessions.activity().touch();
        let command = match action {
            Action::Settings(crate::args::SettingsCommand::AutoLock { seconds }) => {
                if let Some(seconds) = seconds {
                    let policy = taypeer_runtime::session::SessionPolicy::new(seconds)
                        .ok_or(CliError::Input)?;
                    taypeer_runtime::session::SessionSettings::save(&self.profile, policy)?;
                    self.sessions.set_policy(policy);
                }
                return Ok(json!({"idle_seconds": self.sessions.policy().idle_seconds()}));
            }
            Action::Sync(command) => return self.sync(command),
            Action::Invite(command) => return self.invite(command),
            Action::Device(command) => return self.device(command),
            Action::Attachment(command) => crate::binary_host::attachment(command, &self.input)?,
            Action::Icon(crate::binary_args::IconCommand::List) => {
                return serde_json::to_value(taypeer_core::LUCIDE_KEYS)
                    .map_err(|_| CliError::Input);
            }
            Action::Icon(command) => crate::binary_host::icon(command, &self.input)?,
            Action::Appearance(command) => crate::binary_host::appearance(command, &self.input)?,
            Action::Storage(command) => crate::binary_host::storage(command)?,
            Action::Trash(command) => crate::lifecycle_host::trash(command, &self.input)?,
            Action::Pending(command) => crate::lifecycle_host::pending(command, &self.input)?,
            Action::Db(command) => return self.database(command),
            Action::Group(command) => match command {
                GroupCommand::Tree => Command::Tree,
                GroupCommand::Move {
                    id,
                    parent,
                    position,
                    operation: op,
                } => Command::MoveGroup {
                    request: GroupMove {
                        group: GroupId::new(id),
                        parent: parent.map(GroupId::new),
                        position: crate::lifecycle_host::position(position),
                        review: None,
                        name: None,
                    },
                    operation: operation(op)?,
                },
                GroupCommand::Clone {
                    id,
                    parent,
                    name,
                    operation: op,
                } => Command::CloneGroup {
                    group: GroupId::new(id),
                    parent: parent.map(GroupId::new),
                    name,
                    operation: operation(op)?,
                },
                GroupCommand::Trash { id, operation: op } => Command::TrashObject {
                    target: ObjectId::Group(GroupId::new(id)),
                    operation: operation(op)?,
                },
                GroupCommand::List => Command::Groups,
                GroupCommand::Create {
                    name,
                    parent,
                    operation,
                } => Command::CreateGroup {
                    operation: self::operation(operation)?,
                    name,
                    parent: parent.map(GroupId::new),
                },
                GroupCommand::Rename {
                    id,
                    name,
                    operation,
                } => Command::RenameGroup {
                    operation: self::operation(operation)?,
                    id: GroupId::new(id),
                    name,
                },
            },
            Action::Entry(command) => match command {
                EntryCommand::Move {
                    id,
                    group,
                    operation: op,
                } => Command::MoveEntry {
                    entry: EntryId::new(id),
                    group: group.map(GroupId::new),
                    review: None,
                    operation: operation(op)?,
                },
                EntryCommand::Trash { id, operation: op } => Command::TrashObject {
                    target: ObjectId::Entry(EntryId::new(id)),
                    operation: operation(op)?,
                },
                EntryCommand::List { group, query } => Command::Entries {
                    group: group.map(GroupId::new),
                    query,
                },
                EntryCommand::Show { id } => Command::Entry(EntryId::new(id)),
                EntryCommand::Create {
                    group,
                    fields,
                    operation,
                } => Command::CreateEntry {
                    operation: self::operation(operation)?,
                    group: group.map(GroupId::new),
                    patch: self.input.fields(fields)?,
                },
                EntryCommand::Update {
                    id,
                    fields,
                    operation,
                } => Command::UpdateEntry {
                    operation: self::operation(operation)?,
                    id: EntryId::new(id),
                    patch: self.input.fields(fields)?,
                },
                EntryCommand::Clone {
                    id,
                    group,
                    title,
                    operation: op,
                } => Command::CloneEntry {
                    entry: EntryId::new(id),
                    group: group.map(GroupId::new),
                    title,
                    operation: operation(op)?,
                },
                EntryCommand::Reveal { id, attribute } => match attribute {
                    Some(attribute) => Command::RevealAttribute {
                        entry: EntryId::new(id),
                        attribute: AttributeId::new(attribute),
                    },
                    None => Command::RevealPassword(EntryId::new(id)),
                },
            },
            Action::Draft(command) => return self.draft(command),
            Action::History(command) => return self.history(command),
            Action::Generate(command) => return generate(command),
            Action::Search { query } => return self.search(&query),
            Action::Session | Action::Worker => return Err(CliError::SessionOnly),
            #[cfg(feature = "ui-test-support")]
            Action::PublicFixtureWorker => return Err(CliError::SessionOnly),
            Action::Exit => {
                self.close_all()?;
                return Ok(Value::Null);
            }
        };
        self.request(command)
    }

    fn search(&mut self, query: &str) -> Result<Value, CliError> {
        let mut results = Value::Array(Vec::new());
        let result = (|| {
            for (id, database) in &mut self.databases {
                if let Some(worker) = &mut database.worker
                    && worker.is_open()
                {
                    let entries = worker.request(&Command::Entries {
                        group: None,
                        query: query.to_owned(),
                    })?;
                    results
                        .as_array_mut()
                        .expect("search owns an array")
                        .push(json!({"database": id, "entries": entries}));
                }
            }
            // An earlier database may be revoked while a later database is being searched.
            for item in results.as_array().expect("search owns an array") {
                let id = item["database"].as_str().expect("database IDs are strings");
                if let Some(worker) = self.databases.get(id).and_then(|db| db.worker.as_ref())
                    && !worker.is_open()
                {
                    return Err(taypeer_runtime::RuntimeError::OperationInterrupted(
                        worker
                            .session_status()
                            .reason
                            .unwrap_or(taypeer_runtime::session::LockReason::Transport),
                    ));
                }
            }
            Ok(())
        })();
        if let Err(error) = result {
            taypeer_runtime::erase_view(&mut results);
            return Err(error.into());
        }
        Ok(results)
    }

    pub fn lock_all(&mut self) -> Result<(), CliError> {
        for database in self.databases.values() {
            if let Some(worker) = &database.worker {
                worker.invalidate(taypeer_runtime::session::LockReason::Manual);
            }
        }
        let mut result = Ok(());
        for database in self.databases.values_mut() {
            if let Some(mut worker) = database.worker.take() {
                let closed = worker
                    .close_report()
                    .map_err(CliError::from)
                    .and_then(|outcome| {
                        database.closure = Some(outcome);
                        if let Some(error) = outcome.error {
                            return Err(error.into());
                        }
                        if outcome.draft != taypeer_runtime::session::DraftDisposition::Preserved {
                            return Err(taypeer_runtime::RuntimeError::OperationInterrupted(
                                outcome.reason,
                            )
                            .into());
                        }
                        Ok(())
                    });
                if result.is_ok() {
                    result = closed;
                }
            }
        }
        result
    }

    pub fn collect_closed(
        &mut self,
    ) -> Result<Vec<taypeer_runtime::session::SessionStatus>, CliError> {
        let mut outcomes = Vec::new();
        for database in self.databases.values_mut() {
            if database.worker.as_ref().is_some_and(|worker| {
                worker.session_status().phase == taypeer_runtime::session::SessionPhase::Closed
            }) && let Some(mut worker) = database.worker.take()
            {
                database.closure = Some(worker.close_report()?);
                outcomes.push(worker.session_status());
            }
        }
        Ok(outcomes)
    }

    pub fn close_all(&mut self) -> Result<(), CliError> {
        let result = self.lock_all();
        if let Some(runtime) = &self.runtime {
            for id in self.databases.keys() {
                runtime.close(&taypeer_core::DatabaseId::new(id))?;
            }
        }
        self.databases.clear();
        self.selected = None;
        result
    }
}

pub(crate) fn operation(id: Option<String>) -> Result<OperationId, CliError> {
    match id {
        Some(id) if !id.is_empty() => Ok(OperationId::new(id)),
        Some(_) => Err(CliError::Input),
        None => taypeer_services::new_operation_id().map_err(|e| CliError::Runtime(e.into())),
    }
}

fn generate(command: GenerateCommand) -> Result<Value, CliError> {
    use taypeer_services::generator::{self, PasswordOptions};
    let result = match command {
        GenerateCommand::Password {
            length,
            no_uppercase,
            no_lowercase,
            no_digits,
            no_punctuation,
            exclude_similar,
            exclude,
        } => generator::password(&PasswordOptions {
            length,
            uppercase: !no_uppercase,
            lowercase: !no_lowercase,
            digits: !no_digits,
            punctuation: !no_punctuation,
            exclude_similar,
            exclude,
        }),
        GenerateCommand::Phrase { words, separator } => generator::passphrase(words, &separator),
    }
    .map_err(|_| CliError::Input)?;
    Ok(
        json!({"value": result.expose(), "characters": result.expose().chars().count(), "entropy_bits": result.entropy_bits()}),
    )
}
