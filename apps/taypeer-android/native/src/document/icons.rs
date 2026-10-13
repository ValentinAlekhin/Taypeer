//! Explicit image acquisition and scoped previews; document rules remain in services.
#[cfg(feature = "android-test-support")]
pub(super) mod diagnostics;
pub(super) mod memory;
mod render;
#[cfg(feature = "android-test-support")]
pub use diagnostics::IconPreviewStage;

use super::{DocumentSession, LocalForm, SelectedInput, selected::InputCapability, session};
use crate::AndroidError;
use std::sync::Arc;
use taypeer_core::{
    BlobId, DraftId, EntryId, GroupId, IconRef, IconSource, OperationId, RevisionId,
};
use taypeer_runtime::Command;
use taypeer_services::{BinaryTarget, DraftIdentity};
use zeroize::Zeroize;

/// Content-free failures of the shared image acquisition policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum IconFailure {
    /// URL must be HTTP(S) and must not contain credentials.
    InvalidUrl,
    /// A public destination selected an internal address.
    NetworkBoundary,
    /// A network or certificate check failed.
    Network,
    /// The aggregate acquisition deadline elapsed.
    Timeout,
    /// The image exceeds the shared byte or resource limit.
    TooLarge,
    /// Image content is invalid or unsupported.
    InvalidImage,
    /// The explicit selected input could not be read.
    File,
}
pub(super) fn icon_error(error: taypeer_services::icons::IconError) -> AndroidError {
    use taypeer_services::icons::IconError;
    let failure = match error {
        IconError::InvalidUrl => IconFailure::InvalidUrl,
        IconError::NetworkBoundary => IconFailure::NetworkBoundary,
        IconError::Network => IconFailure::Network,
        IconError::Timeout => IconFailure::Timeout,
        IconError::TooLarge => IconFailure::TooLarge,
        IconError::InvalidImage => IconFailure::InvalidImage,
        IconError::File => IconFailure::File,
    };
    AndroidError::Icon { failure }
}

/// Exact explicit request; provenance is encrypted and never logged.
#[derive(uniffi::Enum)]
pub enum IconDownload {
    /// Acquire an original image from this URL.
    ImageUrl {
        /// Exact URL submitted by the user.
        url: String,
    },
    /// Discover a site's favicon only after the explicit action.
    Favicon {
        /// Exact page URL submitted by the user.
        url: String,
    },
}
impl IconDownload {
    fn source(self) -> IconSource {
        match self {
            Self::ImageUrl { url } => IconSource::Url(url),
            Self::Favicon { url } => IconSource::Favicon(url),
        }
    }
}
impl std::fmt::Debug for IconDownload {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("IconDownload([REDACTED])")
    }
}

/// Explicit visible scope, never a bare blob authority.
#[derive(uniffi::Enum)]
pub enum IconScope {
    /// Selected current entry icon.
    Entry {
        /// Entry identity.
        entry: String,
    },
    /// Selected current group icon.
    Group {
        /// Group identity.
        group: String,
    },
    /// Exact retained local form, including an unfinished group.
    Form {
        /// Local form identity.
        draft: String,
    },
    /// Original local favicon candidate held by this exact form.
    FormFavicon {
        /// Local form identity.
        draft: String,
        /// Candidate identity returned by stored_favicon for this form.
        identity: String,
    },
    /// Selected accessible historical entry icon.
    Revision {
        /// Entry identity.
        entry: String,
        /// Revision identity.
        revision: String,
    },
    /// Selected icon of an accessible original group confirmation.
    GroupRevision {
        /// Group identity.
        group: String,
        /// Exact saved confirmation.
        revision: String,
    },
}
impl IconScope {
    fn target(self) -> Result<BinaryTarget, AndroidError> {
        Ok(match self {
            Self::Entry { entry } => BinaryTarget::Entry(EntryId::new(session::identifier(entry)?)),
            Self::Group { group } => BinaryTarget::Group(GroupId::new(session::identifier(group)?)),
            Self::Form { draft } => BinaryTarget::Form(DraftId::new(session::identifier(draft)?)),
            Self::FormFavicon { draft, identity } => BinaryTarget::FormFavicon {
                draft: DraftId::new(session::identifier(draft)?),
                identity: BlobId::new(session::identifier(identity)?),
            },
            Self::Revision { entry, revision } => BinaryTarget::Revision {
                entry: EntryId::new(session::identifier(entry)?),
                revision: RevisionId::new(session::identifier(revision)?),
            },
            Self::GroupRevision { group, revision } => BinaryTarget::GroupRevision {
                group: GroupId::new(session::identifier(group)?),
                revision: RevisionId::new(session::identifier(revision)?),
            },
        })
    }
}

/// One original local favicon candidate within an exact form; no URL or bytes.
#[derive(uniffi::Record)]
pub struct StoredFaviconView {
    /// Opaque original content identity, not authority outside its form.
    pub identity: String,
    /// Whether original content is locally available for selection and preview.
    pub available: bool,
}

/// Available local image origin, excluding its encrypted URL or local path.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum IconOrigin {
    /// Explicit selected file.
    File,
    /// Explicit original-image URL.
    ImageUrl,
    /// Explicit favicon acquisition.
    Favicon,
}
impl From<&IconSource> for IconOrigin {
    fn from(source: &IconSource) -> Self {
        match source {
            IconSource::File => Self::File,
            IconSource::Url(_) => Self::ImageUrl,
            IconSource::Favicon(_) => Self::Favicon,
        }
    }
}

/// Ephemeral local PNG thumbnail. Clear JVM image/cache copies on lock and route change.
#[derive(uniffi::Record)]
pub struct RenderedIcon {
    /// Opaque immutable content identity; cache it only within database/generation/scope.
    pub identity: String,
    /// Rendered width, at most 256 pixels.
    pub width: u32,
    /// Rendered height, at most 256 pixels.
    pub height: u32,
    /// Bounded PNG thumbnail, never an original image or command JSON payload.
    pub png: Vec<u8>,
    /// Origin kind without its source URL.
    pub origin: IconOrigin,
}
impl std::fmt::Debug for RenderedIcon {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("RenderedIcon([REDACTED])")
    }
}

impl DocumentSession {
    fn icon_receipt(
        &self,
        draft: &DraftId,
        source: &IconSource,
        operation: &OperationId,
    ) -> Result<Option<DraftIdentity>, AndroidError> {
        self.command(Command::IconImportReceipt {
            draft: draft.clone(),
            source: source.clone(),
            operation: operation.clone(),
        })
    }
    fn imported_icon_form(&self, identity: DraftIdentity) -> Result<LocalForm, AndroidError> {
        Ok(self
            .forms()?
            .into_iter()
            .find(|form| form.draft == identity.draft.as_str())
            .unwrap_or_else(|| session::form(identity, false)))
    }
    fn preview_icon(
        &self,
        target: &BinaryTarget,
        binary: &taypeer_services::BinaryView,
    ) -> Result<Option<IconRef>, AndroidError> {
        if let BinaryTarget::Revision { entry, revision } = target {
            // BinaryView intentionally retains historical alternatives. The
            // inspector's common projection determines what is selected today.
            let view: taypeer_services::EntryView = self.command(Command::Revision {
                entry: entry.clone(),
                revision: revision.clone(),
            })?;
            return Ok(Some(view.appearance.icon));
        }
        Ok(match binary.icons.as_slice() {
            [icon] => Some(icon.clone()),
            _ => None,
        })
    }
    fn stage_icon(
        &self,
        draft: DraftId,
        source: IconSource,
        operation: OperationId,
        input: InputCapability,
    ) -> Result<LocalForm, AndroidError> {
        self.identity()?;
        // No worker mutation has begun while the selected provider reports its size.
        let length = input.length().map_err(|_| AndroidError::Icon {
            failure: IconFailure::File,
        })?;
        let selected = input.register(&self.selected, length)?;
        let identity = self.command(Command::ImportSelectedIcon {
            draft,
            input: selected.id,
            length,
            source,
            operation,
        })?;
        self.imported_icon_form(identity)
    }
}

#[uniffi::export]
impl DocumentSession {
    /// Inspect the original favicon retained by the exact form without any network request.
    pub fn stored_favicon(&self, draft: String) -> Result<Option<StoredFaviconView>, AndroidError> {
        let view: Option<taypeer_services::StoredFavicon> = self.command(
            Command::StoredFavicon(DraftId::new(session::identifier(draft)?)),
        )?;
        Ok(view.map(|view| StoredFaviconView {
            identity: view.identity.to_string(),
            available: view.bytes.is_some(),
        }))
    }

    /// Reapply the original local favicon only within its owning form. No download or reimport.
    pub fn select_stored_favicon(
        &self,
        draft: String,
        identity: String,
        operation: String,
    ) -> Result<LocalForm, AndroidError> {
        let identity = self.command(Command::SelectStoredFavicon {
            draft: DraftId::new(session::identifier(draft)?),
            identity: BlobId::new(session::identifier(identity)?),
            operation: OperationId::new(session::identifier(operation)?),
        })?;
        self.imported_icon_form(identity)
    }

    /// Freeze one file selection into an exact form. Receipt replay does not invoke length/read.
    /// Success confirms encrypted local staging; only the explicit shared Save commits history.
    pub fn import_icon(
        &self,
        draft: String,
        operation: String,
        input: Box<dyn SelectedInput>,
    ) -> Result<LocalForm, AndroidError> {
        let input = InputCapability::new(input);
        let draft = DraftId::new(session::identifier(draft)?);
        let operation = OperationId::new(session::identifier(operation)?);
        let source = IconSource::File;
        if let Some(identity) = self.icon_receipt(&draft, &source, &operation)? {
            return self.imported_icon_form(identity);
        }
        self.stage_icon(draft, source, operation, input)
    }

    /// Fetch only on this explicit action using shared host Rust HTTP policy.
    /// HTTP never holds the worker command mutex; lock revokes staging and late results.
    pub fn download_icon(
        &self,
        draft: String,
        source: IconDownload,
        operation: String,
    ) -> Result<LocalForm, AndroidError> {
        let draft = DraftId::new(session::identifier(draft)?);
        let operation = OperationId::new(session::identifier(operation)?);
        let source = source.source();
        if let Some(identity) = self.icon_receipt(&draft, &source, &operation)? {
            return self.imported_icon_form(identity);
        }
        self.identity()?;
        let bytes = match &source {
            IconSource::Url(url) => taypeer_services::icons::from_url(url),
            IconSource::Favicon(url) => taypeer_services::icons::favicon(url),
            IconSource::File => return Err(AndroidError::InvalidOptions),
        }
        .map_err(icon_error)?;
        self.identity()?;
        self.stage_icon(
            draft,
            source,
            operation,
            InputCapability::new(Box::new(memory::ImageInput::new(bytes))),
        )
    }

    /// Render only the selected visible local image. Reading or opening a picker never fetches.
    pub fn icon_preview(&self, scope: IconScope) -> Result<Option<RenderedIcon>, AndroidError> {
        let target = scope.target()?;
        #[cfg(feature = "android-test-support")]
        let progress = self.preview_diagnostics.begin(&target)?;
        #[cfg(not(feature = "android-test-support"))]
        let view: taypeer_services::BinaryView =
            self.command(Command::BinaryView(target.clone()))?;
        #[cfg(feature = "android-test-support")]
        let view: taypeer_services::BinaryView =
            self.command_observed(Command::BinaryView(target.clone()), super::runtime, || {
                progress.phase(IconPreviewStage::BinaryView)
            })?;
        #[cfg(feature = "android-test-support")]
        progress.phase(IconPreviewStage::Selecting);
        let Some(IconRef::Image { blob, source }) = self.preview_icon(&target, &view)? else {
            #[cfg(feature = "android-test-support")]
            progress.complete();
            return Ok(None);
        };
        let output = Arc::new(memory::ImageOutput::default());
        let selected = self.selected.output(output.clone())?;
        #[cfg(feature = "android-test-support")]
        progress.phase(IconPreviewStage::WaitingSelectedExport);
        let command = Command::ExportSelectedBinary {
            target,
            blob: blob.clone(),
            output: selected.id,
        };
        #[cfg(not(feature = "android-test-support"))]
        self.command::<serde_json::Value>(command)?;
        #[cfg(feature = "android-test-support")]
        self.command_observed::<serde_json::Value>(command, super::runtime, || {
            progress.phase(IconPreviewStage::SelectedExport)
        })?;
        self.identity()?;
        let bytes = output.take()?;
        #[cfg(feature = "android-test-support")]
        progress.phase(IconPreviewStage::Rendering);
        let mut rendered = render::thumbnail(&bytes, blob.to_string(), (&source).into())?;
        if let Err(error) = self.identity() {
            rendered.png.zeroize();
            return Err(error);
        }
        #[cfg(feature = "android-test-support")]
        progress.complete();
        Ok(Some(rendered))
    }
}

#[cfg(test)]
mod tests;
