//! Authenticated database settings, separate from device preferences.
use super::{forms, style::*, workspace::WorkspaceStore};
use crate::ui_state::*;
use gpui_kit::{
    component::{button::*, *},
    prelude::FluentBuilder,
    *,
};
/// Database-owned settings slot embedded by the device settings view.
pub struct DatabaseSettings {
    store: Entity<WorkspaceStore>,
    _subscription: Subscription,
}
impl DatabaseSettings {
    /// Observe database metadata and policy for this slot lifetime.
    pub fn new(store: Entity<WorkspaceStore>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&store, |_, _, cx| cx.notify());
        Self {
            store,
            _subscription: subscription,
        }
    }
    fn database(&self, cx: &mut Context<Self>) -> AnyElement {
        let state = self.store.read(cx);
        if !state.state().is_unlocked() {
            return empty("ui.settings_locked", cx);
        }
        let Some(id) = state.state().database.as_ref() else {
            return empty("ui.settings_locked", cx);
        };
        let Some(database) = state.catalog().read(cx).database(id) else {
            return empty("ui.settings_locked", cx);
        };
        let id = id.clone();
        let policy = database.policy;
        v_flex()
            .gap_3()
            .p_6()
            .max_w(rems(58.))
            .child(row("name", database.name.clone(), cx))
            .child(row(
                "ui.description",
                database.description.clone().unwrap_or_default(),
                cx,
            ))
            .when(database.metadata_conflict, |el| {
                el.child(
                    div()
                        .px_6()
                        .text_color(cx.theme().danger)
                        .child(tr("ui.metadata_conflict")),
                )
            })
            .child(row(
                "ui.storage",
                database.path.to_string_lossy().into_owned(),
                cx,
            ))
            .child(row(
                "ui.managing_device",
                tr(if database.managing {
                    "ui.this_device"
                } else {
                    "ui.another_device"
                }),
                cx,
            ))
            .child(
                h_flex()
                    .px_6()
                    .gap_2()
                    .child(
                        Button::new("database-info")
                            .label(tr("ui.database_info"))
                            .disabled(!database.writable || database.metadata_conflict)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                forms::database(this.store.clone(), Some(id.clone()), window, cx)
                            })),
                    )
                    .child(
                        Button::new("protection")
                            .label(tr("ui.protection"))
                            .disabled(!database.managing)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.protection(window, cx)),
                            ),
                    ),
            )
            .child(row(
                "ui.kdf_seconds",
                format!("{:.3}", f64::from(policy.kdf_target_ms()) / 1000.),
                cx,
            ))
            .child(row(
                "ui.attachment_limit",
                format!("{} MiB", policy.attachment_bytes() / (1024 * 1024)),
                cx,
            ))
            .child(row(
                "ui.total_attachment_limit",
                format!("{} MiB", policy.total_attachment_bytes() / (1024 * 1024)),
                cx,
            ))
            .into_any_element()
    }
    fn protection(&self, window: &mut Window, cx: &mut Context<Self>) {
        let state = self.store.read(cx);
        let Some(connection) = state.connection().cloned() else {
            return;
        };
        let Some(database) = state.catalog().read(cx).database(&connection.database) else {
            return;
        };
        let policy = database.policy;
        let Ok(operation) = taypeer_services::new_operation_id()
            .map(|id| taypeer_trust::Digest::of(id.as_str().as_bytes()))
        else {
            return;
        };
        let store = self.store.clone();
        forms::text_form(
            "ui.protection",
            vec![
                (
                    "ui.kdf_seconds",
                    format!("{:.3}", f64::from(policy.kdf_target_ms()) / 1000.),
                    false,
                ),
                (
                    "ui.attachment_limit",
                    (policy.attachment_bytes() / (1024 * 1024)).to_string(),
                    false,
                ),
                (
                    "ui.total_attachment_limit",
                    (policy.total_attachment_bytes() / (1024 * 1024)).to_string(),
                    false,
                ),
                ("password", String::new(), true),
            ],
            Box::new(move |values, _, _| {
                let seconds: f64 = values[0].parse().map_err(|_| FormError::InvalidNumber)?;
                if !seconds.is_finite() || !(0.5..=5.).contains(&seconds) {
                    return Err(FormError::InvalidNumber);
                }
                let bytes = |value: &str| {
                    value
                        .parse::<u64>()
                        .ok()
                        .and_then(|n| n.checked_mul(1024 * 1024))
                        .ok_or(FormError::InvalidNumber)
                };
                let updated = taypeer_core::DatabasePolicy::new(
                    bytes(&values[1])?,
                    bytes(&values[2])?,
                    (seconds * 1000.).round() as u32,
                )
                .map_err(|_| FormError::InvalidNumber)?;
                let password = (policy.kdf_target_ms() != updated.kdf_target_ms())
                    .then(|| zeroize::Zeroizing::new(values[3].as_bytes().to_vec()));
                let ticket = connection.command::<serde_json::Value>(
                    taypeer_runtime::Command::SetDatabasePolicy {
                        operation,
                        policy: updated,
                        password,
                    },
                );
                let store = store.clone();
                Ok(Some(Box::new(move |done, _, cx| {
                    store.update(cx, |store, _| {
                        store.watch(ticket, move |store, result, window, cx| {
                            let result = result.map(|_| ()).map_err(FormError::Runtime);
                            if result.is_ok() {
                                store.refresh(cx);
                            }
                            done(result, window, cx);
                        })
                    });
                })))
            }),
            window,
            cx,
        );
    }
}
impl Render for DatabaseSettings {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.database(cx)
    }
}
