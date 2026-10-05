//! Entry and metadata forms use shared draft identities and exact input revisions.

use crate::args::Fields;
use clap::Subcommand;
use std::path::PathBuf;

#[derive(Subcommand)]
pub(crate) enum DraftCommand {
    /// Begin a new entry without saving it.
    Create {
        #[arg(long)]
        group: Option<String>,
        #[command(flatten)]
        fields: Fields,
    },
    /// Begin editing an entry without exposing its existing secrets.
    Edit {
        id: String,
        #[command(flatten)]
        fields: Fields,
    },
    /// Apply addressed fields to the current draft.
    Update {
        #[arg(long)]
        draft: Option<String>,
        #[command(flatten)]
        fields: Fields,
    },
    /// Show draft status without its contents.
    Status,
    /// Confirm the active draft.
    Save {
        #[arg(long)]
        operation: Option<String>,
    },
    /// Discard the active or interrupted draft.
    Discard,
    #[command(about = crate::output::help("help_draft_list"))]
    List,
    #[command(about = crate::output::help("help_draft_resume"))]
    Resume { id: String },
    #[command(about = crate::output::help("help_draft_delete"))]
    Delete { id: String },
    #[command(about = crate::output::help("help_draft_snapshot"))]
    Snapshot {
        id: String,
        #[arg(long)]
        revision: u64,
        #[arg(long)]
        operation: Option<String>,
    },
    #[command(about = crate::output::help("help_draft_persist"))]
    Persist,
    #[command(about = crate::output::help("help_draft_view"))]
    View,
    #[command(about = crate::output::help("help_metadata_draft"), subcommand)]
    Group(GroupDraftCommand),
    #[command(about = crate::output::help("help_metadata_draft"), subcommand)]
    Database(DatabaseDraftCommand),
    #[command(about = crate::output::help("help_metadata_draft"))]
    Metadata { id: String },
}

#[derive(Subcommand)]
pub(crate) enum GroupDraftCommand {
    Create {
        #[arg(long)]
        parent: Option<String>,
        #[arg(long)]
        input: Option<PathBuf>,
    },
    Edit {
        id: String,
        #[arg(long)]
        input: Option<PathBuf>,
    },
    Update {
        id: String,
        #[arg(long)]
        input: PathBuf,
    },
}

#[derive(Subcommand)]
pub(crate) enum DatabaseDraftCommand {
    Edit {
        #[arg(long)]
        input: Option<PathBuf>,
    },
    Update {
        id: String,
        #[arg(long)]
        input: PathBuf,
    },
}
