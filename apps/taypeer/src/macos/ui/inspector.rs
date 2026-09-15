//! Masked details and lazy historical projections; reveal is a separate service command.
pub(super) use super::generator::open_generator;
use super::{editor::EditorView, style::*, workspace::WorkspaceStore};
use crate::ui_state::*;
use gpui_kit::{
    component::{button::*, *},
    prelude::FluentBuilder,
    *,
};
use std::collections::BTreeMap;
use taypeer_runtime::Command;
use zeroize::Zeroizing;

pub(super) struct Inspector {
    store: Entity<WorkspaceStore>,
    fields: Option<(EntityId, Entity<EditorView>)>,
    identity: Option<(DatabaseId, EntryId)>,
    revealed: BTreeMap<String, Zeroizing<String>>,
    revision: Option<RevisionId>,
    snapshot: Option<RevisionId>,
    compare: bool,
    scroll: ScrollHandle,
    _subscription: Subscription,
}
impl Inspector {
    pub fn new(store: Entity<WorkspaceStore>, cx: &mut Context<Self>) -> Self {
        Self {
            _subscription: cx.observe(&store, |this, store, cx| {
                let state = store.read(cx).state();
                if !state.is_unlocked()
                    || this.identity.as_ref().is_some_and(|(db, entry)| {
                        Some(db) != state.database.as_ref()
                            || Some(entry) != state.selected.as_ref()
                    })
                {
                    this.revealed.clear();
                    this.fields = None;
                }
                cx.notify();
            }),
            store,
            fields: None,
            identity: None,
            revealed: BTreeMap::new(),
            revision: None,
            snapshot: None,
            compare: false,
            scroll: ScrollHandle::new(),
        }
    }
    fn value(
        &self,
        key: String,
        value: &str,
        secret: bool,
        attribute: Option<taypeer_core::AttributeId>,
        revision: Option<RevisionId>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let revealed = self.revealed.get(&key);
        let shown = revealed.is_some();
        let text = if secret {
            revealed
                .map(|v| v.to_string())
                .unwrap_or_else(|| "••••••••••••".into())
        } else if value.is_empty() {
            tr("absent").to_string()
        } else {
            value.to_owned()
        };
        let ordinary = value.to_owned();
        let reveal_key = key.clone();
        let copy_key = key.clone();
        let copy_attribute = attribute.clone();
        let copy_revision = revision.clone();
        h_flex()
            .gap_2()
            .min_w_0()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .when(secret, |el| el.font_family("Menlo"))
                    .child(text),
            )
            .when(secret, |el| {
                el.child(
                    icon_button(
                        SharedString::from(format!("show-{key}")),
                        if shown { "eye-off" } else { "eye" },
                        if shown { "hide" } else { "show" },
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if this.revealed.remove(&reveal_key).is_none() {
                            this.reveal(
                                reveal_key.clone(),
                                attribute.clone(),
                                revision.clone(),
                                false,
                                cx,
                            );
                        }
                        cx.notify();
                    })),
                )
            })
            .child(
                icon_button(SharedString::from(format!("copy-{key}")), "copy", "ui.copy")
                    .disabled(!secret && value.is_empty())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if secret {
                            this.reveal(
                                copy_key.clone(),
                                copy_attribute.clone(),
                                copy_revision.clone(),
                                true,
                                cx,
                            );
                        } else if this
                            .store
                            .read(cx)
                            .connection()
                            .is_some_and(|c| c.control.is_open())
                        {
                            super::clipboard::copy(ordinary.clone(), false, cx);
                        }
                    })),
            )
            .into_any_element()
    }
    fn reveal(
        &mut self,
        key: String,
        attribute: Option<taypeer_core::AttributeId>,
        revision: Option<RevisionId>,
        copy: bool,
        cx: &mut Context<Self>,
    ) {
        let Some((db, entry)) = self.identity.clone() else {
            return;
        };
        let Some(connection) = self.store.read(cx).connection().cloned() else {
            return;
        };
        let command = if let Some(revision) = revision {
            Command::RevealRevision {
                entry,
                revision,
                attribute,
            }
        } else if let Some(attribute) = attribute {
            Command::RevealAttribute { entry, attribute }
        } else {
            Command::RevealPassword(entry)
        };
        let ticket = connection.command::<Zeroizing<String>>(command);
        let target = cx.entity().downgrade();
        let identity = self.identity.clone();
        let control = connection.control.clone();
        self.store.update(cx, |store, _| {
            store.watch(ticket, move |store, result, _, cx| {
                if !control.is_open() {
                    return;
                }
                match result {
                    Ok(value) => {
                        let _ = target.update(cx, |this, cx| {
                            if this.identity == identity
                                && this.identity.as_ref().is_some_and(|(id, _)| id == &db)
                            {
                                if copy {
                                    super::clipboard::copy(value.to_string(), true, cx);
                                } else {
                                    this.revealed.insert(key, value);
                                }
                                cx.notify();
                            }
                        });
                    }
                    Err(error) => store.set_notice(super::workspace::error_key(&error), cx),
                }
            })
        });
    }
    fn overview(
        &self,
        content: &EntryContent,
        revision: Option<RevisionId>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        v_flex()
            .children(EntryField::ALL.map(|field| {
                row(
                    field.key(),
                    self.value(
                        format!("{revision:?}-{field:?}"),
                        field.value(content),
                        field == EntryField::Password && content.has_password,
                        None,
                        revision.clone(),
                        cx,
                    ),
                    cx,
                )
            }))
            .into_any_element()
    }
    fn advanced(
        &self,
        content: &EntryContent,
        revision: Option<RevisionId>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut result = v_flex().child(section("attributes"));
        for attribute in &content.attributes {
            result = result.child(row(
                &attribute.key,
                self.value(
                    format!("{revision:?}-{:?}", attribute.id),
                    &attribute.value,
                    attribute.protected,
                    attribute.id.clone(),
                    revision.clone(),
                    cx,
                ),
                cx,
            ));
        }
        result = result.child(section("ui.attachments"));
        for attachment in &content.attachments {
            let id = attachment.id.clone();
            let revision = revision.clone();
            result = result.child(
                h_flex()
                    .min_h(rems(2.75))
                    .px_6()
                    .gap_3()
                    .child(icon("file"))
                    .child(div().flex_1().child(attachment.name.clone()))
                    .child(
                        icon_button(
                            SharedString::from(format!("export-{}", attachment.id.as_str())),
                            "download",
                            "ui.download",
                        )
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                super::forms::export_attachment(
                                    this.store.clone(),
                                    id.clone(),
                                    revision.clone(),
                                    window,
                                    cx,
                                )
                            },
                        )),
                    ),
            );
        }
        result.into_any_element()
    }
    fn appearance(&self, content: &EntryContent, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .child(row(
                "ui.icon",
                super::images::stored_icon(&content.icon, content.icon_blob.as_ref(), cx),
                cx,
            ))
            .child(row(
                "ui.foreground_color",
                content
                    .foreground
                    .map(|c| format!("#{c:06X}"))
                    .unwrap_or_else(|| tr("ui.default_color").to_string()),
                cx,
            ))
            .child(row(
                "ui.background_color",
                content
                    .background
                    .map(|c| format!("#{c:06X}"))
                    .unwrap_or_else(|| tr("ui.default_color").to_string()),
                cx,
            ))
            .child(section("ui.preview"))
            .child(
                div()
                    .mx_6()
                    .p_4()
                    .bg(content
                        .background
                        .map(|c| rgb(c).into())
                        .unwrap_or(cx.theme().background))
                    .text_color(
                        content
                            .foreground
                            .map(|c| rgb(c).into())
                            .unwrap_or(cx.theme().foreground),
                    )
                    .child(content.title.clone()),
            )
            .into_any_element()
    }
    fn history(&self, entry: &Entry, cx: &mut Context<Self>) -> AnyElement {
        let mut result = v_flex().child(
            h_flex()
                .px_6()
                .py_3()
                .child(div().flex_1().child(tr("history")))
                .child(
                    icon_button("clear-history", "trash-2", "ui.clear_history")
                        .disabled(!self.store.read(cx).writable(cx))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.store.update(cx, |s, cx| {
                                s.navigate(Destination::ClearHistory, window, cx)
                            })
                        })),
                ),
        );
        for version in entry.revisions.iter().rev() {
            let id = version.sequence.clone();
            result = result.child(
                Button::new(SharedString::from(format!("version-{}", id.as_str())))
                    .ghost()
                    .justify_start()
                    .h(rems(2.75))
                    .selected(self.revision.as_ref() == Some(&id))
                    .label(format!("{}  {}", stamp(version.saved_at), version.title))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.revision = Some(id.clone());
                        this.compare = false;
                        this.revealed.clear();
                        this.store
                            .update(cx, |s, cx| s.load_revision(id.clone(), cx));
                        cx.notify();
                    })),
            );
        }
        let selected = self.revision.clone();
        result = result.child(
            h_flex()
                .px_6()
                .py_4()
                .gap_2()
                .child(
                    Button::new("view-version")
                        .label(tr("ui.view_revision"))
                        .disabled(selected.is_none())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.snapshot = this.revision.clone();
                            this.revealed.clear();
                            this.store
                                .update(cx, |s, cx| s.select_tab(EntryTab::Overview, cx));
                        })),
                )
                .child(
                    Button::new("compare")
                        .label(tr("ui.compare"))
                        .disabled(selected.is_none())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.compare = !this.compare;
                            this.revealed.clear();
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("restore-version")
                        .label(tr("ui.restore_revision"))
                        .disabled(selected.is_none() || !self.store.read(cx).writable(cx))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if let Some(id) = selected.clone() {
                                this.store.update(cx, |s, cx| {
                                    s.navigate(Destination::RestoreRevision(id), window, cx)
                                });
                            }
                        })),
                ),
        );
        if self.compare
            && let Some(version) = entry
                .revisions
                .iter()
                .find(|r| Some(&r.sequence) == self.revision.as_ref())
        {
            if let Some(old) = &version.content {
                result = result
                    .child(section("ui.previous"))
                    .child(self.overview(old, Some(version.sequence.clone()), cx))
                    .child(self.advanced(old, Some(version.sequence.clone()), cx))
                    .child(self.appearance(old, cx))
                    .child(section("ui.current"))
                    .child(self.overview(&entry.content, None, cx))
                    .child(self.advanced(&entry.content, None, cx))
                    .child(self.appearance(&entry.content, cx));
            } else {
                result = result.child(empty("ui.loading", cx));
            }
        }
        result
            .when(entry.revisions.is_empty(), |el| {
                el.child(empty("ui.no_history", cx))
            })
            .into_any_element()
    }
}
impl Render for Inspector {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let store = self.store.read(cx);
        let identity = store
            .state()
            .database
            .clone()
            .zip(store.state().selected.clone());
        if self.identity != identity {
            self.identity = identity.clone();
            self.revealed.clear();
            self.snapshot = None;
            self.revision = None;
            self.compare = false;
        }
        let editor = store.editor().cloned();
        if let Some(editor) = &editor {
            if self
                .fields
                .as_ref()
                .is_none_or(|(id, _)| *id != editor.entity_id())
            {
                self.fields = Some((
                    editor.entity_id(),
                    cx.new(|cx| EditorView::new(self.store.clone(), editor.clone(), window, cx)),
                ));
                self.snapshot = None;
                self.revealed.clear();
            }
        } else {
            self.fields = None;
        }
        let store = self.store.read(cx);
        let entry = identity
            .as_ref()
            .and_then(|(db, id)| store.catalog().read(cx).entry(db, id))
            .cloned();
        let tab = store.state().tab;
        let writable = store.writable(cx);
        let busy = store.busy();
        if self.revision.as_ref().is_some_and(|id| {
            entry
                .as_ref()
                .is_none_or(|e| !e.revisions.iter().any(|r| &r.sequence == id))
        }) {
            self.revision = None;
            self.snapshot = None;
            self.revealed.clear();
        }
        let editing = editor.is_some();
        let title = entry
            .as_ref()
            .map(|e| e.content.title.clone())
            .unwrap_or_else(|| tr("new_title").to_string());
        let body = if editing
            && matches!(
                tab,
                EntryTab::Overview | EntryTab::Advanced | EntryTab::Appearance
            ) {
            self.fields
                .as_ref()
                .expect("editor fields created")
                .1
                .clone()
                .into_any_element()
        } else if let Some(entry) = &entry {
            let shown = match &self.snapshot {
                Some(id) => entry
                    .revisions
                    .iter()
                    .find(|r| &r.sequence == id)
                    .and_then(|r| r.content.as_ref()),
                None => Some(&entry.content),
            };
            match tab {
                EntryTab::History => self.history(entry, cx),
                EntryTab::Properties => v_flex()
                    .child(row("ui.created", stamp(entry.created), cx))
                    .child(row("ui.modified", stamp(entry.modified), cx))
                    .child(row("ui.id", entry.id.as_str().to_owned(), cx))
                    .into_any_element(),
                _ => match shown {
                    None => empty("ui.loading", cx),
                    Some(content) => match tab {
                        EntryTab::Overview => self.overview(content, self.snapshot.clone(), cx),
                        EntryTab::Advanced => self.advanced(content, self.snapshot.clone(), cx),
                        _ => self.appearance(content, cx),
                    },
                },
            }
        } else {
            empty("ui.loading", cx)
        };
        v_flex()
            .size_full()
            .border_l_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .h(rems(2.75))
                    .px_6()
                    .gap_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .flex_1()
                            .truncate()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title),
                    )
                    .when(editing, |el| {
                        el.child(
                            icon_button("cancel-edit", "x", "cancel")
                                .disabled(busy)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.store.update(cx, |s, cx| {
                                        s.navigate(Destination::CancelEdit, window, cx)
                                    })
                                })),
                        )
                        .child(
                            icon_button("save-entry", "check", "save")
                                .disabled(busy)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.store.update(cx, |s, cx| s.save(cx))
                                })),
                        )
                    })
                    .when(!editing && self.snapshot.is_none(), |el| {
                        el.child(
                            icon_button("edit-entry", "pencil", "edit")
                                .disabled(!writable || entry.as_ref().is_some_and(|e| e.conflicted))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.store.update(cx, |s, cx| s.begin_edit(cx))
                                })),
                        )
                    }),
            )
            .when(self.snapshot.is_some(), |el| {
                el.child(
                    Button::new("current-entry")
                        .ghost()
                        .label(tr("back"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.snapshot = None;
                            this.revealed.clear();
                            cx.notify();
                        })),
                )
            })
            .when(entry.as_ref().is_some_and(|e| e.conflicted), |el| {
                el.child(empty("ui.conflict", cx))
            })
            .child(
                h_flex()
                    .h(rems(2.375))
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .children(EntryTab::ALL.map(|tab| {
                        let selected = self.store.read(cx).state().tab == tab;
                        Button::new(tab.key())
                            .ghost()
                            .compact()
                            .rounded_none()
                            .h_full()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .label(tr(tab.key()))
                            .tooltip(tr(tab.key()))
                            .when(selected, |el| {
                                el.border_b_2().border_color(cx.theme().foreground)
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.store.update(cx, |s, cx| s.select_tab(tab, cx))
                            }))
                    })),
            )
            .child(
                div()
                    .id("inspector-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .child(body),
            )
    }
}
