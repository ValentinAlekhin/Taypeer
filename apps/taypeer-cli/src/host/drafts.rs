//! Addressed local forms share services' identities, revisions and persistence.

use super::*;
use crate::draft_args::{DatabaseDraftCommand, DraftCommand, GroupDraftCommand};
use taypeer_core::DraftId;
use taypeer_services::{DraftIdentity, DraftRevision, DraftTarget};

impl Host {
    pub(super) fn draft(&mut self, command: DraftCommand) -> Result<Value, CliError> {
        let request = match command {
            DraftCommand::Create { group, fields } => {
                self.request(match group {
                    Some(group) => Command::BeginCreate(GroupId::new(group)),
                    None => Command::BeginCreateUngrouped,
                })?;
                self.request(Command::PatchDraft(self.input.fields(fields)?))?;
                return self.request(Command::EditorView);
            }
            DraftCommand::Edit { id, fields } => {
                self.request(Command::BeginEdit(EntryId::new(id)))?;
                self.request(Command::PatchDraft(self.input.fields(fields)?))?;
                return self.request(Command::EditorView);
            }
            DraftCommand::Update { draft, fields } => {
                if let Some(id) = draft {
                    self.request(Command::ResumeDraft(DraftId::new(id)))?;
                }
                self.request(Command::PatchDraft(self.input.fields(fields)?))?;
                return self.request(Command::EditorView);
            }
            DraftCommand::Status => Command::DraftStatus,
            DraftCommand::Save { operation: op } => Command::SaveDraft {
                operation: operation(op)?,
            },
            DraftCommand::Discard => Command::DiscardDraft,
            DraftCommand::List => Command::Drafts,
            DraftCommand::Resume { id } => {
                let identity: DraftIdentity =
                    serde_json::from_value(self.request(Command::ResumeDraft(DraftId::new(id)))?)
                        .map_err(|_| CliError::Io)?;
                return self.request(match identity.target {
                    DraftTarget::Entry(_) | DraftTarget::NewEntry { .. } => Command::EditorView,
                    _ => Command::MetadataDraft(identity.draft),
                });
            }
            DraftCommand::Delete { id } => Command::DeleteDraft(DraftId::new(id)),
            DraftCommand::Snapshot {
                id,
                revision,
                operation: op,
            } => Command::SaveDraftSnapshot {
                draft: DraftId::new(id),
                revision: DraftRevision(revision),
                operation: operation(op)?,
            },
            DraftCommand::Persist => Command::PersistDrafts,
            DraftCommand::View => Command::EditorView,
            DraftCommand::Group(command) => match command {
                GroupDraftCommand::Create { parent, input } => {
                    let view = self.request(Command::BeginCreateGroup(parent.map(GroupId::new)))?;
                    return self.patch_group_on_begin(view, input);
                }
                GroupDraftCommand::Edit { id, input } => {
                    let view = self.request(Command::BeginEditGroup(GroupId::new(id)))?;
                    return self.patch_group_on_begin(view, input);
                }
                GroupDraftCommand::Update { id, input } => {
                    return self.patch_group(DraftId::new(id), &input);
                }
            },
            DraftCommand::Database(command) => match command {
                DatabaseDraftCommand::Edit { input } => {
                    let mut view = self.request(Command::BeginEditDatabaseInfo)?;
                    if let Some(input) = input {
                        let draft = draft_id(&view)?;
                        taypeer_runtime::erase_view(&mut view);
                        return self.patch_database(draft, &input);
                    }
                    return Ok(view);
                }
                DatabaseDraftCommand::Update { id, input } => {
                    return self.patch_database(DraftId::new(id), &input);
                }
            },
            DraftCommand::Metadata { id } => Command::MetadataDraft(DraftId::new(id)),
        };
        self.request(request)
    }

    fn patch_group_on_begin(
        &mut self,
        mut view: Value,
        input: Option<PathBuf>,
    ) -> Result<Value, CliError> {
        if let Some(input) = input {
            let draft = draft_id(&view)?;
            taypeer_runtime::erase_view(&mut view);
            self.patch_group(draft, &input)
        } else {
            Ok(view)
        }
    }

    fn patch_group(&mut self, draft: DraftId, input: &Path) -> Result<Value, CliError> {
        self.request(Command::PatchGroupDraft {
            draft: draft.clone(),
            patch: self.input.document(input)?,
        })?;
        self.request(Command::MetadataDraft(draft))
    }

    fn patch_database(&mut self, draft: DraftId, input: &Path) -> Result<Value, CliError> {
        self.request(Command::PatchDatabaseDraft {
            draft: draft.clone(),
            patch: self.input.document(input)?,
        })?;
        self.request(Command::MetadataDraft(draft))
    }
}

fn draft_id(view: &Value) -> Result<DraftId, CliError> {
    view["identity"]["draft"]
        .as_str()
        .map(DraftId::new)
        .ok_or(CliError::Io)
}
