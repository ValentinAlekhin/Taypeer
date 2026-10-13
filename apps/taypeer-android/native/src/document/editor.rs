//! Addressed ordinary editor operations, with no host file paths or unmasked secrets.
use super::{DocumentSession, EntryEditor, TextEdit, failure, session::identifier};
use crate::AndroidError;
use taypeer_core::{Color, EntryId, FieldUpdate, GroupId, OperationId, RevisionId};
use taypeer_runtime::Command;
use taypeer_services::{BinaryEdit, BinaryRequest, BinaryTarget, IconInput};

/// Explicit color mutation; RGBA bytes are independent of the selected theme.
#[derive(uniffi::Enum)]
pub enum ColorEdit {
    /// Preserve the stored value.
    Keep,
    /// Set an exact RGBA color.
    Set {
        /// Big-endian RGBA bytes.
        rgba: u32,
    },
    /// Follow the application theme.
    Clear,
}
impl ColorEdit {
    fn update(self) -> FieldUpdate<Color> {
        match self {
            Self::Keep => FieldUpdate::Keep,
            Self::Clear => FieldUpdate::Clear,
            Self::Set { rgba } => FieldUpdate::Set(Color(rgba.to_be_bytes())),
        }
    }
}
/// Selected appearance only; reading provenance never downloads an icon.
#[derive(uniffi::Record)]
pub struct AppearanceView {
    /// Bundled icon key; absent for default or stored image.
    pub lucide: Option<String>,
    /// Opaque stored image identity, absent for bundled icons.
    pub image: Option<String>,
    /// Optional RGBA foreground.
    pub foreground: Option<u32>,
    /// Optional RGBA background.
    pub background: Option<u32>,
}
/// Read-only timestamps and selected presentation metadata.
#[derive(uniffi::Record)]
pub struct EntryProperties {
    /// Stable entry identity.
    pub entry: String,
    /// Creation time in UTC milliseconds; not a merge clock.
    pub created_at: i64,
    /// Last effective content time in UTC milliseconds.
    pub modified_at: i64,
    /// Selected expiration in UTC milliseconds.
    pub expires_at: Option<i64>,
    /// Selected exact tags.
    pub tags: Vec<String>,
    /// Selected appearance.
    pub appearance: AppearanceView,
    /// Device-local latest accepted secret access in UTC milliseconds; absent before access.
    pub accessed_at: Option<i64>,
}
fn appearance(
    icon: taypeer_core::IconRef,
    foreground: Option<Color>,
    background: Option<Color>,
) -> AppearanceView {
    let (lucide, image) = match icon {
        taypeer_core::IconRef::Default => (None, None),
        taypeer_core::IconRef::Lucide(key) => (Some(String::from(key)), None),
        taypeer_core::IconRef::Image { blob, .. } => (None, Some(blob.to_string())),
    };
    AppearanceView {
        lucide,
        image,
        foreground: foreground.map(|c| u32::from_be_bytes(c.0)),
        background: background.map(|c| u32::from_be_bytes(c.0)),
    }
}
fn properties(view: taypeer_services::EntryView, accessed_at: Option<i64>) -> EntryProperties {
    EntryProperties {
        entry: view.id.to_string(),
        created_at: view.created_at,
        modified_at: view.modified_at,
        expires_at: view.expires_at,
        tags: view.tags,
        appearance: appearance(
            view.appearance.icon,
            view.appearance.foreground,
            view.appearance.background,
        ),
        accessed_at,
    }
}
impl DocumentSession {
    fn local_properties(
        &self,
        view: taypeer_services::EntryView,
    ) -> Result<EntryProperties, AndroidError> {
        let database = taypeer_core::DatabaseId::new(identifier(self.identity()?.0)?);
        let accessed_at = self
            .host
            .accessed_at(&database, &view.id)
            .map_err(crate::credentials::host_error)?;
        Ok(properties(view, accessed_at))
    }
}
#[uniffi::export]
impl DocumentSession {
    /// Read selected saved or historical attributes with protected values masked by Rust.
    pub fn entry_attributes(
        &self,
        entry: String,
        revision: Option<String>,
    ) -> Result<Vec<super::AttributeRow>, AndroidError> {
        let entry = EntryId::new(identifier(entry)?);
        let command = match revision {
            Some(revision) => Command::Revision {
                entry,
                revision: RevisionId::new(identifier(revision)?),
            },
            None => Command::Entry(entry),
        };
        let view: taypeer_services::EntryView = self.command(command)?;
        Ok(view
            .attributes
            .into_iter()
            .map(|a| super::AttributeRow {
                id: Some(a.id.to_string()),
                name: a.name,
                value: a.value,
                protected: a.protected,
            })
            .collect())
    }
    /// Exact tag collection; validation remains in the shared service.
    pub fn patch_tags(&self, tags: Vec<String>) -> Result<EntryEditor, AndroidError> {
        if tags.len() > 4096 || tags.iter().map(String::len).sum::<usize>() > 256 * 1024 {
            return Err(AndroidError::InvalidOptions);
        }
        self.command::<serde_json::Value>(Command::PatchDraft(taypeer_services::EntryPatch {
            tags: FieldUpdate::Set(tags),
            ..Default::default()
        }))?;
        self.editor()
    }
    /// Read selected current timestamps without opening protected values.
    pub fn entry_properties(&self, entry: String) -> Result<EntryProperties, AndroidError> {
        let view: taypeer_services::EntryView =
            self.command(Command::Entry(EntryId::new(identifier(entry)?)))?;
        self.local_properties(view)
    }
    /// Read one accessible saved revision's timestamps, tags and appearance.
    /// The existing worker validates the entry/revision and session access;
    /// this projection neither reveals protected values nor writes history.
    pub fn revision_properties(
        &self,
        entry: String,
        revision: String,
    ) -> Result<EntryProperties, AndroidError> {
        let view: taypeer_services::EntryView = self.command(Command::Revision {
            entry: EntryId::new(identifier(entry)?),
            revision: RevisionId::new(identifier(revision)?),
        })?;
        self.local_properties(view)
    }
    /// Read the active draft's selected appearance.
    pub fn form_appearance(&self) -> Result<AppearanceView, AndroidError> {
        let view: taypeer_services::BinaryView =
            self.command(Command::BinaryView(BinaryTarget::Draft))?;
        let icon = view.icons.into_iter().next().ok_or(AndroidError::Runtime)?;
        Ok(appearance(
            icon,
            view.foreground.into_iter().next().flatten(),
            view.background.into_iter().next().flatten(),
        ))
    }
    /// Change one bundled icon in the shared active form. The same operation is retryable.
    pub fn set_icon(
        &self,
        key: Option<String>,
        operation: String,
    ) -> Result<EntryEditor, AndroidError> {
        let icon = match key {
            Some(key) => {
                IconInput::Lucide(key.try_into().map_err(|_| AndroidError::InvalidOptions)?)
            }
            None => IconInput::Default,
        };
        self.command::<serde_json::Value>(Command::EditBinary {
            request: BinaryRequest {
                target: BinaryTarget::Draft,
                edit: BinaryEdit::Icon(icon),
                review: None,
            },
            operation: OperationId::new(identifier(operation)?),
        })?;
        self.editor()
    }
    /// Independently set/clear colors without overwriting the icon or entry text.
    pub fn set_colors(
        &self,
        foreground: ColorEdit,
        background: ColorEdit,
        operation: String,
    ) -> Result<EntryEditor, AndroidError> {
        self.command::<serde_json::Value>(Command::EditBinary {
            request: BinaryRequest {
                target: BinaryTarget::Draft,
                edit: BinaryEdit::Appearance {
                    foreground: foreground.update(),
                    background: background.update(),
                },
                review: None,
            },
            operation: OperationId::new(identifier(operation)?),
        })?;
        self.editor()
    }
    /// Restore a reviewed accessible entry version as a new confirmed version.
    /// The UI resolves unsaved changes first and retains this explicit confirmation.
    pub fn restore_revision(
        &self,
        entry: String,
        revision: String,
        group: String,
        operation: String,
    ) -> Result<String, AndroidError> {
        let value: serde_json::Value = self.command(Command::RestoreRevision {
            entry: EntryId::new(identifier(entry)?),
            revision: RevisionId::new(identifier(revision)?),
            group: GroupId::new(identifier(group)?),
            operation: OperationId::new(identifier(operation)?),
        })?;
        serde_json::from_value::<EntryId>(value)
            .map(|id| id.to_string())
            .map_err(failure)
    }
    /// Set a group's icon through its causal metadata form.
    pub fn patch_group_icon(
        &self,
        draft: String,
        key: TextEdit,
    ) -> Result<super::MetadataEditor, AndroidError> {
        let icon = match key {
            TextEdit::Keep => FieldUpdate::Keep,
            TextEdit::Clear => FieldUpdate::Clear,
            TextEdit::Set { value } => FieldUpdate::Set(taypeer_core::IconRef::Lucide(
                value.try_into().map_err(|_| AndroidError::InvalidOptions)?,
            )),
        };
        self.command::<serde_json::Value>(Command::PatchGroupDraft {
            draft: taypeer_core::DraftId::new(identifier(draft.clone())?),
            patch: taypeer_core::GroupMetadataPatch {
                icon,
                name: FieldUpdate::Keep,
                description: FieldUpdate::Keep,
                parent: FieldUpdate::Keep,
            },
        })?;
        self.metadata(draft)
    }
}
/// Bundled icon keys reused by both clients; no second SVG catalog.
#[uniffi::export]
pub fn bundled_icon_keys() -> Vec<String> {
    taypeer_core::LUCIDE_KEYS
        .iter()
        .map(|key| (*key).to_owned())
        .collect()
}
