//! Child-side dispatch. Only this process constructs a plaintext database service.

use crate::{
    Command, RuntimeError,
    protocol::{Boot, Response, read_frame, write_frame},
};
use serde::Serialize;
use serde_json::{Value, json};
use std::io::{Read, Write};
use taypeer_services::{DatabaseService, SessionToken};
use zeroize::Zeroize;

/// Serve one database over inherited private pipes until lock or EOF.
/// Never attach this entry point to a public socket or ordinary terminal output.
pub fn run_worker(reader: &mut impl Read, writer: &mut impl Write) -> Result<(), RuntimeError> {
    let mut boot: Boot = read_frame(reader)?;
    let mut service = DatabaseService::new();
    let opened = match boot.create_name.take() {
        Some(name) => service.create_file(&boot.path, name, boot.password.as_bytes()),
        None => service.open_file(&boot.path, boot.password.as_bytes()),
    };
    boot.password.zeroize();
    let session = match opened {
        Ok(session) => session,
        Err(error) => {
            write_frame(
                writer,
                &Response {
                    result: Err(error.into()),
                },
            )?;
            return Ok(());
        }
    };
    write_frame(
        writer,
        &Response {
            result: value(&session.database),
        },
    )?;
    let outcome = serve(reader, writer, &mut service, &session);
    // EOF and protocol errors also revoke access. A broken parent cannot receive
    // a draft failure, but the process must still stop holding the document.
    let closed = service.lock_all_checked().map_err(RuntimeError::from);
    outcome.and(closed)
}

fn serve(
    reader: &mut impl Read,
    writer: &mut impl Write,
    service: &mut DatabaseService,
    session: &SessionToken,
) -> Result<(), RuntimeError> {
    loop {
        let command: Command = read_frame(reader)?;
        let lock = matches!(command, Command::Lock);
        let result = dispatch(service, session, command);
        write_frame(writer, &Response { result })?;
        if lock {
            return Ok(());
        }
    }
}

fn value(data: &impl Serialize) -> Result<Value, RuntimeError> {
    serde_json::to_value(data).map_err(|_| RuntimeError::Protocol)
}

fn dispatch(
    service: &mut DatabaseService,
    session: &SessionToken,
    command: Command,
) -> Result<Value, RuntimeError> {
    Ok(match command {
        Command::BinaryView(target) => value(&service.binary_view(session, &target)?.value)?,
        Command::EditBinary { request, operation } => {
            value(&service.edit_binary(session, &request, &operation)?.value)?
        }
        Command::ExportBinary {
            target,
            blob,
            path,
            overwrite,
        } => value(
            &service
                .export_binary(session, &target, &blob, &path, overwrite)?
                .value,
        )?,
        Command::StorageUsage => value(&service.storage_usage(session)?.value)?,
        Command::CollectBlobs(operation) => {
            value(&service.collect_blobs(session, &operation)?.value)?
        }
        Command::GroupFavicons {
            group,
            recursive,
            replace,
            operation,
        } => value(
            &service
                .group_favicons(session, &group, recursive, replace, &operation)?
                .value,
        )?,
        Command::Tree => value(&service.tree(session)?.value)?,
        Command::Trash => value(&service.trash(session)?.value)?,
        Command::Inspect(target) => value(&service.inspect_object(session, &target)?.value)?,
        Command::RevealInspected {
            target,
            field,
            origins,
        } => value(
            &service
                .reveal_inspected(session, &target, &field, &origins)?
                .value,
        )?,
        Command::PrepareLifecycle {
            action,
            target,
            destination,
        } => value(
            &service
                .prepare_lifecycle(session, action, target, destination)?
                .value,
        )?,
        Command::ConfirmLifecycle {
            prepared,
            operation,
        } => value(
            &service
                .confirm_lifecycle(session, &prepared, &operation)?
                .value,
        )?,
        Command::MoveGroup { request, operation } => {
            value(&service.move_group(session, &request, &operation)?.value)?
        }
        Command::MoveEntry {
            entry,
            group,
            review,
            operation,
        } => value(
            &service
                .move_entry(session, &entry, group, review, &operation)?
                .value,
        )?,
        Command::CloneGroup {
            group,
            parent,
            name,
            operation,
        } => value(
            &service
                .clone_group(session, &group, parent, name, &operation)?
                .value,
        )?,
        Command::PendingSources => value(&service.pending_sources(session)?.value)?,
        Command::RecoverSource { request, operation } => {
            value(&service.recover_source(session, &request, &operation)?.value)?
        }
        Command::ResolveGeneration {
            address,
            heads,
            operation,
        } => value(
            &service
                .resolve_generation(session, &address, &heads, &operation)?
                .value,
        )?,
        Command::Groups => value(&service.groups(session)?.value)?,
        Command::CreateGroup { name, parent } => {
            value(&service.create_group(session, name, parent)?.value)?
        }
        Command::RenameGroup { id, name } => {
            value(&service.update_group(session, &id, name)?.value)?
        }
        Command::Entries { group, query } => {
            value(&service.entries(session, group.as_ref(), &query)?.value)?
        }
        Command::Entry(id) => value(&service.view_entry(session, &id)?.value)?,
        Command::CreateEntry { group, patch } => {
            service.start_create_entry(session, group)?;
            service.patch_draft(session, patch)?;
            value(&service.save_draft(session)?.value)?
        }
        Command::UpdateEntry { id, patch } => {
            service.start_edit_entry(session, &id)?;
            service.patch_draft(session, patch)?;
            value(&service.save_draft(session)?.value)?
        }
        Command::BeginCreate(group) => {
            service.start_create_entry(session, group)?;
            Value::Null
        }
        Command::BeginEdit(id) => {
            service.start_edit_entry(session, &id)?;
            Value::Null
        }
        Command::PatchDraft(patch) => {
            service.patch_draft(session, patch)?;
            Value::Null
        }
        Command::DraftStatus => {
            let draft = service.draft(session)?.value;
            json!({"active": draft.is_some(), "dirty": draft.as_ref().is_some_and(|d| d.dirty),
                "pending": service.pending_draft(session)?.value})
        }
        Command::SaveDraft => value(&service.save_draft(session)?.value)?,
        Command::DiscardDraft => {
            service.cancel_draft(session)?;
            Value::Null
        }
        Command::RestoreDraft => {
            service.restore_draft(session)?;
            Value::Null
        }
        Command::History(entry) => value(&service.history(session, &entry)?.value)?,
        Command::Revision { entry, revision } => {
            value(&service.revision(session, &entry, &revision)?.value)?
        }
        Command::CloneEntry {
            entry,
            group,
            title,
            operation,
        } => value(
            &service
                .clone_entry(session, &entry, group, title, &operation)?
                .value,
        )?,
        Command::RestoreRevision {
            entry,
            revision,
            group,
            operation,
        } => value(
            &service
                .restore_revision(session, &entry, &revision, group, &operation)?
                .value,
        )?,
        Command::PurgeHistory {
            entry,
            revisions,
            operation,
        } => {
            service.purge_history(session, &entry, revisions, &operation)?;
            Value::Null
        }
        Command::Conflicts(entry) => value(&service.conflicts(session, &entry)?.value)?,
        Command::ResolveConflicts {
            context,
            fields,
            operation,
        } => value(
            &service
                .resolve_conflicts(session, &context, fields, &operation)?
                .value,
        )?,
        Command::RevealConflict {
            entry,
            field,
            origins,
        } => value(
            &service
                .reveal_conflict_variant(session, &entry, &field, &origins)?
                .value,
        )?,
        Command::RevealPassword(entry) => {
            value(&service.reveal_password(session, &entry)?.value.expose())?
        }
        Command::RevealAttribute { entry, attribute } => value(
            &service
                .reveal_attribute(session, &entry, &attribute)?
                .value
                .expose(),
        )?,
        Command::Lock => {
            service.lock(session)?;
            Value::Null
        }
    })
}
