//! Attachment, appearance and explicit icon input forms.
use super::entry::submit_editor;
use super::*;

fn binary_command(
    edit: taypeer_services::BinaryEdit,
    operation: taypeer_core::OperationId,
) -> taypeer_runtime::Command {
    taypeer_runtime::Command::EditBinary {
        request: taypeer_services::BinaryRequest {
            target: taypeer_services::BinaryTarget::Draft,
            edit,
            review: None,
        },
        operation,
    }
}

pub(in crate::macos::ui) fn attachment(
    editor: Entity<EditorStore>,
    index: Option<usize>,
    window: &mut Window,
    cx: &mut App,
) {
    if !editor.read(cx).editable() || editor.read(cx).busy() {
        return;
    }
    let operation = taypeer_services::new_operation_id().ok();
    if let Some(attachment) = index
        .and_then(|i| editor.read(cx).content().attachments.get(i))
        .cloned()
    {
        text_form(
            "ui.attachment",
            vec![("name", attachment.name, false)],
            Box::new(move |values, _, cx| {
                require_name(&values[0])?;
                submit_editor(
                    &editor,
                    binary_command(
                        taypeer_services::BinaryEdit::Attachment(
                            taypeer_services::AttachmentEdit::Rename {
                                attachment: attachment.id.clone(),
                                name: values[0].clone(),
                            },
                        ),
                        operation.clone().ok_or(FormError::Backend)?,
                    ),
                    cx,
                )
                .map(Some)
            }),
            window,
            cx,
        );
    } else {
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        });
        let handle = window.window_handle();
        cx.spawn(async move |cx| {
            let selected = prompt.await;
            let _ = handle.update(cx, |_, _, cx| {
                if let Ok(Ok(Some(paths))) = selected
                    && let Some(path) = paths.into_iter().next()
                {
                    editor.update(cx, |e, cx| {
                        if e.connection().control.is_open() {
                            e.binary(taypeer_services::BinaryEdit::Attachment(
                                taypeer_services::AttachmentEdit::Add { path, name: None },
                            ));
                            cx.notify();
                        }
                    });
                }
            });
        })
        .detach();
    }
}

pub(in crate::macos::ui) fn image_url(
    editor: Entity<EditorStore>,
    window: &mut Window,
    cx: &mut App,
) {
    let operation = taypeer_services::new_operation_id().ok();
    text_form(
        "ui.image_url",
        vec![("url", String::new(), false)],
        Box::new(move |values, _, cx| {
            submit_editor(
                &editor,
                binary_command(
                    taypeer_services::BinaryEdit::Icon(taypeer_services::IconInput::Url(
                        values[0].clone(),
                    )),
                    operation.clone().ok_or(FormError::Backend)?,
                ),
                cx,
            )
            .map(Some)
        }),
        window,
        cx,
    );
}

pub(in crate::macos::ui) fn color(
    editor: Entity<EditorStore>,
    background: bool,
    window: &mut Window,
    cx: &mut App,
) {
    let operation = taypeer_services::new_operation_id().ok();
    let color = if background {
        editor.read(cx).content().background
    } else {
        editor.read(cx).content().foreground
    };
    text_form(
        if background {
            "ui.background_color"
        } else {
            "ui.foreground_color"
        },
        vec![(
            "ui.hex_color",
            color.map(|c| format!("#{c:06x}")).unwrap_or_default(),
            false,
        )],
        Box::new(move |values, _, cx| {
            let value = &values[0];
            let color = if value.is_empty() {
                None
            } else {
                let raw = value.strip_prefix('#').unwrap_or(value);
                if raw.len() != 6 {
                    return Err(FormError::InvalidColor);
                }
                Some(u32::from_str_radix(raw, 16).map_err(|_| FormError::InvalidColor)?)
            };
            let update = color
                .map(|v| taypeer_core::Color([(v >> 16) as u8, (v >> 8) as u8, v as u8, 255]))
                .map_or(
                    taypeer_services::FieldUpdate::Clear,
                    taypeer_services::FieldUpdate::Set,
                );
            let edit = if background {
                taypeer_services::BinaryEdit::Appearance {
                    foreground: taypeer_services::FieldUpdate::Keep,
                    background: update,
                }
            } else {
                taypeer_services::BinaryEdit::Appearance {
                    foreground: update,
                    background: taypeer_services::FieldUpdate::Keep,
                }
            };
            submit_editor(
                &editor,
                binary_command(edit, operation.clone().ok_or(FormError::Backend)?),
                cx,
            )
            .map(Some)
        }),
        window,
        cx,
    );
}

/// Export only an unambiguous, locally available attachment selected by the user.
pub(in crate::macos::ui) fn export_attachment(
    store: Entity<WorkspaceStore>,
    attachment: taypeer_core::AttachmentId,
    revision: Option<RevisionId>,
    window: &mut Window,
    cx: &mut App,
) {
    let state = store.read(cx);
    let (Some(connection), Some(entry)) =
        (state.connection().cloned(), state.state().selected.clone())
    else {
        return;
    };
    let target = revision.map_or_else(
        || taypeer_services::BinaryTarget::Entry(entry.clone()),
        |revision| taypeer_services::BinaryTarget::Revision {
            entry: entry.clone(),
            revision,
        },
    );
    let ticket = connection.command::<taypeer_services::BinaryView>(
        taypeer_runtime::Command::BinaryView(target.clone()),
    );
    let weak = store.downgrade();
    store.update(cx, |store, _| {
        store.watch(ticket, move |store, result, window, cx| {
            let row = result.ok().and_then(|view| {
                view.attachments
                    .into_iter()
                    .find(|row| row.id == attachment)
            });
            let Some(row) = row.filter(|r| {
                !r.deletion_conflict
                    && r.names.len() == 1
                    && r.contents.len() == 1
                    && r.contents[0].bytes.is_some()
            }) else {
                store.set_notice("ui.attachment_unavailable", cx);
                return;
            };
            let blob = row.contents[0].id.clone();
            let prompt = cx
                .prompt_for_new_path(&std::env::temp_dir(), row.names.first().map(String::as_str));
            let handle = window.window_handle();
            let connection = connection.clone();
            let target = target.clone();
            let weak = weak.clone();
            cx.spawn(async move |_, cx| {
                let selected = prompt.await;
                let _ = handle.update(cx, |_, _, cx| {
                    if let Ok(Ok(Some(path))) = selected {
                        let _ = weak.update(cx, |store, _| {
                            if !connection.control.is_open() {
                                return;
                            }
                            let ticket =
                                connection.command::<()>(taypeer_runtime::Command::ExportBinary {
                                    target,
                                    blob,
                                    path,
                                    overwrite: false,
                                });
                            store.watch(ticket, |store, result, _, cx| {
                                store.set_notice(
                                    if result.is_ok() {
                                        "ui.exported"
                                    } else {
                                        "ui.export_failed"
                                    },
                                    cx,
                                )
                            });
                        });
                    }
                });
            })
            .detach();
        })
    });
    let _ = window;
}

pub(in crate::macos::ui) fn image_file(
    editor: Entity<EditorStore>,
    window: &mut Window,
    cx: &mut App,
) {
    let prompt = cx.prompt_for_paths(PathPromptOptions {
        files: true,
        directories: false,
        multiple: false,
        prompt: None,
    });
    let handle = window.window_handle();
    cx.spawn(async move |cx| {
        let selected = prompt.await;
        let _ = handle.update(cx, |_, _, cx| {
            if let Ok(Ok(Some(paths))) = selected
                && let Some(path) = paths.into_iter().next()
            {
                editor.update(cx, |editor, cx| {
                    if editor.connection().control.is_open() {
                        editor.binary(taypeer_services::BinaryEdit::Icon(
                            taypeer_services::IconInput::File(path),
                        ));
                        cx.notify();
                    }
                });
            }
        });
    })
    .detach();
}
