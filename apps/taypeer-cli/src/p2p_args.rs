use clap::Subcommand;
use std::path::PathBuf;
use taypeer_trust::{DeviceId, Digest};

#[derive(Subcommand)]
pub(crate) enum SyncCommand {
    #[command(about = crate::output::help("sync_start"))]
    Start {
        #[arg(long)]
        relay: Option<String>,
        #[arg(long, requires = "relay", help = crate::output::help("relay_only"))]
        relay_only: bool,
        #[arg(long, requires = "relay_only", help = crate::output::help("relay_ca"))]
        relay_ca: Option<PathBuf>,
    },
    #[command(about = crate::output::help("sync_stop"))]
    Stop,
    #[command(about = crate::output::help("sync_status"))]
    Status,
    #[command(about = crate::output::help("sync_now"))]
    Now {
        #[arg(long)]
        peer: PathBuf,
    },
    #[command(about = crate::output::help("sync_apply"))]
    Apply,
}
#[derive(Subcommand)]
pub(crate) enum InviteCommand {
    #[command(about = crate::output::help("invite_create"))]
    Create,
    #[command(about = crate::output::help("invite_requests"))]
    Requests,
    #[command(about = crate::output::help("invite_approve"))]
    Approve { request: Digest },
    #[command(about = crate::output::help("invite_reject"))]
    Reject { request: Digest },
    #[command(about = crate::output::help("invite_cancel"))]
    Cancel { request: Digest },
    #[command(about = crate::output::help("invite_join"))]
    Join {
        path: PathBuf,
        #[arg(long)]
        input: Option<PathBuf>,
    },
    #[command(about = crate::output::help("invite_resume"))]
    Resume { request: Digest },
    #[command(about = crate::output::help("invite_pending"))]
    Pending,
}
#[derive(Subcommand)]
pub(crate) enum DeviceCommand {
    #[command(about = crate::output::help("device_list"))]
    List,
    #[command(about = crate::output::help("device_password"))]
    Password {
        #[arg(long)]
        operation: Option<Digest>,
        #[arg(long)]
        input: Option<PathBuf>,
    },
    #[command(about = crate::output::help("device_revoke"))]
    Revoke {
        device: DeviceId,
        #[arg(long)]
        operation: Option<Digest>,
        #[arg(long)]
        input: Option<PathBuf>,
    },
    #[command(about = crate::output::help("device_consent"))]
    Consent { operation: Digest },
    #[command(about = crate::output::help("device_transfer"))]
    Transfer {
        #[arg(long)]
        input: PathBuf,
    },
    #[command(about = crate::output::help("device_policy"))]
    Policy,
    #[command(about = crate::output::help("device_set_policy"))]
    SetPolicy {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        operation: Option<Digest>,
        #[arg(long)]
        password_input: Option<PathBuf>,
    },
}
