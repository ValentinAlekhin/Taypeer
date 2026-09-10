//! Binary command syntax; validation and effects belong to Rust services.

use clap::{Args, Subcommand};
use std::path::PathBuf;

#[derive(Args)]
pub(crate) struct TargetArgs {
    #[arg(long, help = crate::output::help("help_target_entry"))]
    pub entry: Option<String>,
    #[arg(long, help = crate::output::help("help_target_group"))]
    pub group: Option<String>,
    #[arg(long, help = crate::output::help("help_target_draft"))]
    pub draft: bool,
    #[arg(long, help = crate::output::help("help_target_source"))]
    pub source: Option<String>,
    #[arg(long, requires = "entry")]
    pub revision: Option<String>,
    #[arg(long, conflicts_with = "revision")]
    pub generation: Option<String>,
}
#[derive(Args)]
pub(crate) struct EditArgs {
    #[command(flatten)]
    pub target: TargetArgs,
    #[arg(long)]
    pub operation: Option<String>,
    #[arg(long)]
    pub review: Option<PathBuf>,
}
#[derive(Args)]
pub(crate) struct ExportArgs {
    #[command(flatten)]
    pub target: TargetArgs,
    pub blob: String,
    #[arg(long)]
    pub output: PathBuf,
    #[arg(long)]
    pub overwrite: bool,
}

#[derive(Subcommand)]
pub(crate) enum AttachmentCommand {
    #[command(about = crate::output::help("help_binary_list"))]
    List(TargetArgs),
    #[command(about = crate::output::help("help_attachment_add"))]
    Add {
        #[command(flatten)]
        edit: EditArgs,
        path: PathBuf,
        #[arg(long)]
        name: Option<String>,
    },
    #[command(about = crate::output::help("help_attachment_rename"))]
    Rename {
        #[command(flatten)]
        edit: EditArgs,
        attachment: String,
        #[arg(long)]
        name: String,
    },
    #[command(about = crate::output::help("help_attachment_replace"))]
    Replace {
        #[command(flatten)]
        edit: EditArgs,
        attachment: String,
        path: PathBuf,
    },
    #[command(about = crate::output::help("help_attachment_remove"))]
    Remove {
        #[command(flatten)]
        edit: EditArgs,
        attachment: String,
    },
    #[command(about = crate::output::help("help_binary_export"))]
    Export(ExportArgs),
}

#[derive(Subcommand)]
pub(crate) enum IconCommand {
    #[command(about = crate::output::help("help_icon_list"))]
    List,
    #[command(about = crate::output::help("help_binary_list"))]
    Show(TargetArgs),
    #[command(about = crate::output::help("help_icon_set"))]
    Set {
        #[command(flatten)]
        edit: EditArgs,
        key: String,
    },
    #[command(about = crate::output::help("help_icon_file"))]
    File {
        #[command(flatten)]
        edit: EditArgs,
        path: PathBuf,
    },
    #[command(about = crate::output::help("help_icon_url"))]
    Url {
        #[command(flatten)]
        edit: EditArgs,
        url: String,
    },
    #[command(about = crate::output::help("help_icon_favicon"))]
    Favicon {
        #[command(flatten)]
        edit: EditArgs,
        #[arg(long)]
        url: Option<String>,
    },
    #[command(about = crate::output::help("help_group_favicons"))]
    GroupFavicons {
        group: String,
        #[arg(long)]
        recursive: bool,
        #[arg(long)]
        replace: bool,
        #[arg(long)]
        operation: Option<String>,
    },
    #[command(about = crate::output::help("help_binary_export"))]
    Export(ExportArgs),
}

#[derive(Subcommand)]
pub(crate) enum AppearanceCommand {
    #[command(about = crate::output::help("help_appearance_set"))]
    Set {
        #[command(flatten)]
        edit: EditArgs,
        #[arg(long)]
        foreground: Option<String>,
        #[arg(long)]
        background: Option<String>,
        #[arg(long, conflicts_with = "foreground")]
        clear_foreground: bool,
        #[arg(long, conflicts_with = "background")]
        clear_background: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum StorageCommand {
    #[command(about = crate::output::help("help_storage_usage"))]
    Usage,
    #[command(about = crate::output::help("help_storage_gc"))]
    Gc {
        #[arg(long)]
        operation: Option<String>,
    },
}
