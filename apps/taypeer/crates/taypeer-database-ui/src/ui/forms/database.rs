//! File selection, creation and database metadata forms.
use super::*;

/// Choose a database path, then delegate opening to the feature workflow.
pub fn choose_file(store: Entity<WorkspaceStore>, window: &mut Window, cx: &mut App) {
    let selected = taypeer_ui::file_picker::open(
        cx,
        PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(tr("ui.open_file")),
        },
    );
    let handle = window.window_handle();
    cx.spawn(async move |cx| {
        let result = selected.await;
        let _ = handle.update(cx, |_, window, cx| match result {
            Ok(Some(paths)) => {
                if let Some(path) = paths.into_iter().next() {
                    store.update(cx, |s, cx| s.select_path(path, window, cx));
                }
            }
            Ok(None) => {}
            _ => store.update(cx, |s, cx| s.set_notice("ui.operation_failed", cx)),
        });
    })
    .detach();
}

pub(in crate::ui) fn database(
    store: Entity<WorkspaceStore>,
    id: Option<DatabaseId>,
    window: &mut Window,
    cx: &mut App,
) {
    if id.is_some() {
        store.update(cx, |store, cx| {
            store.open_metadata(taypeer_runtime::Command::BeginEditDatabaseInfo, cx)
        });
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
    let attempt = FormAttempt::default();
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
            let operation = attempt.operation(values)?;
            Ok(Some(Box::new(move |done, window, cx| {
                store.update(cx, |store, cx| {
                    store.create_database(
                        taypeer_services::CreateDatabase {
                            name,
                            description,
                            policy,
                        },
                        password.to_string(),
                        operation,
                        move |result, window, cx| {
                            done(result.map_err(FormError::Runtime), window, cx)
                        },
                        window,
                        cx,
                    )
                });
            })))
        }),
        window,
        cx,
    );
}
