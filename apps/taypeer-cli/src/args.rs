use crate::lifecycle_args::{PendingCommand, PositionArgs, TrashCommand};
use clap::{Args, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Clone, Copy, Default, ValueEnum)]
pub(crate) enum Language {
    #[default]
    En,
    Ru,
}

#[derive(Parser)]
#[command(
    name = "taypeer-cli",
    version,
    about = "Encrypted databases and interactive sessions"
)]
pub(crate) struct Cli {
    /// Output machine-readable JSON.
    #[arg(long, global = true)]
    pub json: bool,
    /// Language for messages and help.
    #[arg(long, global = true, value_enum, default_value = "en")]
    pub lang: Language,
    /// Open this database before executing the command.
    #[arg(long, global = true)]
    pub file: Option<PathBuf>,
    /// Read the exact master password from stdin until EOF; no newline is removed.
    #[arg(long, global = true)]
    pub password_stdin: bool,
    #[command(subcommand)]
    pub command: Action,
}

#[derive(Subcommand)]
pub(crate) enum Action {
    /// Start an interactive session.
    Session,
    /// Create, open, select, lock or close databases.
    #[command(subcommand)]
    Db(DatabaseCommand),
    /// Create and inspect groups.
    #[command(subcommand)]
    Group(GroupCommand),
    /// Create, inspect and edit entries.
    #[command(subcommand)]
    Entry(EntryCommand),
    /// Work with an unconfirmed local draft.
    #[command(subcommand)]
    Draft(DraftCommand),
    /// Inspect, restore or purge saved revisions.
    #[command(subcommand)]
    History(HistoryCommand),
    /// Review and explicitly resolve conflicting values.
    #[command(subcommand)]
    Conflict(ConflictCommand),
    #[command(about = crate::output::help("help_lifecycle"))]
    #[command(subcommand)]
    Trash(TrashCommand),
    #[command(about = crate::output::help("help_pending"))]
    #[command(subcommand)]
    Pending(PendingCommand),
    /// Generate an offline password or passphrase.
    #[command(subcommand)]
    Generate(GenerateCommand),
    /// Search all unlocked databases in this session.
    Search { query: String },
    /// Close every database and leave the session.
    Exit,
    #[command(name = "__worker", hide = true)]
    Worker,
}

#[derive(Subcommand)]
pub(crate) enum DatabaseCommand {
    /// Create a new encrypted file without replacing an existing file.
    Create {
        path: PathBuf,
        #[arg(long)]
        name: String,
    },
    /// Authenticate and open a database.
    Open { path: PathBuf },
    /// List the session's databases.
    List,
    /// Select an already opened database by ID.
    Use { id: String },
    /// Lock the selected database and persist its draft.
    Lock,
    /// Authenticate the selected locked database again.
    Unlock,
    /// Close the selected database and release its file lock.
    Close,
}

#[derive(Subcommand)]
pub(crate) enum GroupCommand {
    #[command(about = crate::output::help("help_tree"))]
    Tree,
    #[command(about = crate::output::help("help_group_move"))]
    Move {
        id: String,
        #[arg(long)]
        parent: Option<String>,
        #[command(flatten)]
        position: PositionArgs,
        #[arg(long)]
        operation: Option<String>,
    },
    #[command(about = crate::output::help("help_group_resolve"))]
    Resolve {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        operation: Option<String>,
    },
    #[command(about = crate::output::help("help_group_clone"))]
    Clone {
        id: String,
        #[arg(long)]
        parent: Option<String>,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        operation: Option<String>,
    },
    #[command(about = crate::output::help("help_group_trash"))]
    Trash { id: String },
    /// List groups.
    List,
    /// Create a group.
    Create {
        #[arg(long)]
        name: String,
        #[arg(long)]
        parent: Option<String>,
    },
    /// Rename a group.
    Rename {
        id: String,
        #[arg(long)]
        name: String,
    },
}

#[derive(Subcommand)]
pub(crate) enum EntryCommand {
    #[command(about = crate::output::help("help_entry_move"))]
    Move {
        id: String,
        #[arg(long)]
        group: String,
        #[arg(long)]
        review: Option<PathBuf>,
        #[arg(long)]
        operation: Option<String>,
    },
    #[command(about = crate::output::help("help_entry_trash"))]
    Trash { id: String },
    /// List entries or search the selected database.
    List {
        #[arg(long)]
        group: Option<String>,
        #[arg(long, default_value = "")]
        query: String,
    },
    /// Show details with protected values hidden.
    Show { id: String },
    /// Create and save an entry.
    Create {
        #[arg(long)]
        group: String,
        #[command(flatten)]
        fields: Fields,
    },
    /// Save addressed changes; omitted fields keep their values.
    Update {
        id: String,
        #[command(flatten)]
        fields: Fields,
    },
    /// Clone an entry with fresh identities and a new history.
    Clone {
        id: String,
        #[arg(long)]
        group: String,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        operation: Option<String>,
    },
    /// Explicitly print one secret value.
    Reveal {
        id: String,
        #[arg(long)]
        attribute: Option<String>,
    },
}

#[derive(Subcommand)]
pub(crate) enum DraftCommand {
    /// Begin a new entry without saving it.
    Create { group: String },
    /// Begin editing an entry without exposing its existing secrets.
    Edit { id: String },
    /// Apply addressed fields to the current draft.
    Update {
        #[command(flatten)]
        fields: Fields,
    },
    /// Show draft status without its contents.
    Status,
    /// Confirm the active draft.
    Save,
    /// Explicitly restore an interrupted draft.
    Restore,
    /// Discard the active or interrupted draft.
    Discard,
}

#[derive(Args, Default)]
pub(crate) struct Fields {
    /// Read an addressed EntryPatch JSON document; '-' means stdin.
    #[arg(long, conflicts_with_all = ["title", "username", "url", "notes", "password_prompt", "clear_password"])]
    pub input: Option<PathBuf>,
    #[arg(long)]
    pub title: Option<String>,
    #[arg(long)]
    pub username: Option<String>,
    #[arg(long)]
    pub url: Option<String>,
    #[arg(long)]
    pub notes: Option<String>,
    /// Ask for an entry password without echoing it.
    #[arg(long, conflicts_with = "clear_password")]
    pub password_prompt: bool,
    /// Remove the entry's password field.
    #[arg(long)]
    pub clear_password: bool,
}

#[derive(Parser)]
#[command(name = "", disable_version_flag = true)]
pub(crate) struct SessionLine {
    #[command(subcommand)]
    pub action: Action,
}

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
        group: String,
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
}

#[derive(Subcommand)]
pub(crate) enum ConflictCommand {
    #[command(about = crate::output::help("help_generation"))]
    Generation {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        operation: Option<String>,
    },
    /// Show the review context and masked alternatives.
    Show { entry: String },
    /// Explicitly reveal an alternative selected by entry, field and origins in JSON.
    Reveal {
        #[arg(long)]
        input: PathBuf,
    },
    /// Submit an explicit context and whole-field resolutions from JSON.
    Resolve {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        operation: Option<String>,
    },
}

#[derive(Subcommand)]
pub(crate) enum GenerateCommand {
    /// Generate independent characters from the selected ASCII alphabet.
    Password {
        #[arg(long, default_value_t = 30)]
        length: u16,
        #[arg(long)]
        no_uppercase: bool,
        #[arg(long)]
        no_lowercase: bool,
        #[arg(long)]
        no_digits: bool,
        #[arg(long)]
        no_punctuation: bool,
        #[arg(long)]
        exclude_similar: bool,
        #[arg(long, default_value = "")]
        exclude: String,
    },
    /// Generate independent words from the bundled EFF Long Wordlist.
    Phrase {
        #[arg(long, default_value_t = 6)]
        words: u8,
        #[arg(long, default_value = "-")]
        separator: String,
    },
}
