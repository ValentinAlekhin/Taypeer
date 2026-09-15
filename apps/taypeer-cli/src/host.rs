use crate::{
    args::{
        Action, ConflictCommand, DatabaseCommand, DraftCommand, EntryCommand, GenerateCommand,
        GroupCommand, HistoryCommand,
    },
    input::Input,
    output::CliError,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
use taypeer_core::{AttributeId, EntryId, GroupId, OperationId, RevisionId};
use taypeer_runtime::{Command, RuntimeHost, Worker};
use taypeer_services::{GroupMove, LifecycleAction, ObjectId};
mod p2p;

struct Database {
    path: PathBuf,
    worker: Option<Worker>,
}

pub(crate) struct Host {
    executable: PathBuf,
    databases: BTreeMap<String, Database>,
    selected: Option<String>,
    pub input: Input,
    runtime: Option<RuntimeHost>,
    profile: PathBuf,
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
    pub fn new(input: Input, profile: Option<PathBuf>) -> Result<Self, CliError> {
        let profile = match profile {
            Some(path) => path,
            None => RuntimeHost::default_profile_path()?,
        };
        Ok(Self {
            executable: std::env::current_exe().map_err(|_| CliError::Io)?,
            databases: BTreeMap::new(),
            selected: None,
            input,
            runtime: None,
            profile,
        })
    }

    pub fn open(&mut self, path: &Path, name: Option<String>) -> Result<Value, CliError> {
        let password = self.input.password(name.is_some())?;
        self.ensure_runtime()?;
        let mut worker = self.runtime.as_ref().ok_or(CliError::Io)?.open(
            &self.executable,
            path,
            password.to_string(),
            name,
        )?;
        let id = worker.database_id().as_str().to_owned();
        if self.databases.contains_key(&id) {
            worker.close()?;
            return Err(CliError::AlreadyOpen);
        }
        let path = path.canonicalize().map_err(|_| CliError::Input)?;
        self.databases.insert(
            id.clone(),
            Database {
                path,
                worker: Some(worker),
            },
        );
        self.selected = Some(id.clone());
        Ok(json!({"database": id, "locked": false}))
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
            worker.close()?;
        }
        Ok(database)
    }
    fn ensure_runtime(&mut self) -> Result<(), CliError> {
        if self.runtime.is_none() {
            self.runtime = Some(RuntimeHost::new(&self.profile)?);
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
        let command = match action {
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
                GroupCommand::Resolve {
                    input,
                    operation: op,
                } => Command::MoveGroup {
                    request: self.input.document(&input)?,
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
                GroupCommand::Trash { id } => Command::PrepareLifecycle {
                    action: LifecycleAction::Trash,
                    target: ObjectId::Group(GroupId::new(id)),
                    destination: None,
                },
                GroupCommand::List => Command::Groups,
                GroupCommand::Create { name, parent } => Command::CreateGroup {
                    name,
                    parent: parent.map(GroupId::new),
                },
                GroupCommand::Rename { id, name } => Command::RenameGroup {
                    id: GroupId::new(id),
                    name,
                },
            },
            Action::Entry(command) => match command {
                EntryCommand::Move {
                    id,
                    group,
                    review,
                    operation: op,
                } => Command::MoveEntry {
                    entry: EntryId::new(id),
                    group: GroupId::new(group),
                    review: review.map(|path| self.input.document(&path)).transpose()?,
                    operation: operation(op)?,
                },
                EntryCommand::Trash { id } => Command::PrepareLifecycle {
                    action: LifecycleAction::Trash,
                    target: ObjectId::Entry(EntryId::new(id)),
                    destination: None,
                },
                EntryCommand::List { group, query } => Command::Entries {
                    group: group.map(GroupId::new),
                    query,
                },
                EntryCommand::Show { id } => Command::Entry(EntryId::new(id)),
                EntryCommand::Create { group, fields } => Command::CreateEntry {
                    group: GroupId::new(group),
                    patch: self.input.fields(fields)?,
                },
                EntryCommand::Update { id, fields } => Command::UpdateEntry {
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
                    group: GroupId::new(group),
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
            Action::Draft(command) => match command {
                DraftCommand::Create { group } => Command::BeginCreate(GroupId::new(group)),
                DraftCommand::Edit { id } => Command::BeginEdit(EntryId::new(id)),
                DraftCommand::Update { fields } => Command::PatchDraft(self.input.fields(fields)?),
                DraftCommand::Status => Command::DraftStatus,
                DraftCommand::Save => Command::SaveDraft,
                DraftCommand::Restore => Command::RestoreDraft,
                DraftCommand::Discard => Command::DiscardDraft,
            },
            Action::History(command) => match command {
                HistoryCommand::List { entry } => Command::History(EntryId::new(entry)),
                HistoryCommand::Show { entry, revision } => Command::Revision {
                    entry: EntryId::new(entry),
                    revision: RevisionId::new(revision),
                },
                HistoryCommand::Restore {
                    entry,
                    revision,
                    group,
                    operation: op,
                } => Command::RestoreRevision {
                    entry: EntryId::new(entry),
                    revision: RevisionId::new(revision),
                    group: GroupId::new(group),
                    operation: operation(op)?,
                },
                HistoryCommand::Purge {
                    entry,
                    revision,
                    yes: _,
                    operation: op,
                } => Command::PurgeHistory {
                    entry: EntryId::new(entry),
                    revisions: revision.into_iter().map(RevisionId::new).collect(),
                    operation: operation(op)?,
                },
            },
            Action::Conflict(command) => match command {
                ConflictCommand::Generation {
                    input,
                    operation: op,
                } => crate::lifecycle_host::generation(&input, &self.input, operation(op)?)?,
                ConflictCommand::Show { entry } => Command::Conflicts(EntryId::new(entry)),
                ConflictCommand::Reveal { input } => {
                    let request: ConflictRevealInput = self.input.document(&input)?;
                    Command::RevealConflict {
                        entry: request.entry,
                        field: request.field,
                        origins: request.origins,
                    }
                }
                ConflictCommand::Resolve {
                    input,
                    operation: op,
                } => {
                    let request: ResolutionInput = self.input.document(&input)?;
                    Command::ResolveConflicts {
                        context: request.context,
                        fields: request.fields,
                        operation: operation(op)?,
                    }
                }
            },
            Action::Generate(command) => return generate(command),
            Action::Search { query } => {
                let mut results = Vec::new();
                for (id, database) in &mut self.databases {
                    if let Some(worker) = &mut database.worker {
                        let entries = worker.request(&Command::Entries {
                            group: None,
                            query: query.clone(),
                        })?;
                        results.push(json!({"database": id, "entries": entries}));
                    }
                }
                return Ok(Value::Array(results));
            }
            Action::Session | Action::Worker => return Err(CliError::SessionOnly),
            Action::Exit => {
                self.close_all()?;
                return Ok(Value::Null);
            }
        };
        self.request(command)
    }

    fn database(&mut self, command: DatabaseCommand) -> Result<Value, CliError> {
        match command {
            DatabaseCommand::Compatibility => {
                let locked = self.selected()?.worker.is_none();
                let id = taypeer_core::DatabaseId::new(self.selected.as_ref().ok_or(CliError::NoDatabase)?.clone());
                let report = self.runtime.as_ref().ok_or(CliError::Io)?.compatibility(&id)?;
                Ok(json!({"database": id, "locked": locked, "admitted": report.admitted, "format": report.format}))
            }
            DatabaseCommand::Create { path, name } => self.open(&path, Some(name)),
            DatabaseCommand::Open { path } => self.open(&path, None),
            DatabaseCommand::List => Ok(Value::Array(self.databases.iter().map(|(id, db)| {
                json!({"database": id, "file": db.path, "locked": db.worker.as_ref().is_none_or(|worker| !worker.is_open()), "selected": self.selected.as_ref() == Some(id)})
            }).collect())),
            DatabaseCommand::Use { id } => {
                if !self.databases.contains_key(&id) { return Err(CliError::UnknownDatabase); }
                self.selected = Some(id);
                Ok(Value::Null)
            }
            DatabaseCommand::Lock => {
                if let Some(mut worker) = self.selected()?.worker.take() { worker.close()?; }
                Ok(Value::Null)
            }
            DatabaseCommand::Unlock => {
                if self.selected()?.worker.is_some() { return Err(CliError::AlreadyOpen); }
                let path = self.selected()?.path.clone();
                let password = self.input.password(false)?;
                let mut worker = self.runtime.as_ref().ok_or(CliError::Io)?.open(&self.executable, &path, password.to_string(), None)?;
                if self.selected.as_deref() != Some(worker.database_id().as_str()) {
                    worker.close()?;
                    return Err(CliError::UnknownDatabase);
                }
                self.selected()?.worker = Some(worker);
                Ok(Value::Null)
            }
            DatabaseCommand::Close => {
                let id = self.selected.take().ok_or(CliError::NoDatabase)?;
                let mut database = self.databases.remove(&id).ok_or(CliError::UnknownDatabase)?;
                self.selected = self.databases.keys().next().cloned();
                if let Some(worker) = &mut database.worker { worker.close()?; }
                self.runtime.as_ref().ok_or(CliError::Io)?.close(&taypeer_core::DatabaseId::new(id))?;
                Ok(Value::Null)
            }
        }
    }

    pub fn lock_all(&mut self) -> Result<(), CliError> {
        let mut result = Ok(());
        for database in self.databases.values_mut() {
            if let Some(mut worker) = database.worker.take() {
                let closed = worker.close().map_err(CliError::from);
                if result.is_ok() {
                    result = closed;
                }
            }
        }
        result
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

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ResolutionInput {
    context: taypeer_services::ConflictContext,
    fields: Vec<taypeer_services::Resolution>,
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

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ConflictRevealInput {
    entry: EntryId,
    field: taypeer_core::EntryField,
    origins: Vec<String>,
}
