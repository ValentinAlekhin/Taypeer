//! Converts command-line paths and selectors to shared service requests.

use crate::{binary_args::*, host::operation, input::Input, output::CliError};
use taypeer_core::{AttachmentId, BlobId, Color, EntryId, GenerationId, GroupId, RevisionId};
use taypeer_runtime::Command;
use taypeer_services::{
    AttachmentEdit, BinaryEdit, BinaryRequest, BinaryTarget, FieldUpdate, IconInput,
    InspectionTarget, ObjectAddress, ObjectId,
};

fn target(args: TargetArgs) -> Result<BinaryTarget, CliError> {
    if usize::from(args.entry.is_some())
        + usize::from(args.group.is_some())
        + usize::from(args.draft)
        + usize::from(args.source.is_some())
        != 1
    {
        return Err(CliError::Input);
    }
    if let Some(source) = args.source {
        if args.generation.is_some() {
            return Err(CliError::Input);
        }
        return Ok(BinaryTarget::Inspection(InspectionTarget::Source(source)));
    }
    if args.draft {
        if args.generation.is_some() {
            return Err(CliError::Input);
        }
        return Ok(BinaryTarget::Draft);
    }
    if let Some(entry) = args.entry {
        let entry = EntryId::new(entry);
        if let Some(revision) = args.revision {
            return Ok(BinaryTarget::Revision {
                entry,
                revision: RevisionId::new(revision),
            });
        }
        if let Some(generation) = args.generation {
            return Ok(BinaryTarget::Inspection(InspectionTarget::Object(
                ObjectAddress {
                    object: ObjectId::Entry(entry),
                    generation: GenerationId::new(generation),
                },
            )));
        }
        return Ok(BinaryTarget::Entry(entry));
    }
    let group = GroupId::new(args.group.ok_or(CliError::Input)?);
    if let Some(generation) = args.generation {
        return Ok(BinaryTarget::Inspection(InspectionTarget::Object(
            ObjectAddress {
                object: ObjectId::Group(group),
                generation: GenerationId::new(generation),
            },
        )));
    }
    Ok(BinaryTarget::Group(group))
}
fn edit(args: EditArgs, edit: BinaryEdit, input: &Input) -> Result<Command, CliError> {
    Ok(Command::EditBinary {
        request: BinaryRequest {
            target: target(args.target)?,
            edit,
            review: args.review.map(|path| input.document(&path)).transpose()?,
        },
        operation: operation(args.operation)?,
    })
}
fn export(args: ExportArgs) -> Result<Command, CliError> {
    Ok(Command::ExportBinary {
        target: target(args.target)?,
        blob: BlobId::new(args.blob),
        path: args.output,
        overwrite: args.overwrite,
    })
}

pub(crate) fn attachment(command: AttachmentCommand, input: &Input) -> Result<Command, CliError> {
    let (args, action) = match command {
        AttachmentCommand::List(args) => return Ok(Command::BinaryView(target(args)?)),
        AttachmentCommand::Export(args) => return export(args),
        AttachmentCommand::Add { edit, path, name } => (edit, AttachmentEdit::Add { path, name }),
        AttachmentCommand::Rename {
            edit,
            attachment,
            name,
        } => (
            edit,
            AttachmentEdit::Rename {
                attachment: AttachmentId::new(attachment),
                name,
            },
        ),
        AttachmentCommand::Replace {
            edit,
            attachment,
            path,
        } => (
            edit,
            AttachmentEdit::Replace {
                attachment: AttachmentId::new(attachment),
                path,
            },
        ),
        AttachmentCommand::Remove { edit, attachment } => (
            edit,
            AttachmentEdit::Remove {
                attachment: AttachmentId::new(attachment),
            },
        ),
    };
    edit(args, BinaryEdit::Attachment(action), input)
}
pub(crate) fn icon(command: IconCommand, input: &Input) -> Result<Command, CliError> {
    let (args, source) = match command {
        IconCommand::List => return Err(CliError::Input),
        IconCommand::Show(args) => return Ok(Command::BinaryView(target(args)?)),
        IconCommand::Export(args) => return export(args),
        IconCommand::Set { edit, key } => (
            edit,
            if key == "default" {
                IconInput::Default
            } else {
                IconInput::Lucide(key.try_into().map_err(|_| CliError::Input)?)
            },
        ),
        IconCommand::File { edit, path } => (edit, IconInput::File(path)),
        IconCommand::Url { edit, url } => (edit, IconInput::Url(url)),
        IconCommand::Favicon { edit, url } => (edit, IconInput::Favicon(url)),
        IconCommand::GroupFavicons {
            group,
            recursive,
            replace,
            operation: op,
        } => {
            return Ok(Command::GroupFavicons {
                group: GroupId::new(group),
                recursive,
                replace,
                operation: operation(op)?,
            });
        }
    };
    edit(args, BinaryEdit::Icon(source), input)
}
fn color(value: Option<String>, clear: bool) -> Result<FieldUpdate<Color>, CliError> {
    if clear {
        return Ok(FieldUpdate::Clear);
    }
    let Some(value) = value else {
        return Ok(FieldUpdate::Keep);
    };
    let value = value.strip_prefix('#').unwrap_or(&value);
    if !matches!(value.len(), 6 | 8) || !value.is_ascii() {
        return Err(CliError::Input);
    }
    let mut rgba = [255; 4];
    for (index, byte) in rgba.iter_mut().enumerate().take(value.len() / 2) {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16)
            .map_err(|_| CliError::Input)?;
    }
    Ok(FieldUpdate::Set(Color(rgba)))
}
pub(crate) fn appearance(command: AppearanceCommand, input: &Input) -> Result<Command, CliError> {
    let AppearanceCommand::Set {
        edit: args,
        foreground,
        background,
        clear_foreground,
        clear_background,
    } = command;
    edit(
        args,
        BinaryEdit::Appearance {
            foreground: color(foreground, clear_foreground)?,
            background: color(background, clear_background)?,
        },
        input,
    )
}
pub(crate) fn storage(command: StorageCommand) -> Result<Command, CliError> {
    Ok(match command {
        StorageCommand::Usage => Command::StorageUsage,
        StorageCommand::Gc { operation: op } => Command::CollectBlobs(operation(op)?),
    })
}
