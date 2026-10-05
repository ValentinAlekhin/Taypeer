use crate::lifecycle_args::{PendingCommand, PositionArgs, TrashCommand};
pub(crate) use crate::{draft_args::DraftCommand, history_args::HistoryCommand};
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
    #[arg(long, global = true, help = crate::output::help("help_profile"))]
    pub profile: Option<PathBuf>,
    #[cfg(feature = "ui-test-support")]
    #[arg(long, global = true, hide = true, requires = "profile")]
    pub public_fixture_profile: bool,
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
    #[command(about = crate::output::help("help_settings"), subcommand)]
    Settings(SettingsCommand),
    #[command(about = crate::output::help("help_sync"), subcommand)]
    Sync(crate::p2p_args::SyncCommand),
    #[command(about = crate::output::help("help_invite"), subcommand)]
    Invite(crate::p2p_args::InviteCommand),
    #[command(about = crate::output::help("help_device"), subcommand)]
    Device(crate::p2p_args::DeviceCommand),
    #[command(about = crate::output::help("help_attachment"), subcommand)]
    Attachment(crate::binary_args::AttachmentCommand),
    #[command(about = crate::output::help("help_icon"), subcommand)]
    Icon(crate::binary_args::IconCommand),
    #[command(about = crate::output::help("help_appearance"), subcommand)]
    Appearance(crate::binary_args::AppearanceCommand),
    #[command(about = crate::output::help("help_storage"), subcommand)]
    Storage(crate::binary_args::StorageCommand),
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
    #[command(about = crate::output::help("help_drafts"), subcommand)]
    Draft(DraftCommand),
    /// Inspect, restore or purge saved revisions.
    #[command(subcommand)]
    History(HistoryCommand),
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
    #[cfg(feature = "ui-test-support")]
    #[command(name = "__public_fixture_worker", hide = true)]
    PublicFixtureWorker,
}

#[derive(Subcommand)]
pub(crate) enum SettingsCommand {
    #[command(about = crate::output::help("help_auto_lock"))]
    AutoLock {
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..), help = crate::output::help("help_auto_lock_seconds"))]
        seconds: Option<u32>,
    },
}

#[derive(Subcommand)]
pub(crate) enum DatabaseCommand {
    #[command(about = crate::output::help("help_compatibility"))]
    Compatibility,
    #[command(about = crate::output::help("help_db_create"))]
    Create {
        #[arg(long)]
        name: String,
        #[arg(long)]
        operation: Option<String>,
    },
    /// Authenticate and open a database.
    Open { path: PathBuf },
    #[command(about = crate::output::help("help_db_list"))]
    List,
    /// Select an already opened database by ID.
    Use { id: String },
    /// Lock the selected database and persist its draft.
    Lock,
    /// Authenticate the selected locked database again.
    Unlock,
    /// Close the selected database and release its file lock.
    Close,
    #[command(about = crate::output::help("help_db_info"))]
    Info,
    #[command(about = crate::output::help("help_db_relocate"))]
    Relocate { path: PathBuf },
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
    Trash {
        id: String,
        #[arg(long)]
        operation: Option<String>,
    },
    /// List groups.
    List,
    /// Create a group.
    Create {
        #[arg(long)]
        operation: Option<String>,
        #[arg(long)]
        name: String,
        #[arg(long)]
        parent: Option<String>,
    },
    /// Rename a group.
    Rename {
        #[arg(long)]
        operation: Option<String>,
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
        group: Option<String>,
        #[arg(long)]
        operation: Option<String>,
    },
    #[command(about = crate::output::help("help_entry_trash"))]
    Trash {
        id: String,
        #[arg(long)]
        operation: Option<String>,
    },
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
        operation: Option<String>,
        #[arg(long)]
        group: Option<String>,
        #[command(flatten)]
        fields: Fields,
    },
    /// Save addressed changes; omitted fields keep their values.
    Update {
        #[arg(long)]
        operation: Option<String>,
        id: String,
        #[command(flatten)]
        fields: Fields,
    },
    /// Clone an entry with fresh identities and a new history.
    Clone {
        id: String,
        #[arg(long)]
        group: Option<String>,
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

#[cfg(test)]
mod retry_tests {
    use super::*;
    #[test]
    fn ordinary_writes_accept_an_explicit_retry_identity() {
        for args in [
            vec!["group", "create", "--name", "PUBLIC group"],
            vec!["group", "rename", "PUBLIC-id", "--name", "PUBLIC renamed"],
            vec![
                "entry",
                "create",
                "--group",
                "PUBLIC-group",
                "--title",
                "PUBLIC entry",
            ],
            vec![
                "entry",
                "update",
                "PUBLIC-entry",
                "--title",
                "PUBLIC updated",
            ],
            vec!["draft", "save"],
        ] {
            let command = Cli::try_parse_from(
                std::iter::once("taypeer-cli")
                    .chain(args)
                    .chain(["--operation", "PUBLIC-retry"]),
            )
            .unwrap()
            .command;
            let operation = match command {
                Action::Group(
                    GroupCommand::Create { operation, .. } | GroupCommand::Rename { operation, .. },
                )
                | Action::Entry(
                    EntryCommand::Create { operation, .. } | EntryCommand::Update { operation, .. },
                )
                | Action::Draft(DraftCommand::Save { operation }) => operation,
                _ => panic!("expected an ordinary write"),
            };
            assert_eq!(operation.as_deref(), Some("PUBLIC-retry"));
        }
    }

    #[test]
    fn automation_routes_keep_confirmations_and_remove_interactive_conflict_steps() {
        for args in [
            vec!["db", "create", "/PUBLIC/legacy.taypeer", "--name", "PUBLIC"],
            vec!["invite", "join", "/PUBLIC/legacy.taypeer"],
            vec!["draft", "restore"],
            vec!["conflict", "resolve", "--input", "PUBLIC.json"],
            vec!["group", "resolve", "--input", "PUBLIC.json"],
            vec!["entry", "move", "PUBLIC-entry", "--review", "PUBLIC.json"],
            vec![
                "history",
                "purge",
                "PUBLIC-entry",
                "--revision",
                "PUBLIC-revision",
            ],
            vec![
                "history",
                "group",
                "purge",
                "PUBLIC-group",
                "--revision",
                "PUBLIC-revision",
            ],
            vec![
                "history",
                "database",
                "purge",
                "--revision",
                "PUBLIC-revision",
            ],
        ] {
            assert!(Cli::try_parse_from(std::iter::once("taypeer-cli").chain(args)).is_err());
        }
        for args in [
            vec!["entry", "create", "--title", "PUBLIC"],
            vec!["entry", "move", "PUBLIC-entry"],
            vec!["entry", "clone", "PUBLIC-entry"],
            vec!["history", "restore", "PUBLIC-entry", "PUBLIC-revision"],
            vec!["draft", "create", "--username", "PUBLIC unfinished"],
            vec![
                "db",
                "create",
                "--name",
                "PUBLIC",
                "--operation",
                "PUBLIC create",
            ],
            vec![
                "draft",
                "snapshot",
                "PUBLIC-draft",
                "--revision",
                "7",
                "--operation",
                "PUBLIC save",
            ],
            vec![
                "entry",
                "trash",
                "PUBLIC-entry",
                "--operation",
                "PUBLIC trash",
            ],
        ] {
            assert!(Cli::try_parse_from(std::iter::once("taypeer-cli").chain(args)).is_ok());
        }
    }

    #[test]
    fn public_fixture_credentials_are_explicit_and_disabled_in_production() {
        assert!(
            Cli::try_parse_from(["taypeer-cli", "--public-fixture-profile", "db", "list"]).is_err()
        );
        let parsed = Cli::try_parse_from([
            "taypeer-cli",
            "--profile",
            "/PUBLIC/profile",
            "--public-fixture-profile",
            "db",
            "list",
        ]);
        assert_eq!(parsed.is_ok(), cfg!(feature = "ui-test-support"));
    }
}
