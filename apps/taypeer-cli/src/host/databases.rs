//! Catalog and worker orchestration; path and publication rules belong to RuntimeHost.

use super::*;
use crate::args::DatabaseCommand;
use taypeer_core::{DatabaseId, DatabasePolicy};
use taypeer_services::CreateDatabase;

impl Host {
    pub(super) fn database(&mut self, command: DatabaseCommand) -> Result<Value, CliError> {
        match command {
            DatabaseCommand::Compatibility => {
                let locked = self.selected()?.worker.is_none();
                let id =
                    DatabaseId::new(self.selected.as_ref().ok_or(CliError::NoDatabase)?.clone());
                let report = self
                    .runtime
                    .as_ref()
                    .ok_or(CliError::Io)?
                    .compatibility(&id)?;
                Ok(
                    json!({"database": id, "locked": locked, "admitted": report.admitted, "format": report.format}),
                )
            }
            DatabaseCommand::Create {
                name,
                operation: op,
            } => {
                let op = operation(op)?;
                let password = self.input.password(true)?;
                self.ensure_runtime()?;
                let worker = self.runtime.as_ref().ok_or(CliError::Io)?.create_internal(
                    &self.executable,
                    &op,
                    password.to_string(),
                    CreateDatabase {
                        name,
                        description: None,
                        policy: DatabasePolicy::default(),
                    },
                )?;
                self.remember_open(worker)
            }
            DatabaseCommand::Open { path } => self.open(&path),
            DatabaseCommand::Info => self.request(Command::DatabaseInfo),
            DatabaseCommand::List => {
                let copies = RuntimeHost::read_working_copies(&self.profile)?;
                Ok(Value::Array(copies.into_iter().map(|copy| {
                    let id = copy.database.as_str();
                    let db = self.databases.get(id);
                    let worker = db.and_then(|db| db.worker.as_ref());
                    json!({"database": copy.database, "file": copy.path, "locked": worker.is_none_or(|worker| !worker.is_open()), "selected": self.selected.as_deref() == Some(id), "session": worker.map(Worker::session_status), "last_lock": db.and_then(|db| db.closure)})
                }).collect()))
            }
            DatabaseCommand::Use { id } => {
                if !self.databases.contains_key(&id) {
                    let copy = RuntimeHost::read_working_copies(&self.profile)?
                        .into_iter()
                        .find(|copy| copy.database.as_str() == id)
                        .ok_or(CliError::UnknownDatabase)?;
                    self.databases.insert(
                        id.clone(),
                        Database {
                            path: copy.path,
                            worker: None,
                            closure: None,
                        },
                    );
                }
                self.selected = Some(id);
                Ok(Value::Null)
            }
            DatabaseCommand::Lock => {
                let database = self.selected()?;
                if let Some(mut worker) = database.worker.take() {
                    database.closure = Some(worker.close_report()?);
                }
                Ok(json!(database.closure))
            }
            DatabaseCommand::Unlock => {
                if self.selected()?.worker.is_some() {
                    return Err(CliError::AlreadyOpen);
                }
                let path = self.selected()?.path.clone();
                let password = self.input.password(false)?;
                self.ensure_runtime()?;
                let mut worker = self.runtime.as_ref().ok_or(CliError::Io)?.open(
                    &self.executable,
                    &path,
                    password.to_string(),
                    None,
                )?;
                if self.selected.as_deref() != Some(worker.database_id().as_str()) {
                    worker.close()?;
                    return Err(CliError::UnknownDatabase);
                }
                let database = self.selected()?;
                database.worker = Some(worker);
                database.closure = None;
                Ok(Value::Null)
            }
            DatabaseCommand::Close => {
                let id = self.selected.clone().ok_or(CliError::NoDatabase)?;
                self.close_database(&id)?;
                self.databases.remove(&id);
                self.selected = self.databases.keys().next().cloned();
                Ok(Value::Null)
            }
            DatabaseCommand::Relocate { path } => {
                let id = self.selected.clone().ok_or(CliError::NoDatabase)?;
                self.close_database(&id)?;
                self.ensure_runtime()?;
                let relocated = self
                    .runtime
                    .as_ref()
                    .ok_or(CliError::Io)?
                    .relocate_working_copy(&DatabaseId::new(id.clone()), &path)?;
                self.databases
                    .get_mut(&id)
                    .ok_or(CliError::UnknownDatabase)?
                    .path = relocated.copy.path.clone();
                serde_json::to_value(relocated).map_err(|_| CliError::Io)
            }
        }
    }

    fn close_database(&mut self, id: &str) -> Result<(), CliError> {
        let db = self
            .databases
            .get_mut(id)
            .ok_or(CliError::UnknownDatabase)?;
        if let Some(mut worker) = db.worker.take() {
            worker.close()?;
        }
        if let Some(runtime) = &self.runtime {
            runtime.close(&DatabaseId::new(id))?;
        }
        Ok(())
    }
}
