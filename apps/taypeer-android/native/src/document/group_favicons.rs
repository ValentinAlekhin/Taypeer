//! A finite live-session group action; host acquisition never holds the worker mutex.
use super::{DocumentSession, IconFailure, failure, icons, selected::InputCapability, session};
use crate::AndroidError;
use std::sync::{
    Arc, Mutex, Weak,
    atomic::{AtomicBool, Ordering},
};
use taypeer_core::{GroupId, OperationId};
use taypeer_runtime::Command;
use taypeer_services::{
    FaviconEntryRequest, FaviconPlanAction, GroupFaviconPlan, GroupFaviconRequest,
};

/// Per-entry result; only Applied denotes a durable saved icon and history receipt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum GroupFaviconState {
    /// Shared selection deliberately keeps this custom icon.
    Skipped,
    /// The saved entry has no ordinary URL.
    MissingUrl,
    /// Fixed request awaits its explicit batch run.
    Ready,
    /// This one request is currently being acquired or committed.
    Running,
    /// Durable receipt acknowledged this exact icon operation.
    Applied,
    /// Definite acquisition or prepublication failure; exact retry is available.
    Failed,
    /// Publication/reply could not be confirmed; reconcile before any new write.
    Uncertain,
    /// Remaining operation was revoked, without undoing confirmed siblings.
    Cancelled,
}

/// Content-free per-entry reason. No URL, HTTP response or native error text is exported.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum GroupFaviconFailure {
    /// Common image/network acquisition category.
    Image {
        /// Typed shared policy refusal.
        failure: IconFailure,
    },
    /// Shared input/context validation refused before publication.
    Rejected,
    /// A definite storage failure occurred before acknowledgement.
    Storage,
    /// A publication or transport reply is unresolved.
    Unconfirmed,
}

/// Ordinary row label and actual outcome; no acquisition URL or protected fields.
#[derive(Clone, uniffi::Record)]
pub struct GroupFaviconRow {
    /// Saved entry identity selected by Rust.
    pub entry: String,
    /// Original ordinary label for this batch's presentation.
    pub title: String,
    /// Current exact operation state.
    pub state: GroupFaviconState,
    /// Categorized failure, if present.
    pub failure: Option<GroupFaviconFailure>,
}

struct Row {
    view: GroupFaviconRow,
    request: Option<FaviconEntryRequest>,
}

/// Opaque frozen plan, usable only while its original document generation is OPEN.
/// It is not a restart journal. Cancel/lock denies late apply; saved siblings survive.
#[derive(uniffi::Object)]
pub struct GroupFaviconBatch {
    session: Weak<DocumentSession>,
    identity: (String, u64),
    rows: Mutex<Vec<Row>>,
    running: AtomicBool,
    cancelled: AtomicBool,
}

impl GroupFaviconBatch {
    fn from_plan(
        session: &Arc<DocumentSession>,
        identity: (String, u64),
        plan: GroupFaviconPlan,
    ) -> Self {
        Self {
            session: Arc::downgrade(session),
            identity,
            rows: Mutex::new(
                plan.rows
                    .into_iter()
                    .map(|row| {
                        let (state, request) = match row.action {
                            FaviconPlanAction::Skipped => (GroupFaviconState::Skipped, None),
                            FaviconPlanAction::MissingUrl => (GroupFaviconState::MissingUrl, None),
                            FaviconPlanAction::Acquire(request) => {
                                (GroupFaviconState::Ready, Some(request))
                            }
                        };
                        Row {
                            view: GroupFaviconRow {
                                entry: row.entry.to_string(),
                                title: row.title,
                                state,
                                failure: None,
                            },
                            request,
                        }
                    })
                    .collect(),
            ),
            running: AtomicBool::new(false),
            cancelled: AtomicBool::new(false),
        }
    }
    fn checked_session(&self) -> Result<Arc<DocumentSession>, AndroidError> {
        if self.cancelled.load(Ordering::Acquire) {
            return Err(AndroidError::Runtime);
        }
        let session = self.session.upgrade().ok_or(AndroidError::Runtime)?;
        if session.identity()? != self.identity {
            return Err(AndroidError::Runtime);
        }
        Ok(session)
    }
    fn publish(
        &self,
        index: usize,
        state: GroupFaviconState,
        failure: Option<GroupFaviconFailure>,
    ) -> Result<(), AndroidError> {
        let mut rows = self.rows.lock().map_err(super::failure)?;
        let row = rows.get_mut(index).ok_or(AndroidError::Runtime)?;
        row.view.state = state;
        row.view.failure = failure;
        Ok(())
    }
    fn command<T: serde::de::DeserializeOwned>(
        &self,
        session: &DocumentSession,
        mut command: Command,
    ) -> Result<T, AndroidError> {
        let mut worker = session.worker.lock().map_err(failure)?;
        // Cancel may arrive while this request waits behind another command.
        // Recheck after acquiring that queue, before admitting the request.
        self.checked_session()?;
        let result = worker.request(&command);
        command.erase_input();
        let value = result.map_err(super::runtime)?;
        self.checked_session()?;
        serde_json::from_value(value).map_err(failure)
    }
    fn apply(&self, request: FaviconEntryRequest) -> Result<(), AndroidError> {
        let session = self.checked_session()?;
        if self.command::<bool>(&session, Command::FaviconEntryReceipt(request.clone()))? {
            return Ok(());
        }
        self.checked_session()?;
        // This is the only HTTP hop. The sequential worker request above has
        // completed and released its mutex before the network begins.
        let bytes = taypeer_services::icons::favicon(&request.url).map_err(icons::icon_error)?;
        self.checked_session()?;
        let input = InputCapability::new(Box::new(icons::memory::ImageInput::new(bytes)));
        let length = input.length()?;
        let selected = input.register(&session.selected, length)?;
        self.checked_session()?;
        self.command::<serde_json::Value>(
            &session,
            Command::ImportSelectedEntryFavicon {
                request,
                input: selected.id,
                length,
            },
        )?;
        self.checked_session()?;
        Ok(())
    }
}

struct RunGuard<'a>(&'a AtomicBool);
impl Drop for RunGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
impl Drop for GroupFaviconBatch {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

fn categorize(error: AndroidError) -> (GroupFaviconState, GroupFaviconFailure) {
    match error {
        AndroidError::Icon { failure } => (
            GroupFaviconState::Failed,
            GroupFaviconFailure::Image { failure },
        ),
        AndroidError::InvalidInput | AndroidError::InvalidOptions => {
            (GroupFaviconState::Failed, GroupFaviconFailure::Rejected)
        }
        AndroidError::StorageIo => (GroupFaviconState::Failed, GroupFaviconFailure::Storage),
        _ => (
            GroupFaviconState::Uncertain,
            GroupFaviconFailure::Unconfirmed,
        ),
    }
}

#[uniffi::export]
impl GroupFaviconBatch {
    /// Read actual per-entry states without network or worker I/O.
    pub fn rows(&self) -> Result<Vec<GroupFaviconRow>, AndroidError> {
        Ok(self
            .rows
            .lock()
            .map_err(failure)?
            .iter()
            .map(|row| row.view.clone())
            .collect())
    }
    /// Acquire pending rows sequentially on an IO thread. Confirmed rows are
    /// skipped before HTTP; uncertain publication stops subsequent sibling writes.
    pub fn run(&self) -> Result<Vec<GroupFaviconRow>, AndroidError> {
        if self
            .running
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(AndroidError::InvalidOptions);
        }
        let _running = RunGuard(&self.running);
        self.checked_session()?;
        let count = self.rows.lock().map_err(failure)?.len();
        for index in 0..count {
            self.checked_session()?;
            let request = {
                let rows = self.rows.lock().map_err(failure)?;
                let row = &rows[index];
                if matches!(
                    row.view.state,
                    GroupFaviconState::Ready
                        | GroupFaviconState::Failed
                        | GroupFaviconState::Uncertain
                ) {
                    row.request.clone()
                } else {
                    None
                }
            };
            let Some(request) = request else {
                continue;
            };
            self.publish(index, GroupFaviconState::Running, None)?;
            match self.apply(request) {
                Ok(()) => self.publish(index, GroupFaviconState::Applied, None)?,
                Err(error) => {
                    if self.checked_session().is_err() {
                        self.cancel();
                        return Err(AndroidError::Runtime);
                    }
                    let (state, reason) = categorize(error);
                    self.publish(index, state, Some(reason))?;
                    if state == GroupFaviconState::Uncertain {
                        break;
                    }
                }
            }
        }
        self.rows()
    }
    /// Immediate nonblocking fence. In-flight HTTP may finish but cannot start
    /// another not-yet-admitted worker apply. An already admitted atomic commit
    /// may finish concurrently; cancellation never undoes durable receipts.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        if let Ok(mut rows) = self.rows.lock() {
            for row in &mut *rows {
                if !matches!(
                    row.view.state,
                    GroupFaviconState::Applied
                        | GroupFaviconState::Skipped
                        | GroupFaviconState::MissingUrl
                ) {
                    row.view.state = GroupFaviconState::Cancelled;
                    row.view.failure = None;
                }
            }
        }
    }
}

#[uniffi::export]
impl DocumentSession {
    /// Prepare only after an explicit user action. Shared selection is frozen;
    /// opening the UI form does not call this method or perform any HTTP.
    pub fn prepare_group_favicons(
        self: Arc<Self>,
        group: String,
        recursive: bool,
        replace: bool,
        operation: String,
    ) -> Result<Arc<GroupFaviconBatch>, AndroidError> {
        let identity = self.identity()?;
        let request = GroupFaviconRequest {
            group: GroupId::new(session::identifier(group)?),
            recursive,
            replace,
            operation: OperationId::new(session::identifier(operation)?),
        };
        let plan = self.command::<GroupFaviconPlan>(Command::PrepareGroupFavicons(request))?;
        if self.identity()? != identity {
            return Err(AndroidError::Runtime);
        }
        Ok(Arc::new(GroupFaviconBatch::from_plan(
            &self, identity, plan,
        )))
    }
}
