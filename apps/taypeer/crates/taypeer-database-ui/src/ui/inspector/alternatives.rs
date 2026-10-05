//! Optional history inspection. Ordinary edits never wait for conflict resolution.
use super::*;
use taypeer_core::{EntryField as F, FieldValue};

impl Inspector {
    pub(super) fn alternatives(&mut self, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let mut body = v_flex().px_6().gap_2().child(
            Button::new("entry-alternatives")
                .ghost()
                .label(tr("ui.alternatives"))
                .on_click(cx.listener(|this, _, _, cx| this.load_alternatives(cx))),
        );
        let fields = self
            .alternatives
            .as_ref()
            .map(|view| view.fields.clone())
            .unwrap_or_default();
        for (field_index, field) in fields.into_iter().enumerate() {
            body = body.child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(tr(label(&field.field))),
            );
            for (index, variant) in field.variants.into_iter().enumerate() {
                let key = format!("alternative-{field_index}-{index}");
                let shown = self.revealed.contains_key(&key);
                let text = self
                    .revealed
                    .get(&key)
                    .map(|value| value.to_string())
                    .or_else(|| variant.value.as_ref().map(value_text))
                    .unwrap_or_else(|| "••••••••••••".into());
                let address = field.field.clone();
                let origins = variant.origins;
                let reveal_key = key.clone();
                body = body.child(
                    h_flex()
                        .gap_2()
                        .child(div().id(SharedString::from(key)).flex_1().child(text))
                        .when(field.protected, |el| {
                            el.child(
                                icon_button(
                                    SharedString::from(format!(
                                        "reveal-alternative-{field_index}-{index}"
                                    )),
                                    if shown { "eye-off" } else { "eye" },
                                    if shown { "hide" } else { "show" },
                                )
                                .on_click(cx.listener(
                                    move |this, _, window, cx| {
                                        if this.revealed.remove(&reveal_key).is_some() {
                                            this.reveal_epoch += 1;
                                            this.clear_values(window, cx);
                                        } else if let Some((_, entry)) = this.identity.clone() {
                                            this.reveal_command(
                                                reveal_key.clone(),
                                                Command::RevealConflict {
                                                    entry,
                                                    field: address.clone(),
                                                    origins: origins.clone(),
                                                },
                                                false,
                                                cx,
                                            );
                                        }
                                        cx.notify();
                                    },
                                )),
                            )
                        }),
                );
            }
        }
        body.into_any_element()
    }
    fn load_alternatives(&mut self, cx: &mut Context<Self>) {
        let Some((_, entry)) = self.identity.clone() else {
            return;
        };
        let Some(connection) = self.store.read(cx).connection().cloned() else {
            return;
        };
        let control = connection.control.clone();
        let identity = self.identity.clone();
        let target = cx.entity().downgrade();
        self.store.update(cx, |store, _| {
            store.watch(
                connection.command::<taypeer_services::ConflictView>(Command::Conflicts(entry)),
                move |store, result, window, cx| match result {
                    Ok(view) => window.defer(cx, move |_, cx| {
                        let _ = target.update(cx, |this, cx| {
                            if control.is_open()
                                && this.identity == identity
                                && this.store.read(cx).state().tab == EntryTab::History
                            {
                                this.alternatives = Some(view);
                                cx.notify();
                            }
                        });
                    }),
                    Err(error) => store.set_notice(super::super::workspace::error_key(&error), cx),
                },
            )
        });
    }
}
fn label(field: &F) -> &'static str {
    match field {
        F::Title => "name",
        F::Username => "username",
        F::Password => "password",
        F::Url => "url",
        F::Notes => "notes",
        F::Tags => "ui.tags",
        F::ExpiresAt => "ui.expires",
        F::Icon => "ui.icon",
        F::Foreground => "ui.foreground_color",
        F::Background => "ui.background_color",
        F::AttributeName(_) | F::AttributeValue(_) | F::AttributePresence(_) => "ui.attributes",
        F::AttachmentName(_) | F::AttachmentBlob(_) | F::AttachmentPresence(_) => "ui.attachments",
    }
}
fn value_text(value: &FieldValue) -> String {
    match value {
        FieldValue::Text(value) => value.clone().unwrap_or_default(),
        FieldValue::Tags(tags) => tags.iter().cloned().collect::<Vec<_>>().join("\n"),
        FieldValue::Timestamp(value) => value.map(stamp).unwrap_or_default(),
        FieldValue::Attribute(value) => value.value.clone(),
        FieldValue::Presence(value) => {
            tr(if *value { "ui.present" } else { "ui.absent" }).to_string()
        }
        FieldValue::Blob(blob) => blob.as_str().to_owned(),
        FieldValue::Icon(icon) => crate::ui_state::icon_name(icon),
        FieldValue::Color(color) => color_text(crate::ui_state::color_value(*color)),
    }
}
