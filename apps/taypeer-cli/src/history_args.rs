//! Saved originals and alternatives remain available without conflict-resolution flows.

use clap::Subcommand;
use std::path::PathBuf;

#[derive(Subcommand)]
pub(crate) enum HistoryCommand {
    /// List saved revisions with secrets hidden.
    List { entry: String },
    /// Show a masked saved revision.
    Show { entry: String, revision: String },
    /// Restore a saved revision as a new current version.
    Restore {
        entry: String,
        revision: String,
        #[arg(long)]
        group: Option<String>,
        #[arg(long)]
        operation: Option<String>,
    },
    /// Permanently remove selected revisions from available history.
    Purge {
        entry: String,
        #[arg(long, required = true)]
        revision: Vec<String>,
        #[arg(long, required = true)]
        yes: bool,
        #[arg(long)]
        operation: Option<String>,
    },
    #[command(about = crate::output::help("help_history_reveal"))]
    Reveal {
        entry: String,
        revision: String,
        #[arg(long)]
        attribute: Option<String>,
    },
    #[command(about = crate::output::help("help_history_alternatives"))]
    Alternatives { entry: String },
    #[command(about = crate::output::help("help_history_reveal_alternative"))]
    RevealAlternative {
        #[arg(long)]
        input: PathBuf,
    },
    #[command(about = crate::output::help("help_history_group"), subcommand)]
    Group(MetadataHistoryCommand),
    #[command(about = crate::output::help("help_history_database"), subcommand)]
    Database(DatabaseHistoryCommand),
}

#[derive(Subcommand)]
pub(crate) enum MetadataHistoryCommand {
    List {
        id: String,
    },
    Purge {
        id: String,
        #[arg(long, required = true)]
        revision: Vec<String>,
        #[arg(long, required = true)]
        yes: bool,
        #[arg(long)]
        operation: Option<String>,
    },
}

#[derive(Subcommand)]
pub(crate) enum DatabaseHistoryCommand {
    List,
    Purge {
        #[arg(long, required = true)]
        revision: Vec<String>,
        #[arg(long, required = true)]
        yes: bool,
        #[arg(long)]
        operation: Option<String>,
    },
}
