//! Entry read view, historical snapshots and the common editor header.

pub(super) use super::generator::open_generator;
use super::{editor::EditorView, style::*, workspace::WorkspaceStore};
use crate::ui_state::*;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    component::{
        button::*,
        menu::{DropdownMenu, PopupMenuItem},
        *,
    },
    *,
};
use std::collections::BTreeSet;

pub(super) struct Inspector {
    store: Entity<WorkspaceStore>,
    fields: Option<(EntityId, Entity<EditorView>)>,
    identity: Option<(DatabaseId, EntryId)>,
    revealed: BTreeSet<String>,
    revision: Option<u64>,
    snapshot: Option<u64>,
    compare: bool,
    scroll: ScrollHandle,
    _subscription: Subscription,
}
impl Inspector {
    pub fn new(store: Entity<WorkspaceStore>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&store, |_, _, cx| cx.notify());
        Self {
            store,
            fields: None,
            identity: None,
            revealed: BTreeSet::new(),
            revision: None,
            snapshot: None,
            compare: false,
            scroll: ScrollHandle::new(),
            _subscription: subscription,
        }
    }
    fn value(
        &self,
        id: impl Into<String>,
        value: &str,
        secret: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = id.into();
        let reveal = self.revealed.contains(&id);
        let text = if value.is_empty() {
            tr("absent").to_string()
        } else if secret && !reveal {
            "••••••••••••".into()
        } else {
            value.to_owned()
        };
        let copy = value.to_owned();
        let copy_id = SharedString::from(format!("copy-{id}"));
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
                        SharedString::from(format!("reveal-{id}")),
                        if reveal { "eye-off" } else { "eye" },
                        if reveal { "hide" } else { "show" },
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.revealed.remove(&id) {
                            this.revealed.insert(id.clone());
                        }
                        cx.notify();
                    })),
                )
            })
            .child(
                icon_button(copy_id, "copy", "ui.copy")
                    .disabled(value.is_empty())
                    .on_click(move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))
                    }),
            )
            .into_any_element()
    }
    fn overview(&self, content: &EntryContent, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .children(EntryField::ALL.map(|field| {
                row(
                    field.key(),
                    self.value(
                        format!("field-{field:?}"),
                        field.value(content),
                        field == EntryField::Password,
                        cx,
                    ),
                    cx,
                )
            }))
            .into_any_element()
    }
    fn advanced(&self, content: &EntryContent, prefix: &str, cx: &mut Context<Self>) -> AnyElement {
        let mut result = v_flex().child(section("attributes"));
        for (index, attribute) in content.attributes.iter().enumerate() {
            result = result.child(
                h_flex()
                    .min_h(rems(2.75))
                    .px_6()
                    .gap_4()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .w(rems(9.))
                            .truncate()
                            .text_color(cx.theme().muted_foreground)
                            .child(attribute.key.clone()),
                    )
                    .child(div().flex_1().min_w_0().child(self.value(
                        format!("{prefix}-attribute-{index}"),
                        &attribute.value,
                        attribute.protected,
                        cx,
                    ))),
            );
        }
        result = result.child(section("ui.attachments"));
        for (index, attachment) in content.attachments.iter().enumerate() {
            result = result.child(
                h_flex()
                    .min_h(rems(2.75))
                    .px_6()
                    .gap_3()
                    .child(icon("file"))
                    .child(div().flex_1().truncate().child(attachment.name.clone()))
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!("{} KiB", attachment.bytes / 1024)),
                    )
                    .child(
                        icon_button(
                            SharedString::from(format!("{prefix}-download-{index}")),
                            "download",
                            "ui.download",
                        )
                        .tooltip(tr("ui.demo_attachments"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.store.update(cx, |store, cx| {
                                store.set_notice("ui.demo_download", cx);
                                cx.notify();
                            })
                        })),
                    ),
            );
        }
        result
            .when(
                content.attributes.is_empty() && content.attachments.is_empty(),
                |el| el.child(empty("ui.no_additional", cx)),
            )
            .into_any_element()
    }
    fn appearance(&self, content: &EntryContent, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .child(row(
                "ui.icon",
                h_flex()
                    .gap_3()
                    .child(icon(&content.icon))
                    .child(content.icon.clone()),
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
                    .child(
                        h_flex()
                            .gap_3()
                            .child(icon(&content.icon))
                            .child(content.title.clone()),
                    ),
            )
            .into_any_element()
    }
    fn properties(&self, entry: &Entry, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .child(row("ui.created", stamp(entry.created), cx))
            .child(row(
                "ui.modified",
                stamp(self.snapshot.unwrap_or(entry.modified)),
                cx,
            ))
            .child(row("ui.accessed", tr("absent"), cx))
            .child(row(
                "ui.id",
                self.value("entry-id", &format!("UI-{:08}", entry.id.0), false, cx),
                cx,
            ))
            .into_any_element()
    }
    fn history(&self, entry: &Entry, cx: &mut Context<Self>) -> AnyElement {
        let mut result = v_flex().child(
            h_flex()
                .px_6()
                .py_3()
                .justify_between()
                .child(tr("history"))
                .child(
                    icon_button("history-menu", "ellipsis", "ui.history_actions").dropdown_menu({
                        let store = self.store.clone();
                        move |menu, _, _| {
                            let store = store.clone();
                            menu.item(PopupMenuItem::new(tr("ui.clear_history")).on_click(
                                move |_, window, cx| {
                                    store.update(cx, |store, cx| {
                                        store.navigate(Destination::ClearHistory, window, cx)
                                    })
                                },
                            ))
                        }
                    }),
                ),
        );
        for revision in entry.revisions.iter().rev() {
            let seq = revision.sequence;
            let selected = self.revision == Some(seq);
            result = result.child(
                Button::new(("revision", seq))
                    .ghost()
                    .justify_start()
                    .h(rems(2.75))
                    .selected(selected)
                    .label(format!("{}    {}", stamp(seq), tr("revision")))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.revision = Some(seq);
                        this.compare = false;
                        cx.notify();
                    })),
            );
        }
        let selected = self.revision;
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
                            this.snapshot = this.revision;
                            this.revealed.clear();
                            this.store
                                .update(cx, |store, cx| store.select_tab(EntryTab::Overview, cx));
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
                        .disabled(selected.is_none())
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if let Some(seq) = selected {
                                this.store.update(cx, |store, cx| {
                                    store.navigate(Destination::RestoreRevision(seq), window, cx)
                                });
                            }
                        })),
                ),
        );
        if self.compare
            && let Some(revision) = entry
                .revisions
                .iter()
                .find(|r| Some(r.sequence) == selected)
        {
            result = result.child(section("ui.comparison"));
            for field in EntryField::ALL {
                let old = field.value(&revision.content);
                let current = field.value(&entry.content);
                if old != current {
                    result = result
                        .child(section(field.key()))
                        .child(row(
                            "ui.previous",
                            self.value(
                                format!("old-{field:?}"),
                                old,
                                field == EntryField::Password,
                                cx,
                            ),
                            cx,
                        ))
                        .child(row(
                            "ui.current",
                            self.value(
                                format!("current-{field:?}"),
                                current,
                                field == EntryField::Password,
                                cx,
                            ),
                            cx,
                        ));
                }
            }
            let old = &revision.content;
            let current = &entry.content;
            if old.icon != current.icon
                || old.foreground != current.foreground
                || old.background != current.background
            {
                result = result
                    .child(section("ui.appearance"))
                    .child(section("ui.previous"))
                    .child(self.appearance(old, cx))
                    .child(section("ui.current"))
                    .child(self.appearance(current, cx));
            }
            if old.attributes != current.attributes || old.attachments != current.attachments {
                result = result
                    .child(section("ui.advanced"))
                    .child(section("ui.previous"))
                    .child(self.advanced(old, "old", cx))
                    .child(section("ui.current"))
                    .child(self.advanced(current, "current", cx));
            }
            if revision.content == entry.content {
                result = result.child(empty("ui.no_changes", cx));
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
        let identity = store.state().database.zip(store.state().selected);
        if self.identity != identity {
            self.identity = identity;
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
            }
        } else {
            self.fields = None;
        }
        let store = self.store.read(cx);
        let entry = identity
            .and_then(|(db, id)| store.catalog().read(cx).entry(db, id))
            .cloned();
        if let Some(entry) = &entry
            && self
                .revision
                .is_some_and(|seq| !entry.revisions.iter().any(|r| r.sequence == seq))
        {
            self.revision = None;
            self.snapshot = None;
        }
        let snapshot = entry.as_ref().and_then(|entry| {
            entry
                .revisions
                .iter()
                .find(|r| Some(r.sequence) == self.snapshot)
        });
        let title = entry
            .as_ref()
            .map(|entry| entry.content.title.clone())
            .unwrap_or_else(|| tr("new_title").to_string());
        let tab = store.state().tab;
        let editing = editor.is_some() && snapshot.is_none();
        let content = if editing
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
            let content = snapshot.map(|s| &s.content).unwrap_or(&entry.content);
            match tab {
                EntryTab::Overview => self.overview(content, cx),
                EntryTab::Advanced => self.advanced(content, "read", cx),
                EntryTab::Appearance => self.appearance(content, cx),
                EntryTab::Properties => self.properties(entry, cx),
                EntryTab::History => self.history(entry, cx),
            }
        } else {
            empty("ui.after_save", cx)
        };
        v_flex()
            .size_full()
            .border_l_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .h(rems(2.75))
                    .flex_shrink_0()
                    .px_6()
                    .gap_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .flex_1()
                            .truncate()
                            .text_base()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title),
                    )
                    .when(editing, |el| {
                        el.child(
                            icon_button("cancel-edit", "x", "cancel").on_click(cx.listener(
                                |this, _, window, cx| {
                                    this.store.update(cx, |store, cx| {
                                        store.navigate(Destination::CancelEdit, window, cx)
                                    })
                                },
                            )),
                        )
                        .child(
                            icon_button("save-entry", "check", "save").on_click(cx.listener(
                                |this, _, _, cx| {
                                    this.store.update(cx, |store, cx| {
                                        store.save(cx);
                                    })
                                },
                            )),
                        )
                    })
                    .when(!editing && self.snapshot.is_none(), |el| {
                        el.child(
                            icon_button("edit-entry", "pencil", "edit").on_click(cx.listener(
                                |this, _, _, cx| {
                                    this.store.update(cx, |store, cx| store.begin_edit(cx))
                                },
                            )),
                        )
                    }),
            )
            .when_some(self.snapshot, |el, seq| {
                el.child(
                    h_flex()
                        .px_4()
                        .py_2()
                        .gap_2()
                        .child(tr("revision"))
                        .child(stamp(seq))
                        .child(
                            Button::new("current-entry")
                                .ghost()
                                .label(tr("back"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.snapshot = None;
                                    this.revealed.clear();
                                    cx.notify();
                                })),
                        ),
                )
            })
            .child(
                h_flex()
                    .h(rems(2.375))
                    .flex_shrink_0()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .children(EntryTab::ALL.map(|tab| {
                        let selected = self.store.read(cx).state().tab == tab;
                        Button::new(SharedString::from(format!("tab-{tab:?}")))
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
                                this.store.update(cx, |store, cx| store.select_tab(tab, cx))
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
                    .child(content),
            )
    }
}
