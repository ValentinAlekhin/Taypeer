//! File selection, creation and database metadata forms.
use super::*;

pub(in crate::macos::ui) fn choose_file(
    store: Entity<WorkspaceStore>,
    window: &mut Window,
    cx: &mut App,
) {
    let selected = cx.prompt_for_paths(PathPromptOptions {
        files: true,
        directories: false,
        multiple: false,
        prompt: Some(tr("ui.open_file")),
    });
    let handle = window.window_handle();
    cx.spawn(async move |cx| {
        let result = selected.await;
        let _ = handle.update(cx, |_, window, cx| match result {
            Ok(Ok(Some(paths))) => {
                if let Some(path) = paths.into_iter().next() {
                    store.update(cx, |s, cx| s.select_path(path, window, cx));
                }
            }
            Ok(Ok(None)) => {}
            _ => store.update(cx, |s, cx| s.set_notice("ui.operation_failed", cx)),
        });
    })
    .detach();
}

pub(in crate::macos::ui) fn database(
    store: Entity<WorkspaceStore>,
    id: Option<DatabaseId>,
    window: &mut Window,
    cx: &mut App,
) {
    let connection = store.read(cx).connection().cloned();
    if id.is_some()
        && id
            .as_ref()
            .and_then(|id| store.read(cx).catalog().read(cx).database(id))
            .is_none_or(|db| db.metadata_conflict || !db.writable)
    {
        return;
    }
    let (name, description) = id
        .as_ref()
        .and_then(|id| store.read(cx).catalog().read(cx).database(id))
        .map(|db| (db.name.clone(), db.description.clone()))
        .unwrap_or_default();
    let mut fields = vec![
        ("name", name, false),
        (
            "ui.description",
            description.clone().unwrap_or_default(),
            false,
        ),
    ];
    if id.is_none() {
        fields.extend([
            ("password", String::new(), true),
            ("confirm_password", String::new(), true),
            ("ui.kdf_seconds", "1.0".into(), false),
        ]);
    }
    text_form(
        if id.is_some() {
            "ui.database_info"
        } else {
            "create_db"
        },
        fields,
        Box::new(move |values, _, _| {
            require_name(&values[0])?;
            let name = values[0].clone();
            let description = if values[1] == description.as_deref().unwrap_or_default() {
                description.clone()
            } else {
                Some(values[1].clone())
            };
            let store = store.clone();
            if id.is_some() {
                let connection = connection.clone().ok_or(FormError::MissingObject)?;
                return Ok(Some(Box::new(move |done, _, cx| {
                    store.update(cx, |s, _| {
                        s.watch(
                            connection.command::<()>(taypeer_runtime::Command::SetDatabaseInfo {
                                name,
                                description,
                            }),
                            move |s, result, window, cx| {
                                let result = result.map_err(FormError::Runtime);
                                if result.is_ok() {
                                    s.refresh(cx);
                                }
                                done(result, window, cx);
                            },
                        );
                    });
                })));
            }
            if values[2].is_empty() || values[2] != values[3] {
                return Err(FormError::PasswordConfirmation);
            }
            let seconds = values[4]
                .parse::<f64>()
                .map_err(|_| FormError::InvalidNumber)?;
            if !seconds.is_finite() {
                return Err(FormError::InvalidNumber);
            }
            let defaults = taypeer_core::DatabasePolicy::default();
            let policy = taypeer_core::DatabasePolicy::new(
                defaults.attachment_bytes(),
                defaults.total_attachment_bytes(),
                (seconds * 1000.).round() as u32,
            )
            .map_err(|_| FormError::InvalidNumber)?;
            let password = zeroize::Zeroizing::new(values[2].clone());
            Ok(Some(Box::new(move |done, window, cx| {
                let epoch = store.read(cx).secret_epoch();
                let prompt =
                    cx.prompt_for_new_path(std::path::Path::new("."), Some("database.taypeer"));
                let handle = window.window_handle();
                cx.spawn(async move |cx| {
                    let selected = prompt.await;
                    let _ = handle.update(cx, |_, window, cx| match selected {
                        Ok(Ok(Some(path))) => store.update(cx, |s, cx| {
                            if s.secret_epoch() != epoch {
                                done(Err(FormError::Canceled), window, cx);
                                return;
                            }
                            s.open_file(
                                path,
                                password.to_string(),
                                Some(taypeer_services::CreateDatabase {
                                    name,
                                    description,
                                    policy,
                                }),
                                move |result, window, cx| {
                                    done(result.map_err(FormError::Runtime), window, cx)
                                },
                                window,
                                cx,
                            )
                        }),
                        Ok(Ok(None)) => done(Err(FormError::Canceled), window, cx),
                        _ => done(Err(FormError::Backend), window, cx),
                    });
                })
                .detach();
            })))
        }),
        window,
        cx,
    );
}
