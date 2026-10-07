//! Masked details and lazy historical projections; reveal is a separate service command.
pub(super) use super::generator::open_generator;
use super::{editor::EditorView, style::*, workspace::WorkspaceStore};
use crate::actions::{
    ClearExpiry, ClearNotes, ClearPassword, ClearTags, ClearUrl, ClearUsername,
    ToggleEditorPassword,
};
use crate::ui_state::*;
use gpui_kit::{
    component::{
        button::*,
        menu::{DropdownMenu, PopupMenuItem},
        *,
    },
    prelude::FluentBuilder,
    *,
};
use std::collections::BTreeMap;
use taypeer_runtime::Command;
use zeroize::Zeroizing;
mod alternatives;

#[derive(Default)]
struct ValueSource {
    attribute: Option<taypeer_core::AttributeId>,
    revision: Option<RevisionId>,
}

/// Retained entry inspection, revision comparison, and editor presentation.
pub struct Inspector {
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
    alternatives: Option<taypeer_services::ConflictView>,
    scroll: ScrollHandle,
    _subscription: Subscription,
}
impl Inspector {
    #[cfg(feature = "ui-test-support")]
    /// Whether capture must be suppressed because a value is revealed.
    pub fn has_revealed_values(&self) -> bool {
        !self.revealed.is_empty()
    }

    /// Create subscriptions and reconcile retained controls before the first render.
    pub fn new(store: Entity<WorkspaceStore>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut view = Self {
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
                    this.alternatives = None;
                    this.fields = None;
                }
                this.reconcile(window, cx);
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
            alternatives: None,
            scroll: ScrollHandle::new(),
        };
        view.reconcile(window, cx);
        view
    }
    fn reconcile(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
        if self.revision.as_ref().is_some_and(|id| {
            entry
                .as_ref()
                .is_none_or(|e| !e.revisions.iter().any(|r| &r.sequence == id))
        }) {
            self.revision = None;
            self.snapshot = None;
            self.clear_values(window, cx);
        }
        self.sync_values(window, cx);
    }

    fn sync_values(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let entry = self
            .identity
            .as_ref()
            .and_then(|(db, id)| self.store.read(cx).catalog().read(cx).entry(db, id))
            .cloned();
        let mut desired = BTreeMap::<String, Zeroizing<String>>::new();
        if (self.fields.is_none() || matches!(self.tab, EntryTab::Properties | EntryTab::History))
            && let Some(entry) = entry
        {
            let mut add_content = |content: &EntryContent, revision: Option<RevisionId>| {
                let mut add = |key: String, text: &str, secret: bool| {
                    let text = if secret {
                        self.revealed.get(&key).map(|s| s.as_str())
                    } else {
                        Some(text)
                    };
                    if let Some(text) = text {
                        desired.insert(key, Zeroizing::new(text.to_owned()));
                    }
                };
                for field in EntryField::ALL {
                    add(
                        format!("{revision:?}-{field:?}"),
                        field.value(content),
                        field == EntryField::Password && content.has_password,
                    );
                }
                for attribute in &content.attributes {
                    add(
                        format!("{revision:?}-{:?}", attribute.id),
                        &attribute.value,
                        attribute.protected && attribute.has_value,
                    );
                }
                if content.foreground.is_some() {
                    add(
                        format!("foreground-{revision:?}"),
                        &color_text(content.foreground),
                        false,
                    );
                }
                if content.background.is_some() {
                    add(
                        format!("background-{revision:?}"),
                        &color_text(content.background),
                        false,
                    );
                }
            };
            add_content(&entry.content, None);
            for version in &entry.revisions {
                if (Some(&version.sequence) == self.snapshot.as_ref()
                    || (self.compare && Some(&version.sequence) == self.revision.as_ref()))
                    && let Some(content) = &version.content
                {
                    add_content(content, Some(version.sequence.clone()));
                }
            }
            desired.insert("created".into(), Zeroizing::new(stamp(entry.created)));
            desired.insert("modified".into(), Zeroizing::new(stamp(entry.modified)));
            desired.insert("id".into(), Zeroizing::new(entry.id.as_str().to_owned()));
        }
        self.values.retain(|key, input| {
            if desired.contains_key(key) {
                true
            } else {
                input.clear(window, cx);
                false
            }
        });
        for (key, text) in desired {
            if let Some(input) = self.values.get(&key) {
                input.sync(&text, window, cx);
            } else {
                self.values
                    .insert(key, super::read_value::ReadValue::new(&text, window, cx));
            }
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
    fn clear_field(&mut self, field: EntryField, cx: &mut Context<Self>) {
        let Some(editor) = self.store.read(cx).editor().cloned() else {
            return;
        };
        editor.update(cx, |editor, cx| {
            editor.clear_field(field);
            cx.notify();
        });
    }
    fn shown_content<'a>(&self, cx: &'a App) -> Option<&'a EntryContent> {
        let (database, entry) = self.identity.as_ref()?;
        let entry = self
            .store
            .read(cx)
            .catalog()
            .read(cx)
            .entry(database, entry)?;
        match &self.snapshot {
            Some(id) => entry
                .revisions
                .iter()
                .find(|version| &version.sequence == id)?
                .content
                .as_ref(),
            None => Some(&entry.content),
        }
    }
    fn copy_field(&mut self, field: EntryField, cx: &mut Context<Self>) {
        let Some(content) = self.shown_content(cx) else {
            return;
        };
        if field == EntryField::Password && content.has_password {
            self.reveal(
                format!("{:?}-{field:?}", self.snapshot),
                None,
                self.snapshot.clone(),
                true,
                cx,
            );
        } else if self
            .store
            .read(cx)
            .connection()
            .is_some_and(|connection| connection.control.is_open())
        {
            super::clipboard::copy(field.value(content).to_owned(), false, cx);
        }
    }
    fn toggle_read_password(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let key = format!("{:?}-Password", self.snapshot);
        if self.revealed.remove(&key).is_some() {
            self.reveal_epoch += 1;
            if let Some(value) = self.values.remove(&key) {
                value.clear(window, cx);
            }
            cx.notify();
        } else if self
            .shown_content(cx)
            .is_some_and(|content| content.has_password)
        {
            self.reveal(key, None, self.snapshot.clone(), false, cx);
        }
    }
    fn value(
        &mut self,
        key: String,
        value: &str,
        secret: bool,
        source: ValueSource,
        _window: &mut Window,
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
                .or_insert_with(|| super::read_value::ReadValue::new(text, _window, cx));
            input.sync(text, _window, cx);
            input.render(
                format!("read-{key}").into(),
                text.clone(),
                secret,
                self.store.clone(),
                self.identity.clone(),
            )
        } else {
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
        let actions = present.then(|| {
            h_flex()
                .gap_2()
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
                    icon_button(SharedString::from(format!("copy-{key}")), "copy", "ui.copy")
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
        });
        super::read_value::frame(
            SharedString::from(format!("value-{key}")),
            body,
            secret,
            actions,
            cx,
        )
    }
    fn reveal(
        &mut self,
        key: String,
        attribute: Option<taypeer_core::AttributeId>,
        revision: Option<RevisionId>,
        copy: bool,
        cx: &mut Context<Self>,
    ) {
        let Some((_, entry)) = self.identity.clone() else {
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
        self.reveal_command(key, command, copy, cx);
    }
    fn reveal_command(
        &mut self,
        key: String,
        command: Command,
        copy: bool,
        cx: &mut Context<Self>,
    ) {
        let Some((db, _)) = self.identity.clone() else {
            return;
        };
        let Some(connection) = self.store.read(cx).connection().cloned() else {
            return;
        };
        let ticket = connection.command::<Zeroizing<String>>(command);
        let target = cx.entity().downgrade();
        let identity = self.identity.clone();
        let epoch = self.reveal_epoch;
        let control = connection.control.clone();
        self.store.update(cx, |store, _| {
            store.watch(ticket, move |store, result, window, cx| {
                if !control.is_open() {
                    return;
                }
                match result {
                    Ok(value) => {
                        // watch runs while WorkspaceStore is leased. Reconciliation
                        // reads that store, so deliver only after its update finishes.
                        window.defer(cx, move |window, cx| {
                            if !control.is_open() {
                                return;
                            }
                            let _ = target.update(cx, |this, cx| {
                                let state = this.store.read(cx).state();
                                let still_selected = state.is_unlocked()
                                    && state.route == Route::Workspace
                                    && identity.as_ref().is_some_and(|(db, entry)| {
                                        state.database.as_ref() == Some(db)
                                            && state.selected.as_ref() == Some(entry)
                                    });
                                if still_selected
                                    && this.reveal_epoch == epoch
                                    && this.identity == identity
                                    && this.identity.as_ref().is_some_and(|(id, _)| id == &db)
                                {
                                    if copy {
                                        super::clipboard::copy(value.to_string(), true, cx);
                                    } else {
                                        this.revealed.insert(key, value);
                                        this.sync_values(window, cx);
                                    }
                                    cx.notify();
                                }
                            });
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
                super::card::overview_row(
                    field,
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
        let mut result = v_flex().child(super::details::section_header("attributes", None));
        for attribute in &content.attributes {
            result = result.child(super::details::attribute_row(
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
                None,
                cx,
            ));
        }
        result = result.child(super::details::section_header("ui.attachments", None));
        for attachment in &content.attachments {
            let id = attachment.id.clone();
            let revision = revision.clone();
            result = result.child(super::details::attachment_row(
                &attachment.name,
                None,
                icon_button(
                    SharedString::from(format!("export-{}", attachment.id.as_str())),
                    "download",
                    "ui.download",
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    super::forms::export_attachment(
                        this.store.clone(),
                        id.clone(),
                        revision.clone(),
                        window,
                        cx,
                    )
                })),
                cx,
            ));
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
        super::details::appearance(
            content,
            super::images::stored_icon(&content.icon, content.icon_blob.as_ref(), cx),
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
        )
    }
    fn history(
        &mut self,
        entry: &Entry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut result = v_flex()
            .child(super::history::header(
                icon_button("history-menu", "ellipsis", "ui.entry_actions").dropdown_menu({
                    let store = self.store.clone();
                    move |menu, _, cx| {
                        let clear = store.clone();
                        menu.item(
                            PopupMenuItem::new(tr("ui.clear_history"))
                                .disabled(!store.read(cx).writable(cx))
                                .on_click(move |_, window, cx| {
                                    clear.update(cx, |store, cx| {
                                        store.navigate(Destination::ClearHistory, window, cx)
                                    })
                                }),
                        )
                    }
                }),
            ))
            .child(super::history::column_header(cx));
        for (index, version) in entry.revisions.iter().rev().enumerate() {
            let id = version.sequence.clone();
            result = result.child(
                super::history::version(
                    SharedString::from(format!("version-{}", id.as_str())),
                    version.saved_at,
                    div().truncate().child(version.title.clone()),
                    self.revision.as_ref() == Some(&id),
                    !index.is_multiple_of(2),
                    cx,
                )
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
        result = result.child(self.alternatives(window, cx));
        let selected = self.revision.clone();
        result = result.child(super::history::commands(
            super::history::command("view-version", "ui.view_revision")
                .disabled(selected.is_none())
                .on_click(cx.listener(|this, _, window, cx| {
                    this.snapshot = this.revision.clone();
                    this.clear_values(window, cx);
                    this.store
                        .update(cx, |s, cx| s.select_tab(EntryTab::Overview, cx));
                })),
            super::history::command("compare", "ui.compare")
                .disabled(selected.is_none())
                .on_click(cx.listener(|this, _, window, cx| {
                    this.compare = !this.compare;
                    this.clear_values(window, cx);
                    this.sync_values(window, cx);
                    cx.notify();
                })),
            super::history::command("restore-version", "ui.restore_revision")
                .disabled(selected.is_none() || !self.store.read(cx).writable(cx))
                .on_click(cx.listener(move |this, _, window, cx| {
                    if let Some(id) = selected.clone() {
                        this.store.update(cx, |s, cx| {
                            s.navigate(Destination::RestoreRevision(id), window, cx)
                        });
                    }
                })),
        ));
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
        self.reconcile(window, cx);
        let store = self.store.read(cx);
        let editor = store.editor().cloned();
        let entry = self
            .identity
            .as_ref()
            .and_then(|(db, id)| store.catalog().read(cx).entry(db, id))
            .cloned();
        let tab = store.state().tab;
        let writable = store.writable(cx);
        let busy = store.busy();
        let editing = editor.is_some();
        let title = entry
            .as_ref()
            .map(|e| e.content.title.clone())
            .unwrap_or_else(|| tr("new_title").to_string());
        let menu_store = self.store.clone();
        let menu_inspector = cx.entity().downgrade();
        let snapshot = self.snapshot.is_some();
        let password_shown = self
            .revealed
            .contains_key(&format!("{:?}-Password", self.snapshot));
        let editor_password_masked = self
            .fields
            .as_ref()
            .is_none_or(|(_, view)| view.read(cx).password_masked(cx));
        let has_password = self
            .shown_content(cx)
            .is_some_and(|content| content.has_password);
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
                EntryTab::Properties => super::details::properties(
                    [
                        self.value(
                            "created".into(),
                            &stamp(entry.created),
                            false,
                            ValueSource::default(),
                            window,
                            cx,
                        ),
                        self.value(
                            "modified".into(),
                            &stamp(entry.modified),
                            false,
                            ValueSource::default(),
                            window,
                            cx,
                        ),
                        self.value(
                            "id".into(),
                            entry.id.as_str(),
                            false,
                            ValueSource::default(),
                            window,
                            cx,
                        ),
                    ],
                    cx,
                ),
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
        super::card::frame(cx)
            .on_action(cx.listener(|this, _: &ClearUsername, _, cx| {
                this.clear_field(EntryField::Username, cx)
            }))
            .on_action(cx.listener(|this, _: &ClearPassword, _, cx| {
                this.clear_field(EntryField::Password, cx)
            }))
            .on_action(
                cx.listener(|this, _: &ClearUrl, _, cx| this.clear_field(EntryField::Url, cx)),
            )
            .on_action(
                cx.listener(|this, _: &ClearTags, _, cx| this.clear_field(EntryField::Tags, cx)),
            )
            .on_action(
                cx.listener(|this, _: &ClearNotes, _, cx| this.clear_field(EntryField::Notes, cx)),
            )
            .on_action(
                cx.listener(|this, _: &ClearExpiry, _, cx| {
                    this.clear_field(EntryField::Expires, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &ToggleEditorPassword, window, cx| {
                if let Some((_, fields)) = &this.fields {
                    fields.update(cx, |fields, cx| fields.toggle_password(window, cx));
                }
            }))
            .relative()
            .child(
                div()
                    .absolute()
                    .left_0()
                    .top_0()
                    .bottom_0()
                    .w(px(1.))
                    .bg(cx.theme().border),
            )
            .child(super::card::header(
                title,
                h_flex()
                    .gap_2()
                    .when(editing, |el| {
                        el.child(
                            icon_button("save-entry", "check", "save")
                                .disabled(
                                    self.store
                                        .read(cx)
                                        .editor()
                                        .is_none_or(|editor| editor.read(cx).saving()),
                                )
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.store.update(cx, |store, cx| store.save(cx));
                                })),
                        )
                        .child(
                            icon_button("cancel-entry", "x", "cancel")
                                .disabled(busy)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.store.update(cx, |store, cx| store.cancel_edit(cx));
                                })),
                        )
                        .child(
                            icon_button("close-editor", "arrow-left", "back")
                                .disabled(busy)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.store.update(cx, |s, cx| {
                                        s.navigate(Destination::CancelEdit, window, cx)
                                    })
                                })),
                        )
                    })
                    .when(!editing && self.snapshot.is_none(), |el| {
                        el.child(
                            icon_button("edit-entry", "pencil", "edit")
                                .disabled(!writable)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.store.update(cx, |s, cx| s.begin_edit(cx))
                                })),
                        )
                    })
                    .child(
                        icon_button("entry-menu", "ellipsis", "ui.entry_actions").dropdown_menu(
                            move |menu, window, cx| {
                                if editing {
                                    menu.item(
                                        PopupMenuItem::new(tr(if editor_password_masked {
                                            "ui.reveal_current"
                                        } else {
                                            "hide"
                                        }))
                                        .disabled(busy)
                                        .action(Box::new(ToggleEditorPassword)),
                                    )
                                    .submenu(
                                        tr("ui.clear_field"),
                                        window,
                                        cx,
                                        move |mut menu, _, _| {
                                            for (label, action) in [
                                                (
                                                    "username",
                                                    Box::new(ClearUsername) as Box<dyn Action>,
                                                ),
                                                ("password", Box::new(ClearPassword)),
                                                ("url", Box::new(ClearUrl)),
                                                ("ui.tags", Box::new(ClearTags)),
                                                ("notes", Box::new(ClearNotes)),
                                                ("ui.expires", Box::new(ClearExpiry)),
                                            ] {
                                                menu = menu.item(
                                                    PopupMenuItem::new(tr(label))
                                                        .disabled(!writable)
                                                        .action(action),
                                                );
                                            }
                                            menu
                                        },
                                    )
                                } else {
                                    let delete = menu_store.clone();
                                    let reveal = menu_inspector.clone();
                                    let copy = menu_inspector.clone();
                                    menu.item(
                                        PopupMenuItem::new(tr(if password_shown {
                                            "hide"
                                        } else {
                                            "show"
                                        }))
                                        .disabled(!has_password)
                                        .on_click(
                                            move |_, window, cx| {
                                                let _ = reveal.update(cx, |view, cx| {
                                                    view.toggle_read_password(window, cx)
                                                });
                                            },
                                        ),
                                    )
                                    .submenu(tr("ui.copy"), window, cx, move |mut menu, _, _| {
                                        for field in EntryField::ALL {
                                            let target = copy.clone();
                                            menu = menu.item(
                                                PopupMenuItem::new(tr(field.key())).on_click(
                                                    move |_, _, cx| {
                                                        let _ = target.update(cx, |view, cx| {
                                                            view.copy_field(field, cx)
                                                        });
                                                    },
                                                ),
                                            );
                                        }
                                        menu
                                    })
                                    .separator()
                                    .item(
                                        PopupMenuItem::new(tr("ui.delete_entry"))
                                            .disabled(!writable || snapshot)
                                            .on_click(move |_, window, cx| {
                                                delete.update(cx, |store, cx| {
                                                    store.navigate(
                                                        Destination::TrashEntry,
                                                        window,
                                                        cx,
                                                    )
                                                })
                                            }),
                                    )
                                }
                            },
                        ),
                    ),
                cx,
            ))
            .when(self.snapshot.is_some(), |el| {
                el.child(super::card::current_entry().on_click(cx.listener(
                    |this, _, window, cx| {
                        this.snapshot = None;
                        this.clear_values(window, cx);
                        this.sync_values(window, cx);
                        cx.notify();
                    },
                )))
            })
            .when(entry.as_ref().is_some_and(|e| e.conflicted), |el| {
                el.child(
                    div()
                        .px_6()
                        .py_2()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(tr("ui.history_alternatives")),
                )
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
            .child(super::card::body(body).track_scroll(&self.scroll))
    }
}
