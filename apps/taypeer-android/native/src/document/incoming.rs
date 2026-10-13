//! Content-free automatic application observations independent of worker commands.
use super::DocumentSession;

/// Latest local application outcome; ciphertext receipt is a separate stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum ApplicationPhase {
    /// No attempt has been reported in this process generation.
    Idle,
    /// Available original changes were applied and durably confirmed.
    Applied,
    /// Received ciphertext still awaits dependencies or an explicit decision.
    Pending,
    /// Application failed and must not be presented as success.
    Failed,
}
/// Safe metadata from one current unlocked process generation. No IDs, document
/// values, source details or error text cross this projection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ApplicationState {
    /// Counter for every automatic application result, including identical reports.
    pub revision: u64,
    /// Last report that durably changed data or the control baseline in this generation.
    /// Empty and failed reports retain it; zero means none has been observed.
    pub applied_revision: u64,
    /// The shared runtime's application outcome.
    pub status: ApplicationPhase,
    /// Newly accepted original changes in this attempt.
    pub applied: u64,
    /// Number of retained ciphertext packets still waiting.
    pub pending: u64,
}
impl From<taypeer_runtime::ApplicationSnapshot> for ApplicationState {
    fn from(snapshot: taypeer_runtime::ApplicationSnapshot) -> Self {
        Self {
            revision: snapshot.revision,
            applied_revision: snapshot.applied_revision,
            status: match snapshot.phase {
                taypeer_runtime::ApplicationPhase::Idle => ApplicationPhase::Idle,
                taypeer_runtime::ApplicationPhase::Applied => ApplicationPhase::Applied,
                taypeer_runtime::ApplicationPhase::Pending => ApplicationPhase::Pending,
                taypeer_runtime::ApplicationPhase::Failed => ApplicationPhase::Failed,
            },
            applied: snapshot.applied,
            pending: snapshot.pending,
        }
    }
}
#[uniffi::export]
impl DocumentSession {
    /// Poll safe local application metadata without the command mutex, blocking
    /// Binder I/O or reading a document. A closed generation or briefly contended
    /// snapshot returns None. The UI also guards subsequent queries by generation.
    pub fn application_state(&self) -> Option<ApplicationState> {
        self.application_observer.snapshot().map(Into::into)
    }
}
