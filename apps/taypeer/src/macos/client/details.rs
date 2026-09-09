//! Read-only entry/revision contents and explicitly revealed values.

use super::{Client, EntryTab};
use crate::macos::common::{field_row, format_date, tr};
use gpui_kit::component::{button::*, *};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use taypeer_services::{AttributeId, EntryView, RevisionId};

impl Client {
    fn reveal(&mut self, attribute: Option<AttributeId>, cx: &mut Context<Self>) {
        let key = attribute
            .as_ref()
            .map(|id| id.as_str().to_string())
            .unwrap_or_else(|| "password".into());
        if self.revealed.remove(&key).is_some() {
            cx.notify();
            return;
        }
        let (Some(token), Some(entry)) = (&self.session, &self.selected) else {
            return;
        };
        let result = match (&self.revision, attribute) {
            (Some(revision), Some(attr)) => self
                .service
                .reveal_revision_attribute(token, entry, revision, &attr),
            (Some(revision), None) => self
                .service
                .reveal_revision_password(token, entry, revision),
            (None, Some(attr)) => self.service.reveal_attribute(token, entry, &attr),
            (None, None) => self.service.reveal_password(token, entry),
        };
        match result {
            Ok(reply) if self.accepts(&reply.session) => {
                self.revealed
                    .insert(key, reply.value.expose().to_string().into());
            }
            _ => self.error = Some("error"),
        }
        cx.notify();
    }

    pub(super) fn detail_content(&self, view: &EntryView, cx: &mut Context<Self>) -> AnyElement {
        if self.tab == EntryTab::Attributes {
            return v_flex()
                .children(view.attributes.iter().map(|attr| {
                    let id = attr.id.clone();
                    let key = id.as_str().to_string();
                    h_flex()
                        .min_h_11()
                        .gap_4()
                        .border_b_1()
                        .border_color(cx.theme().border)
                        .child(div().w(rems(9.)).child(attr.name.clone()))
                        .child(div().flex_1().child(if attr.protected {
                            self.revealed
                                .get(&key)
                                .cloned()
                                .unwrap_or_else(|| "••••••••".into())
                        } else {
                            attr.value.clone().unwrap_or_default().into()
                        }))
                        .when(attr.protected, |el| {
                            el.child(
                                Button::new(SharedString::from(format!("reveal-{key}")))
                                    .label(tr(if self.revealed.contains_key(&key) {
                                        "hide"
                                    } else {
                                        "show"
                                    }))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.reveal(Some(id.clone()), cx)
                                    })),
                            )
                        })
                        .into_any_element()
                }))
                .into_any_element();
        }
        if self.tab == EntryTab::History && self.revision.is_none() {
            let history = self.entry_history();
            return v_flex()
                .gap_2()
                .children(history.into_iter().map(|revision| {
                    let id = revision.id.clone();
                    Button::new(SharedString::from(format!("revision-{}", id.as_str())))
                        .label(format!(
                            "{} · {}",
                            revision.title,
                            format_date(revision.saved_at)
                        ))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.show_revision(id.clone(), window, cx);
                        }))
                }))
                .into_any_element();
        }
        v_flex()
            .child(field_row("title", view.title.clone(), cx))
            .child(field_row(
                "username",
                view.username
                    .clone()
                    .unwrap_or_else(|| tr("absent").to_string()),
                cx,
            ))
            .child(field_row(
                "password",
                h_flex()
                    .gap_2()
                    .child(div().flex_1().child(if view.has_password {
                        self.revealed
                            .get("password")
                            .cloned()
                            .unwrap_or_else(|| "••••••••".into())
                    } else {
                        tr("absent")
                    }))
                    .when(view.has_password, |el| {
                        el.child(
                            Button::new("reveal-password")
                                .label(tr(if self.revealed.contains_key("password") {
                                    "hide"
                                } else {
                                    "show"
                                }))
                                .on_click(cx.listener(|this, _, _, cx| this.reveal(None, cx))),
                        )
                    }),
                cx,
            ))
            .child(field_row(
                "url",
                view.url.clone().unwrap_or_else(|| tr("absent").to_string()),
                cx,
            ))
            .child(field_row(
                "notes",
                view.notes
                    .clone()
                    .unwrap_or_else(|| tr("absent").to_string()),
                cx,
            ))
            .child(field_row("tags", view.tags.join("\n"), cx))
            .child(field_row(
                "expires",
                view.expires_at
                    .map(format_date)
                    .unwrap_or_else(|| tr("absent").to_string()),
                cx,
            ))
            .into_any_element()
    }

    fn show_revision(&mut self, id: RevisionId, window: &mut Window, cx: &mut Context<Self>) {
        self.root_focus.focus(window, cx);
        self.revision = Some(id.clone());
        self.revealed.clear();
        self.tab = EntryTab::Overview;
        cx.notify();
    }
}
