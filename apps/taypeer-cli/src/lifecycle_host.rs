//! Typed CLI-to-service request conversion; no document rules or mutations.

use crate::{host::operation, input::Input, lifecycle_args::*, output::CliError};
use std::path::Path;
use taypeer_core::{EntryId, GenerationId, GroupId, OperationId};
use taypeer_runtime::Command;
use taypeer_services::{
    InspectionTarget, LifecycleAction, ObjectAddress, ObjectId, RecoveryMode, RecoveryRequest,
    SiblingPosition,
};

fn object(kind: ObjectKind, id: String) -> ObjectId {
    match kind {
        ObjectKind::Group => ObjectId::Group(GroupId::new(id)),
        ObjectKind::Entry => ObjectId::Entry(EntryId::new(id)),
    }
}

pub(crate) fn position(args: PositionArgs) -> SiblingPosition {
    if args.first {
        SiblingPosition::First
    } else if let Some(id) = args.before {
        SiblingPosition::Before(GroupId::new(id))
    } else if let Some(id) = args.after {
        SiblingPosition::After(GroupId::new(id))
    } else {
        SiblingPosition::Last
    }
}

pub(crate) fn trash(command: TrashCommand, input: &Input) -> Result<Command, CliError> {
    Ok(match command {
        TrashCommand::List => Command::Trash,
        TrashCommand::Prepare {
            action,
            kind,
            id,
            destination,
        } => Command::PrepareLifecycle {
            action: match action {
                Transition::Trash => LifecycleAction::Trash,
                Transition::Restore => LifecycleAction::Restore,
                Transition::Purge => LifecycleAction::Purge,
            },
            target: object(kind, id),
            destination: destination.map(GroupId::new),
        },
        TrashCommand::Confirm {
            input: path,
            yes: _,
            operation: op,
        } => Command::ConfirmLifecycle {
            prepared: input.document(&path)?,
            operation: operation(op)?,
        },
        TrashCommand::Show {
            kind,
            id,
            generation,
        } => Command::Inspect(InspectionTarget::Object(ObjectAddress {
            object: object(kind, id),
            generation: GenerationId::new(generation),
        })),
        TrashCommand::Reveal { input: path } => reveal(&path, input)?,
    })
}

pub(crate) fn pending(command: PendingCommand, input: &Input) -> Result<Command, CliError> {
    Ok(match command {
        PendingCommand::List => Command::PendingSources,
        PendingCommand::Show { source } => Command::Inspect(InspectionTarget::Source(source)),
        PendingCommand::Restore {
            source,
            destination,
            name,
            operation: op,
        } => Command::RecoverSource {
            request: RecoveryRequest {
                source,
                mode: RecoveryMode::Restore,
                destination: destination.map(GroupId::new),
                name,
                fields: None,
            },
            operation: operation(op)?,
        },
        PendingCommand::Clone {
            source,
            destination,
            name,
            operation: op,
        } => Command::RecoverSource {
            request: RecoveryRequest {
                source,
                mode: RecoveryMode::Clone,
                destination: destination.map(GroupId::new),
                name,
                fields: None,
            },
            operation: operation(op)?,
        },
        PendingCommand::Recover {
            input: path,
            operation: op,
        } => Command::RecoverSource {
            request: input.document(&path)?,
            operation: operation(op)?,
        },
        PendingCommand::Reveal { input: path } => reveal(&path, input)?,
    })
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RevealInput {
    target: InspectionTarget,
    field: taypeer_core::EntryField,
    origins: Vec<String>,
}
fn reveal(path: &Path, input: &Input) -> Result<Command, CliError> {
    let request: RevealInput = input.document(path)?;
    Ok(Command::RevealInspected {
        target: request.target,
        field: request.field,
        origins: request.origins,
    })
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct GenerationInput {
    address: ObjectAddress,
    heads: Vec<String>,
}
pub(crate) fn generation(
    path: &Path,
    input: &Input,
    operation: OperationId,
) -> Result<Command, CliError> {
    let request: GenerationInput = input.document(path)?;
    Ok(Command::ResolveGeneration {
        address: request.address,
        heads: request.heads,
        operation,
    })
}
