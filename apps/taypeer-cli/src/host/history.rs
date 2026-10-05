//! Original alternatives are inspected in history; services selects ordinary values.

use super::*;
use crate::history_args::{DatabaseHistoryCommand, HistoryCommand, MetadataHistoryCommand};
use taypeer_core::RevisionId;

impl Host {
    pub(super) fn history(&mut self, command: HistoryCommand) -> Result<Value, CliError> {
        let request = match command {
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
                group: group.map(GroupId::new),
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
            HistoryCommand::Reveal {
                entry,
                revision,
                attribute,
            } => Command::RevealRevision {
                entry: EntryId::new(entry),
                revision: RevisionId::new(revision),
                attribute: attribute.map(AttributeId::new),
            },
            HistoryCommand::Alternatives { entry } => Command::Conflicts(EntryId::new(entry)),
            HistoryCommand::RevealAlternative { input } => {
                let request: AlternativeReveal = self.input.document(&input)?;
                Command::RevealConflict {
                    entry: request.entry,
                    field: request.field,
                    origins: request.origins,
                }
            }
            HistoryCommand::Group(command) => match command {
                MetadataHistoryCommand::List { id } => Command::GroupHistory(GroupId::new(id)),
                MetadataHistoryCommand::Purge {
                    id,
                    revision,
                    yes: _,
                    operation: op,
                } => Command::PurgeGroupHistory {
                    group: GroupId::new(id),
                    revisions: revision.into_iter().map(RevisionId::new).collect(),
                    operation: operation(op)?,
                },
            },
            HistoryCommand::Database(command) => match command {
                DatabaseHistoryCommand::List => Command::DatabaseHistory,
                DatabaseHistoryCommand::Purge {
                    revision,
                    yes: _,
                    operation: op,
                } => Command::PurgeDatabaseHistory {
                    revisions: revision.into_iter().map(RevisionId::new).collect(),
                    operation: operation(op)?,
                },
            },
        };
        self.request(request)
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct AlternativeReveal {
    entry: EntryId,
    field: taypeer_core::EntryField,
    origins: Vec<String>,
}
