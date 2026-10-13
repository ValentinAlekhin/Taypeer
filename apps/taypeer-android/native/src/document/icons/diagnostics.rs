//! DEBUG-only bounded content-free phases; never reads document values or key material.
use super::super::{DocumentSession, failure};
use crate::AndroidError;
use std::{
    collections::{BTreeMap, VecDeque},
    sync::Mutex,
};
use taypeer_services::BinaryTarget;

/// Last phase of one current-generation local preview, for Android regression diagnosis.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum IconPreviewStage {
    /// No request for this scope exists in the bounded diagnostic window.
    NotRequested,
    /// Waiting for the host command mutex before requesting a binary projection.
    WaitingBinaryView,
    /// Waiting for the real worker's selected binary projection.
    BinaryView,
    /// Selecting one accessible local icon.
    Selecting,
    /// Waiting for the host command mutex before selected export.
    WaitingSelectedExport,
    /// Waiting for checked private stream export and its explicit completion.
    SelectedExport,
    /// Rendering the bounded original as a thumbnail.
    Rendering,
    /// Complete value passed its final session fence.
    Ready,
    /// Native operation exited before completing a value.
    Failed,
}
#[derive(Default)]
struct State {
    next: u64,
    rows: BTreeMap<String, (u64, IconPreviewStage)>,
    order: VecDeque<String>,
}
#[derive(Default)]
pub(in crate::document) struct PreviewDiagnostics(Mutex<State>);
impl PreviewDiagnostics {
    pub(super) fn begin(&self, target: &BinaryTarget) -> Result<PreviewGuard<'_>, AndroidError> {
        let key = serde_json::to_string(target).map_err(failure)?;
        let mut state = self.0.lock().map_err(failure)?;
        state.next = state.next.checked_add(1).ok_or(AndroidError::Runtime)?;
        let request = state.next;
        state.order.retain(|old| old != &key);
        state.order.push_back(key.clone());
        state
            .rows
            .insert(key.clone(), (request, IconPreviewStage::WaitingBinaryView));
        while state.order.len() > 64 {
            if let Some(old) = state.order.pop_front() {
                state.rows.remove(&old);
            }
        }
        Ok(PreviewGuard {
            owner: self,
            key,
            request,
            complete: false,
        })
    }
    fn read(&self, target: &BinaryTarget) -> Result<IconPreviewStage, AndroidError> {
        let key = serde_json::to_string(target).map_err(failure)?;
        Ok(self
            .0
            .lock()
            .map_err(failure)?
            .rows
            .get(&key)
            .map(|(_, stage)| *stage)
            .unwrap_or(IconPreviewStage::NotRequested))
    }
    pub(in crate::document) fn clear(&self) {
        if let Ok(mut state) = self.0.lock() {
            state.rows.clear();
            state.order.clear();
        }
    }
}
pub(super) struct PreviewGuard<'a> {
    owner: &'a PreviewDiagnostics,
    key: String,
    request: u64,
    complete: bool,
}
impl PreviewGuard<'_> {
    pub(super) fn phase(&self, stage: IconPreviewStage) {
        if let Ok(mut state) = self.owner.0.lock()
            && let Some((request, phase)) = state.rows.get_mut(&self.key)
            && *request == self.request
        {
            *phase = stage;
        }
    }
    pub(super) fn complete(mut self) {
        self.phase(IconPreviewStage::Ready);
        self.complete = true;
    }
}
impl Drop for PreviewGuard<'_> {
    fn drop(&mut self) {
        if !self.complete {
            self.phase(IconPreviewStage::Failed);
        }
    }
}
#[uniffi::export]
impl DocumentSession {
    /// DEBUG probe reads only a numeric phase, independently of a blocked worker command.
    /// Closed generations refuse access; stored scope identifiers are bounded to 64.
    pub fn icon_preview_stage(
        &self,
        scope: super::IconScope,
    ) -> Result<IconPreviewStage, AndroidError> {
        self.identity()?;
        self.preview_diagnostics.read(&scope.target()?)
    }
}
