//! Explicit inspection of unaccepted original sources; no source is admitted by viewing it.
use super::{
    DocumentSession, EntryRow,
    session::{entry_row, identifier},
};
use crate::AndroidError;
use taypeer_core::{EntryId, GroupId, OperationId};
use taypeer_runtime::Command;

/// Verified source identity without entry contents, keys or ciphertext packaging IDs.
#[derive(uniffi::Record)]
pub struct ReceivedSourceRow {
    /// Stable original change hash used by all following actions.
    pub change: String,
    /// Verified original author; this does not grant current network admission.
    pub author: String,
    /// Number of causal dependencies, not an ordering or completeness claim.
    pub dependencies: u32,
    /// Explicit local disposition retained across future packaging and restarts.
    pub discarded: bool,
}

#[uniffi::export]
impl DocumentSession {
    /// List verified, unaccepted originals without revealing any source content.
    pub fn received_sources(&self) -> Result<Vec<ReceivedSourceRow>, AndroidError> {
        let rows: Vec<taypeer_services::ReceivedSource> = self.command(Command::ReceivedSources)?;
        rows.into_iter()
            .map(|row| {
                Ok(ReceivedSourceRow {
                    change: row.change,
                    author: row.author.to_string(),
                    dependencies: row.dependencies.len().try_into().map_err(super::failure)?,
                    discarded: row.discarded,
                })
            })
            .collect()
    }

    /// Inspect the selected source's exact causal state. Viewing never merges it;
    /// passwords remain masked until the separate explicit reveal command.
    pub fn inspect_received(&self, change: String) -> Result<Vec<EntryRow>, AndroidError> {
        let entries: Vec<taypeer_services::EntryView> =
            self.command(Command::InspectReceived(identifier(change)?))?;
        Ok(entries.into_iter().map(entry_row).collect())
    }

    /// Reveal only one selected source password. The client clears the returned
    /// value on focus loss, route departure and lock; JVM zeroization is not promised.
    pub fn reveal_received(&self, change: String, entry: String) -> Result<String, AndroidError> {
        self.command(Command::RevealReceived {
            change: identifier(change)?,
            entry: EntryId::new(identifier(entry)?),
        })
    }

    /// Mark exactly one original source as discarded. Repetition is idempotent
    /// in the shared service; failure cannot be presented as a durable discard.
    pub fn discard_received(&self, change: String) -> Result<(), AndroidError> {
        self.command::<serde_json::Value>(Command::DiscardReceived(identifier(change)?))
            .map(|_| ())
    }

    /// Extract one selected entry into an explicit active group with a new identity.
    /// Reuse the exact operation, source, entry and destination on uncertain retry.
    /// Original history and network admission are not inherited by this confirmation.
    pub fn extract_received(
        &self,
        change: String,
        entry: String,
        group: String,
        operation: String,
    ) -> Result<String, AndroidError> {
        let created: EntryId = self.command(Command::ExtractReceived {
            change: identifier(change)?,
            entry: EntryId::new(identifier(entry)?),
            group: GroupId::new(identifier(group)?),
            operation: OperationId::new(identifier(operation)?),
        })?;
        Ok(created.to_string())
    }
}
