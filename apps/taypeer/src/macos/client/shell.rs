//! Window chrome, unlock screen and resizable workspace composition.

use super::{Client, Form, Navigation};
use crate::macos::common::tr;
use gpui_kit::component::input::{Input, InputContentType};
use gpui_kit::component::{button::*, *};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

impl Client {
    pub(super) fn header(&self, cx: &mut Context<Self>) -> AnyElement {
        h_flex()
            .h_11()
            .px_3()
            .gap_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(div().font_weight(FontWeight::SEMIBOLD).child("Taypeer"))
            .children(self.service.databases().into_iter().map(|db| {
                let id = db.id.clone();
                Button::new(SharedString::from(format!("database-{}", db.id.as_str())))
                    .label(db.name)
                    .selected(self.database.as_ref() == Some(&db.id))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.navigate(Navigation::Database(id.clone()), window, cx)
                    }))
            }))
            .child(Button::new("create-db").label(tr("create_db")).on_click(
                cx.listener(|this, _, window, cx| this.open_form(Form::Database, "", window, cx)),
            ))
            .child(
                Button::new("open-db")
                    .label(tr("open_db"))
                    .on_click(cx.listener(|this, _, window, cx| this.choose_file(window, cx))),
            )
            .child(div().flex_1())
            .when(self.session.is_some(), |el| {
                el.child(
                    Button::new("lock")
                        .label(tr("lock"))
                        .on_click(cx.listener(|this, _, window, cx| this.lock(window, cx))),
                )
            })
            .child(
                Button::new("settings")
                    .icon(IconName::Settings)
                    .tooltip(tr("settings"))
                    .accessibility_label(tr("settings"))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.settings = !this.settings;
                        if this.settings {
                            this.modal_focus.focus(window, cx);
                        }
                        cx.notify();
                    })),
            )
            .into_any_element()
    }

    pub(super) fn unlock_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_4()
            .child(div().text_xl().font_weight(FontWeight::SEMIBOLD).child(tr(
                if self.database.is_some() {
                    "locked"
                } else {
                    "welcome"
                },
            )))
            .when(self.database.is_some(), |el| {
                el.child(
                    div().text_color(cx.theme().muted_foreground).child(
                        if self
                            .database
                            .as_ref()
                            .is_some_and(|id| self.service.is_file(id))
                        {
                            tr("file_password_hint")
                        } else {
                            rust_i18n::t!(
                                "password_hint",
                                password = taypeer_services::DEMO_PASSWORD
                            )
                            .to_string()
                            .into()
                        },
                    ),
                )
                .child(
                    div().w_80().child(
                        Input::new(&self.password)
                            .aria_label(tr("password"))
                            .content_type(InputContentType::Password)
                            .mask_toggle(),
                    ),
                )
                .child(
                    Button::new("unlock")
                        .primary()
                        .label(tr("unlock"))
                        .on_click(cx.listener(|this, _, window, cx| this.unlock(window, cx))),
                )
            })
            .when(self.database.is_none(), |el| {
                el.child(
                    Button::new("welcome-create")
                        .label(tr("create_db"))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_form(Form::Database, "", window, cx)
                        })),
                )
            })
            .into_any_element()
    }

    pub(super) fn workspace(&self, cx: &mut Context<Self>) -> AnyElement {
        let inspector = self.selected.is_some() || self.editor.is_some();
        let scale = self.prefs.font_size as f32 / 16.;
        let group = self.groups_panel(cx);
        let entries = self.entries_panel(cx);
        let mut panels = h_resizable("workspace-panels")
            .child(
                resizable_panel()
                    .size(px(self.prefs.group_width * scale))
                    .size_range(px(192.)..px(280.))
                    .child(group),
            )
            .child(
                resizable_panel()
                    .size(px(self.prefs.entry_width * scale))
                    .size_range(px(380.)..px(if inspector { 560. } else { 4000. }))
                    .child(entries),
            );
        if inspector {
            panels = panels.child(
                resizable_panel()
                    .size_range(px(480.)..px(4000.))
                    .child(self.inspector(cx)),
            );
        }
        panels
            .on_resize(cx.listener(|this, state: &Entity<ResizableState>, _, cx| {
                let sizes = state.read(cx).sizes();
                let scale = this.prefs.font_size as f32 / 16.;
                if let Some(width) = sizes.first() {
                    this.prefs.group_width = (f32::from(*width) / scale).clamp(192., 280.);
                }
                if (this.selected.is_some() || this.editor.is_some())
                    && let Some(width) = sizes.get(1)
                {
                    this.prefs.entry_width = (f32::from(*width) / scale).clamp(380., 560.);
                }
                this.save_preferences();
            }))
            .into_any_element()
    }
}
