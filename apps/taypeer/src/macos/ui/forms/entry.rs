//! Attribute form and acknowledgment of auxiliary draft commands.
use super::*;

pub(in crate::macos::ui) fn attribute(
    editor: Entity<EditorStore>,
    index: Option<usize>,
    window: &mut Window,
    cx: &mut App,
) {
    if !editor.read(cx).editable() || editor.read(cx).busy() {
        return;
    }
    let attribute = index
        .and_then(|index| editor.read(cx).content().attributes.get(index))
        .cloned();
    let original = zeroize::Zeroizing::new(
        attribute
            .as_ref()
            .map(|a| a.value.clone())
            .unwrap_or_default(),
    );
    let id = attribute.as_ref().and_then(|a| a.id.clone());
    let protected = attribute.as_ref().is_some_and(|a| a.protected);
    text_form(
        "ui.attribute",
        vec![
            (
                "ui.key",
                attribute
                    .as_ref()
                    .map(|a| a.key.clone())
                    .unwrap_or_default(),
                false,
            ),
            ("value", original.to_string(), protected),
        ],
        Box::new(move |values, _, cx| {
            require_name(&values[0])?;
            if editor
                .read(cx)
                .content()
                .attributes
                .iter()
                .any(|a| a.id != id && a.key == values[0])
            {
                return Err(FormError::DuplicateAttribute);
            }
            let patch = taypeer_services::AttributePatch {
                id: id.clone(),
                name: values[0].clone(),
                value: if values[1] == *original && id.is_some() {
                    taypeer_services::FieldUpdate::Keep
                } else {
                    taypeer_services::FieldUpdate::Set(values[1].clone())
                },
                protected,
            };
            submit_editor(
                &editor,
                taypeer_runtime::Command::PatchAttribute {
                    patch,
                    remove: false,
                },
                cx,
            )
            .map(Some)
        }),
        window,
        cx,
    );
}

pub(super) fn submit_editor(
    editor: &Entity<EditorStore>,
    command: taypeer_runtime::Command,
    cx: &mut App,
) -> Result<Deferred, FormError> {
    if !editor.read(cx).editable() || editor.read(cx).busy() {
        return Err(FormError::MissingObject);
    }
    let control = editor.read(cx).connection().control.clone();
    let ticket = editor.update(cx, |editor, cx| {
        let ticket = editor.form_command(command);
        cx.notify();
        ticket
    });
    let editor = editor.downgrade();
    Ok(Box::new(move |done, window, cx| {
        let handle = window.window_handle();
        cx.spawn(async move |cx| {
            let result = cx
                .background_executor()
                .spawn(async move { ticket.wait() })
                .await;
            let _ = handle.update(cx, |_, window, cx| {
                let result = if control.is_open() {
                    result
                } else {
                    Err(taypeer_runtime::RuntimeError::Closed)
                };
                let _ = editor.update(cx, |editor, cx| {
                    editor.finish_form(&result);
                    cx.notify();
                });
                done(result.map_err(FormError::Runtime), window, cx);
            });
        })
        .detach();
    }))
}
