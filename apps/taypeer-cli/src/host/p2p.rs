use super::*;
use crate::p2p_args::{DeviceCommand, InviteCommand, SyncCommand};
use taypeer_runtime::{InvitationCode, JoinProgress};
use taypeer_sync::{EndpointAddr, RelaySetting};
use taypeer_trust::{Digest, Invitation};
use zeroize::Zeroizing;

impl Host {
    fn ensure_network(&mut self) -> Result<(), CliError> {
        self.ensure_runtime()?;
        let runtime = self.runtime.as_mut().ok_or(CliError::Io)?;
        if runtime.network_address().is_err() {
            runtime.start_network(RelaySetting::Disabled)?;
        }
        Ok(())
    }
    pub(super) fn sync(&mut self, command: SyncCommand) -> Result<Value, CliError> {
        match command {
            SyncCommand::Start {
                relay,
                relay_only,
                relay_ca,
            } => {
                self.ensure_runtime()?;
                let relay = match relay.as_deref() {
                    None | Some("off") => RelaySetting::Disabled,
                    Some("default") => RelaySetting::Default,
                    Some(url) => RelaySetting::Custom(url.parse().map_err(|_| CliError::Input)?),
                };
                let relay = if relay_only {
                    let RelaySetting::Custom(url) = relay else {
                        return Err(CliError::Input);
                    };
                    let root_der = relay_ca
                        .map(|path| {
                            use std::io::Read;
                            let mut bytes = Vec::new();
                            std::fs::File::open(path)
                                .map_err(|_| CliError::Input)?
                                .take(65537)
                                .read_to_end(&mut bytes)
                                .map_err(|_| CliError::Input)?;
                            if bytes.is_empty() || bytes.len() > 65536 {
                                return Err(CliError::Input);
                            }
                            Ok(bytes)
                        })
                        .transpose()?;
                    RelaySetting::RelayOnly { url, root_der }
                } else {
                    relay
                };
                value(
                    self.runtime
                        .as_mut()
                        .ok_or(CliError::Io)?
                        .start_network(relay)?,
                )
            }
            SyncCommand::Stop => {
                if let Some(runtime) = &mut self.runtime {
                    runtime.stop_network();
                }
                Ok(Value::Null)
            }
            SyncCommand::Status => {
                let runtime = self.runtime.as_ref().ok_or(CliError::NoDatabase)?;
                let application: BTreeMap<_, _> = self
                    .databases
                    .iter()
                    .map(|(id, database)| {
                        (
                            id,
                            database
                                .worker
                                .as_ref()
                                .and_then(|worker| worker.application_progress()),
                        )
                    })
                    .collect();
                let (address, peers) = match runtime.network_address() {
                    Ok(address) => (Some(address), runtime.network_progress()?),
                    Err(taypeer_runtime::RuntimeError::Closed) => (None, Vec::new()),
                    Err(error) => return Err(error.into()),
                };
                Ok(json!({"address": address, "peers": peers, "application": application}))
            }
            SyncCommand::Now { peer } => {
                let address: EndpointAddr = self.input.document(&peer)?;
                let database = self.selected.clone().ok_or(CliError::NoDatabase)?;
                self.ensure_network()?;
                let received = self
                    .runtime
                    .as_ref()
                    .ok_or(CliError::Io)?
                    .exchange(address, taypeer_core::DatabaseId::new(database))?;
                let application = if self.selected()?.worker.is_some() {
                    Some(self.request(Command::ApplyReceived)?)
                } else {
                    None
                };
                Ok(json!({"ciphertext": received, "application": application}))
            }
            SyncCommand::Apply => self.request(Command::ApplyReceived),
            SyncCommand::Collect => self.request(Command::CollectReceived),
            SyncCommand::Sources => self.request(Command::ReceivedSources),
            SyncCommand::Inspect { change } => self.request(Command::InspectReceived(change)),
            SyncCommand::Reveal { change, entry } => self.request(Command::RevealReceived {
                change,
                entry: taypeer_core::EntryId::new(entry),
            }),
            SyncCommand::Discard { change, yes } => {
                if !yes {
                    return Err(CliError::Input);
                }
                self.request(Command::DiscardReceived(change))
            }
            SyncCommand::Extract {
                change,
                entry,
                group,
                operation,
            } => self.request(Command::ExtractReceived {
                change,
                entry: taypeer_core::EntryId::new(entry),
                group: taypeer_core::GroupId::new(group),
                operation: taypeer_core::OperationId::new(operation),
            }),
        }
    }
    pub(super) fn invite(&mut self, command: InviteCommand) -> Result<Value, CliError> {
        match command {
            InviteCommand::Create => {
                self.ensure_network()?;
                let material = self.request(Command::CreateInvitation)?;
                let (invitation, secret): (Invitation, Zeroizing<[u8; 32]>) =
                    serde_json::from_value(material).map_err(|_| CliError::Io)?;
                value(InvitationCode {
                    invitation,
                    secret,
                    address: self
                        .runtime
                        .as_ref()
                        .ok_or(CliError::Io)?
                        .network_address()?,
                })
            }
            InviteCommand::Requests => {
                let id = self.selected.as_ref().ok_or(CliError::NoDatabase)?;
                let snapshot = self
                    .runtime
                    .as_ref()
                    .ok_or(CliError::Io)?
                    .coordinator()
                    .snapshot(&taypeer_core::DatabaseId::new(id))
                    .map_err(|_| CliError::Io)?;
                value(&snapshot.metadata().journal.invitations)
            }
            InviteCommand::Approve { request } => self.request(Command::ApproveInvitation(request)),
            InviteCommand::Reject { request } => self.request(Command::CloseInvitation {
                request,
                reject: true,
            }),
            InviteCommand::Cancel { request } => self.request(Command::CloseInvitation {
                request,
                reject: false,
            }),
            InviteCommand::Join { path, input } => {
                let code = match input {
                    Some(path) => self.input.document(&path)?,
                    None => serde_json::from_str(&self.input.secret("invitation_code")?)
                        .map_err(|_| CliError::Input)?,
                };
                self.ensure_network()?;
                value(self.runtime.as_ref().ok_or(CliError::Io)?.join(
                    &self.executable,
                    code,
                    &path,
                )?)
            }
            InviteCommand::Pending => {
                self.ensure_runtime()?;
                value(self.runtime.as_ref().ok_or(CliError::Io)?.pending_joins()?)
            }
            InviteCommand::Resume { request } => {
                self.ensure_network()?;
                let runtime = self.runtime.as_ref().ok_or(CliError::Io)?;
                let path = runtime
                    .pending_joins()?
                    .get(&request)
                    .ok_or(CliError::Input)?
                    .path
                    .clone();
                let progress = runtime.resume_join(request)?;
                if let JoinProgress::Received(database) = &progress {
                    let id = database.as_str().to_owned();
                    self.databases.insert(
                        id.clone(),
                        Database {
                            path,
                            worker: None,
                            closure: None,
                        },
                    );
                    self.selected = Some(id);
                }
                value(progress)
            }
        }
    }
    pub(super) fn device(&mut self, command: DeviceCommand) -> Result<Value, CliError> {
        match command {
            DeviceCommand::Recover {
                path,
                operation,
                input,
            } => {
                let password = self.administrative_password(input)?;
                self.request(Command::RecoverTrust {
                    path,
                    operation,
                    password,
                })
            }
            DeviceCommand::List => self.request(Command::Authority),
            DeviceCommand::Policy => self.request(Command::DatabasePolicy),
            DeviceCommand::Password { operation, input } => {
                let password = self.administrative_password(input)?;
                self.request(Command::RotatePassword {
                    operation: admin_operation(operation)?,
                    password,
                    revoke: None,
                })
            }
            DeviceCommand::Revoke {
                device,
                operation,
                input,
            } => {
                let password = self.administrative_password(input)?;
                self.request(Command::RotatePassword {
                    operation: admin_operation(operation)?,
                    password,
                    revoke: Some(device),
                })
            }
            DeviceCommand::Consent { operation } => {
                self.request(Command::ConsentManagement(operation))
            }
            DeviceCommand::Transfer { input } => {
                self.request(Command::TransferManagement(self.input.document(&input)?))
            }
            DeviceCommand::SetPolicy {
                input,
                operation,
                password_input,
            } => {
                let policy: taypeer_core::DatabasePolicy = self.input.document(&input)?;
                let current: taypeer_core::DatabasePolicy =
                    serde_json::from_value(self.request(Command::DatabasePolicy)?)
                        .map_err(|_| CliError::Io)?;
                let password = if current.kdf_target_ms() != policy.kdf_target_ms() {
                    Some(self.administrative_password(password_input)?)
                } else {
                    None
                };
                self.request(Command::SetDatabasePolicy {
                    operation: admin_operation(operation)?,
                    policy,
                    password,
                })
            }
        }
    }
    fn administrative_password(
        &self,
        input: Option<PathBuf>,
    ) -> Result<Zeroizing<Vec<u8>>, CliError> {
        let text: Zeroizing<String> = if let Some(path) = input {
            self.input.document(&path)?
        } else {
            if self.input.password_stdin {
                return Err(CliError::StdinConflict);
            }
            self.input.password(true)?
        };
        Ok(Zeroizing::new(text.as_bytes().to_vec()))
    }
}
fn value(data: impl serde::Serialize) -> Result<Value, CliError> {
    serde_json::to_value(data).map_err(|_| CliError::Io)
}
fn admin_operation(operation: Option<Digest>) -> Result<Digest, CliError> {
    match operation {
        Some(id) => Ok(id),
        None => Ok(Digest::of(
            taypeer_services::new_operation_id()
                .map_err(|e| CliError::Runtime(e.into()))?
                .as_str()
                .as_bytes(),
        )),
    }
}
