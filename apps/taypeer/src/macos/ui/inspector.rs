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

#[derive(Default)]
struct ValueSource {
    attribute: Option<taypeer_core::AttributeId>,
    revision: Option<RevisionId>,
}

pub(super) struct Inspector {
    store: Entity<WorkspaceStore>,
    fields: Option<(EntityId, Entity<EditorView>)>,
    identity: Option<(DatabaseId, EntryId)>,
    values: BTreeMap<String, super::read_value::ReadValue>,
    reveal_epoch: u64,
    tab: EntryTab,
    revealed: BTreeMap<String, Zeroizing<String>>,
    revision: Option<RevisionId>,
    snapshot: Option<RevisionId>,
    compare: bool,
    scroll: ScrollHandle,
    _subscription: Subscription,
}
impl Inspector {
    pub fn new(store: Entity<WorkspaceStore>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            _subscription: cx.observe_in(&store, window, |this, store, window, cx| {
                let state = store.read(cx).state();
                if !state.is_unlocked()
                    || state.route != Route::Workspace
                    || state.tab != this.tab
                    || this.identity.as_ref().is_some_and(|(db, entry)| {
                        Some(db) != state.database.as_ref()
                            || Some(entry) != state.selected.as_ref()
                    })
                {
                    this.clear_values(window, cx);
                    this.fields = None;
                }
                cx.notify();
            }),
            store,
            fields: None,
            identity: None,
            values: BTreeMap::new(),
            reveal_epoch: 0,
            tab: EntryTab::Overview,
            revealed: BTreeMap::new(),
            revision: None,
            snapshot: None,
            compare: false,
            scroll: ScrollHandle::new(),
        }
    }
    fn clear_values(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for value in self.values.values() {
            value.clear(window, cx);
        }
        self.values.clear();
        self.revealed.clear();
        self.reveal_epoch += 1;
    }
    fn value(
        &mut self,
        key: String,
        value: &str,
        secret: bool,
        source: ValueSource,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let ValueSource {
            attribute,
            revision,
        } = source;
        let shown = self.revealed.contains_key(&key);
        let text = if secret {
            self.revealed.get(&key).map(|v| v.to_string())
        } else if value.is_empty() {
            None
        } else {
            Some(value.to_owned())
        };
        let body = if let Some(text) = &text {
            let input = self
                .values
                .entry(key.clone())
                .or_insert_with(|| super::read_value::ReadValue::new(text, window, cx));
            input.sync(text, window, cx);
            input.render(secret, self.store.clone(), self.identity.clone())
        } else {
            if let Some(input) = self.values.remove(&key) {
                input.clear(window, cx);
            }
            div()
                .child(if secret {
                    "••••••••••••"
                } else {
                    "—"
                })
                .into_any_element()
        };
        let present = secret || !value.is_empty();
        let ordinary = value.to_owned();
        let reveal_key = key.clone();
        let copy_key = key.clone();
        let copy_attribute = attribute.clone();
        let copy_revision = revision.clone();
        h_flex()
            .group("entry-value")
            .gap_2()
            .min_w_0()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .when(secret, |el| el.font_family("Menlo"))
                    .child(body),
            )
            .when(present, |el| {
                el.child(
                    h_flex()
                        .gap_1()
                        .invisible()
                        .group_hover("entry-value", |style| style.visible())
                        .when(secret, |el| {
                            el.child(
                                icon_button(
                                    SharedString::from(format!("show-{key}")),
                                    if shown { "eye-off" } else { "eye" },
                                    if shown { "hide" } else { "show" },
                                )
                                .on_click(cx.listener(
                                    move |this, _, window, cx| {
                                        if this.revealed.remove(&reveal_key).is_none() {
                                            this.reveal(
                                                reveal_key.clone(),
                                                attribute.clone(),
                                                revision.clone(),
                                                false,
                                                cx,
                                            );
                                        } else {
                                            this.reveal_epoch += 1;
                                            if let Some(input) = this.values.remove(&reveal_key) {
                                                input.clear(window, cx);
                                            }
                                        }
                                        cx.notify();
                                    },
                                )),
                            )
                        })
                        .child(
                            icon_button(
                                SharedString::from(format!("copy-{key}")),
                                "copy",
                                "ui.copy",
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
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
                                },
                            )),
                        ),
                )
            })
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
        let epoch = self.reveal_epoch;
        let control = connection.control.clone();
        self.store.update(cx, |store, _| {
            store.watch(ticket, move |store, result, _, cx| {
                if !control.is_open() {
                    return;
                }
                match result {
                    Ok(value) => {
                        let _ = target.update(cx, |this, cx| {
                            if this.reveal_epoch == epoch
                                && this.identity == identity
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
        &mut self,
        content: &EntryContent,
        revision: Option<RevisionId>,
        window: &mut Window,
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
                        ValueSource {
                            attribute: None,
                            revision: revision.clone(),
                        },
                        window,
                        cx,
                    ),
                    cx,
                )
            }))
            .into_any_element()
    }
    fn advanced(
        &mut self,
        content: &EntryContent,
        revision: Option<RevisionId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut result = v_flex().child(section("attributes"));
        for attribute in &content.attributes {
            result = result.child(row(
                &attribute.key,
                self.value(
                    format!("{revision:?}-{:?}", attribute.id),
                    &attribute.value,
                    attribute.protected && attribute.has_value,
                    ValueSource {
                        attribute: attribute.id.clone(),
                        revision: revision.clone(),
                    },
                    window,
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
    fn appearance(
        &mut self,
        content: &EntryContent,
        revision: Option<RevisionId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        v_flex()
            .child(row(
                "ui.icon",
                super::images::stored_icon(&content.icon, content.icon_blob.as_ref(), cx),
                cx,
            ))
            .child(row(
                "ui.foreground_color",
                self.value(
                    format!("foreground-{revision:?}"),
                    &content
                        .foreground
                        .map(|_| color_text(content.foreground))
                        .unwrap_or_default(),
                    false,
                    ValueSource::default(),
                    window,
                    cx,
                ),
                cx,
            ))
            .child(row(
                "ui.background_color",
                self.value(
                    format!("background-{revision:?}"),
                    &content
                        .background
                        .map(|_| color_text(content.background))
                        .unwrap_or_default(),
                    false,
                    ValueSource::default(),
                    window,
                    cx,
                ),
                cx,
            ))
            .child(section("ui.preview"))
            .child(
                div()
                    .mx_6()
                    .p_4()
                    .bg(content
                        .background
                        .map(|c| rgba(c).into())
                        .unwrap_or(cx.theme().background))
                    .text_color(
                        content
                            .foreground
                            .map(|c| rgba(c).into())
                            .unwrap_or(cx.theme().foreground),
                    )
                    .child(content.title.clone()),
            )
            .into_any_element()
    }
    fn history(
        &mut self,
        entry: &Entry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
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
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.revision = Some(id.clone());
                        this.compare = false;
                        this.clear_values(window, cx);
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
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.snapshot = this.revision.clone();
                            this.clear_values(window, cx);
                            this.store
                                .update(cx, |s, cx| s.select_tab(EntryTab::Overview, cx));
                        })),
                )
                .child(
                    Button::new("compare")
                        .label(tr("ui.compare"))
                        .disabled(selected.is_none())
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.compare = !this.compare;
                            this.clear_values(window, cx);
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
                    .child(self.overview(old, Some(version.sequence.clone()), window, cx))
                    .child(self.advanced(old, Some(version.sequence.clone()), window, cx))
                    .child(self.appearance(old, Some(version.sequence.clone()), window, cx))
                    .child(section("ui.current"))
                    .child(self.overview(&entry.content, None, window, cx))
                    .child(self.advanced(&entry.content, None, window, cx))
                    .child(self.appearance(&entry.content, None, window, cx));
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
        let editor = store.editor().cloned();
        if self.identity != identity {
            self.identity = identity.clone();
            self.clear_values(window, cx);
            self.snapshot = None;
            self.revision = None;
            self.compare = false;
        }
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
                self.clear_values(window, cx);
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
        self.tab = tab;
        let writable = store.writable(cx);
        let busy = store.busy();
        if self.revision.as_ref().is_some_and(|id| {
            entry
                .as_ref()
                .is_none_or(|e| !e.revisions.iter().any(|r| &r.sequence == id))
        }) {
            self.revision = None;
            self.snapshot = None;
            self.clear_values(window, cx);
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
                EntryTab::History => self.history(entry, window, cx),
                EntryTab::Properties => v_flex()
                    .child(row(
                        "ui.created",
                        self.value(
                            "created".into(),
                            &stamp(entry.created),
                            false,
                            ValueSource::default(),
                            window,
                            cx,
                        ),
                        cx,
                    ))
                    .child(row(
                        "ui.modified",
                        self.value(
                            "modified".into(),
                            &stamp(entry.modified),
                            false,
                            ValueSource::default(),
                            window,
                            cx,
                        ),
                        cx,
                    ))
                    .child(row(
                        "ui.id",
                        self.value(
                            "id".into(),
                            entry.id.as_str(),
                            false,
                            ValueSource::default(),
                            window,
                            cx,
                        ),
                        cx,
                    ))
                    .into_any_element(),
                _ => match shown {
                    None => empty("ui.loading", cx),
                    Some(content) => match tab {
                        EntryTab::Overview => {
                            self.overview(content, self.snapshot.clone(), window, cx)
                        }
                        EntryTab::Advanced => {
                            self.advanced(content, self.snapshot.clone(), window, cx)
                        }
                        _ => self.appearance(content, self.snapshot.clone(), window, cx),
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
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.snapshot = None;
                            this.clear_values(window, cx);
                            cx.notify();
                        })),
                )
            })
            .when(entry.as_ref().is_some_and(|e| e.conflicted), |el| {
                el.child(empty("ui.conflict", cx))
            })
            .child(
                tabs(
                    "entry-tabs",
                    &EntryTab::ALL.map(EntryTab::key),
                    EntryTab::ALL
                        .iter()
                        .position(|candidate| *candidate == tab)
                        .unwrap_or(0),
                    cx,
                )
                .on_click(cx.listener(|this, index, _, cx| {
                    if let Some(tab) = EntryTab::ALL.get(*index) {
                        this.store.update(cx, |s, cx| s.select_tab(*tab, cx));
                    }
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
