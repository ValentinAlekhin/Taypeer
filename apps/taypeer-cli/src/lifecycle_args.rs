//! CLI syntax only; all lifecycle selection and placement rules live in Rust services.

use clap::{Args, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Args)]
#[group(multiple = false)]
pub(crate) struct PositionArgs {
    #[arg(long)]
    pub first: bool,
    #[arg(long)]
    pub last: bool,
    #[arg(long)]
    pub before: Option<String>,
    #[arg(long)]
    pub after: Option<String>,
}

#[derive(Clone, Copy, ValueEnum)]
pub(crate) enum ObjectKind {
    Group,
    Entry,
}
#[derive(Clone, Copy, ValueEnum)]
pub(crate) enum Transition {
    Trash,
    Restore,
    Purge,
}

#[derive(Subcommand)]
pub(crate) enum TrashCommand {
    #[command(about = crate::output::help("help_trash_list"))]
    List,
    #[command(about = crate::output::help("help_trash_prepare"))]
    Prepare {
        #[arg(value_enum)]
        action: Transition,
        #[arg(value_enum)]
        kind: ObjectKind,
        id: String,
        #[arg(long)]
        destination: Option<String>,
    },
    #[command(about = crate::output::help("help_trash_confirm"))]
    Confirm {
        #[arg(long)]
        input: PathBuf,
        #[arg(long, required = true)]
        yes: bool,
        #[arg(long)]
        operation: Option<String>,
    },
    #[command(about = crate::output::help("help_trash_show"))]
    Show {
        #[arg(value_enum)]
        kind: ObjectKind,
        id: String,
        #[arg(long)]
        generation: String,
    },
    #[command(about = crate::output::help("help_trash_reveal"))]
    Reveal {
        #[arg(long)]
        input: PathBuf,
    },
}

#[derive(Subcommand)]
pub(crate) enum PendingCommand {
    #[command(about = crate::output::help("help_pending_list"))]
    List,
    #[command(about = crate::output::help("help_pending_show"))]
    Show { source: String },
    #[command(about = crate::output::help("help_pending_restore"))]
    Restore {
        source: String,
        #[arg(long)]
        destination: Option<String>,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        operation: Option<String>,
    },
    #[command(about = crate::output::help("help_pending_clone"))]
    Clone {
        source: String,
        #[arg(long)]
        destination: Option<String>,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        operation: Option<String>,
    },
    #[command(about = crate::output::help("help_pending_recover"))]
    Recover {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        operation: Option<String>,
    },
    #[command(about = crate::output::help("help_pending_reveal"))]
    Reveal {
        #[arg(long)]
        input: PathBuf,
    },
}
