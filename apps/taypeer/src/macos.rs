use crate::preferences::Preferences;
use gpui_kit::assets::Assets;
use gpui_kit::component::input::{Textarea, TextareaState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{
    button::*,
    input::{Input, InputContentType, InputEvent, InputState},
    resizable::*,
    *,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use rust_i18n::t;
use std::collections::{BTreeMap, BTreeSet};
use taypeer_services::*;

actions!(
    taypeer,
    [
        SaveEntry,
        LockDatabase,
        CancelEditing,
        FocusSearch,
        GroupUp,
        GroupDown,
        GroupLeft,
        GroupRight,
        GroupEnter,
        EntryUp,
        EntryDown,
        EntryEnter
    ]
);

fn format_date(value: i64) -> String {
    chrono::DateTime::from_timestamp_millis(value)
        .map(|v| v.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default()
}

fn tr(key: &str) -> SharedString {
    t!(key).to_string().into()
}
fn input(value: &str, masked: bool, window: &mut Window, cx: &mut App) -> Entity<InputState> {
    cx.new(|cx| {
        let mut state = InputState::new(window, cx).masked(masked);
        state.set_value(value, window, cx);
        state
    })
}
struct AttributeInputs {
    name: Entity<InputState>,
    value: Entity<InputState>,
}
struct Editor {
    session: SessionToken,
    draft: DraftView,
    fields: Vec<Entity<InputState>>,
    multiline: Vec<Entity<TextareaState>>,
    attributes: Vec<AttributeInputs>,
    subscriptions: Vec<Subscription>,
}
#[derive(Clone)]
enum Navigation {
    Form(Form, String),
    Database(DatabaseId),
    Group(GroupId),
    Entry(EntryId),
    NewEntry,
    Cancel,
}
#[derive(Clone)]
enum Form {
    Database,
    Group(Option<GroupId>),
    Rename(GroupId),
}
struct Client {
    service: DemoService,
    database: Option<DatabaseId>,
    session: Option<SessionToken>,
    sessions: BTreeMap<DatabaseId, SessionToken>,
    group: Option<GroupId>,
    selected: Option<EntryId>,
    editor: Option<Editor>,
    password: Entity<InputState>,
    search: Entity<InputState>,
    form_input: Entity<InputState>,
    form: Option<Form>,
    settings: bool,
    root_focus: FocusHandle,
    modal_focus: FocusHandle,
    group_focus: FocusHandle,
    entry_focus: FocusHandle,
    group_scroll: ScrollHandle,
    entry_scroll: ScrollHandle,
    pending: Option<Navigation>,
    restore: bool,
    tab: u8,
    revision: Option<RevisionId>,
    revealed: BTreeMap<String, SharedString>,
    collapsed: BTreeSet<GroupId>,
    descending: bool,
    prefs: Preferences,
    invalid_prefs: bool,
    error: Option<&'static str>,
    subscriptions: Vec<Subscription>,
}
impl Client {
    fn accepts(&self, token: &SessionToken) -> bool {
        self.session.as_ref() == Some(token) && self.service.is_current(token)
    }
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (prefs, invalid_prefs) = Preferences::load();
        rust_i18n::set_locale(&prefs.language);
        let password = input("", true, window, cx);
        let search = input("", false, window, cx);
        let form_input = input("", false, window, cx);
        let subscriptions = vec![
            cx.subscribe_in(&search, window, |_, _, _: &InputEvent, _, cx| cx.notify()),
            cx.observe_window_appearance(window, |this, window, cx| this.apply_theme(window, cx)),
            cx.observe_window_activation(window, |this, window, cx| this.apply_theme(window, cx)),
        ];
        let mut this = Self {
            service: DemoService::new(),
            database: None,
            session: None,
            sessions: BTreeMap::new(),
            group: None,
            selected: None,
            editor: None,
            password,
            search,
            form_input,
            form: None,
            settings: false,
            root_focus: cx.focus_handle(),
            modal_focus: cx.focus_handle(),
            group_focus: cx.focus_handle(),
            entry_focus: cx.focus_handle(),
            group_scroll: ScrollHandle::new(),
            entry_scroll: ScrollHandle::new(),
            pending: None,
            restore: false,
            tab: 0,
            revision: None,
            revealed: BTreeMap::new(),
            collapsed: BTreeSet::new(),
            descending: false,
            prefs,
            invalid_prefs,
            error: invalid_prefs.then_some("prefs_error"),
            subscriptions,
        };
        this.bind_prompt_inputs(window, cx);
        this.apply_theme(window, cx);
        this
    }
    fn bind_prompt_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.subscriptions.truncate(3);
        self.subscriptions.push(cx.subscribe_in(
            &self.password,
            window,
            |this, state, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. })
                    && this.password.entity_id() == state.entity_id()
                    && this.session.is_none()
                    && !this.settings
                    && this.form.is_none()
                {
                    this.unlock(window, cx);
                }
            },
        ));
        self.subscriptions.push(cx.subscribe_in(
            &self.form_input,
            window,
            |this, state, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. })
                    && this.form_input.entity_id() == state.entity_id()
                    && this.form.is_some()
                {
                    this.commit_form(window, cx);
                }
            },
        ));
        self.update_placeholders(window, cx);
    }
    fn update_placeholders(&self, window: &mut Window, cx: &mut App) {
        self.search.update(cx, |state, cx| {
            state.set_placeholder(tr("search"), window, cx)
        });
        self.password.update(cx, |state, cx| {
            state.set_placeholder(tr("password"), window, cx)
        });
        self.form_input.update(cx, |state, cx| {
            state.set_placeholder(tr("name"), window, cx)
        });
    }
    fn apply_theme(&self, window: &mut Window, cx: &mut App) {
        let dark = match self.prefs.theme.as_str() {
            "light" => false,
            "dark" => true,
            _ => matches!(
                window.appearance(),
                WindowAppearance::Dark | WindowAppearance::VibrantDark
            ),
        };
        Theme::change(
            if dark {
                ThemeMode::Dark
            } else {
                ThemeMode::Light
            },
            None,
            cx,
        );
        let palettes: BTreeMap<String, BTreeMap<String, u32>> =
            toml::from_str(include_str!("../../../resources/theme.toml"))
                .expect("validated embedded palette");
        let palette = &palettes[if dark { "dark" } else { "light" }];
        let color = |role: &str| -> Hsla { rgb(palette[role]).into() };
        let theme = Theme::global_mut(cx);
        theme.background = color("background");
        theme.tokens.background = color("background").into();
        theme.foreground = color("foreground");
        theme.border = color("border");
        theme.input = color("border");
        theme.primary = color("primary");
        theme.primary_foreground = color("on_primary");
        theme.selection = color("selection");
        theme.ring = color("focus");
        theme.muted_foreground = color("muted");
        theme.danger = color("danger");
        theme.warning = color("warning");
        theme.success = color("success");
        theme.font_size = px(self.prefs.font_size as f32);
        Theme::sync_base(cx);
        window.refresh();
    }
    fn save_preferences(&mut self) {
        if self.invalid_prefs {
            self.error = Some("prefs_error");
        } else if self.prefs.save().is_err() {
            self.error = Some("prefs_write");
        }
    }
    fn clear_content(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editor = None;
        self.selected = None;
        self.group = None;
        self.revealed.clear();
        self.revision = None;
        self.pending = None;
        self.restore = false;
        self.tab = 0;
        self.form = None;
        self.password = input("", true, window, cx);
        self.form_input = input("", false, window, cx);
        self.search = input("", false, window, cx);
        self.subscriptions[0] =
            cx.subscribe_in(&self.search, window, |_, _, _: &InputEvent, _, cx| {
                cx.notify()
            });
        self.bind_prompt_inputs(window, cx);
        window.close_all_dialogs(cx);
        self.root_focus.focus(window, cx);
    }
    fn lock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(token) = self.session.take() {
            self.sessions.remove(&token.database);
            if self.service.lock(&token).is_err() {
                self.error = Some("error");
            }
        }
        self.clear_content(window, cx);
        cx.notify();
    }
    fn unlock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = &self.database else {
            return;
        };
        match self
            .service
            .unlock(id, self.password.read(cx).value().as_ref())
        {
            Ok(token) => {
                self.password = input("", true, window, cx);
                self.restore = self
                    .service
                    .pending_draft(&token)
                    .is_ok_and(|v| self.service.is_current(&v.session) && v.value.is_some());
                self.sessions.insert(token.database.clone(), token.clone());
                self.session = Some(token);
                self.error = None;
                self.bind_prompt_inputs(window, cx);
                if self.restore {
                    self.modal_focus.focus(window, cx);
                } else {
                    self.root_focus.focus(window, cx);
                }
            }
            Err(_) => self.error = Some("password_error"),
        }
        cx.notify();
    }
    fn navigate(&mut self, action: Navigation, window: &mut Window, cx: &mut Context<Self>) {
        if self.editor.as_ref().is_some_and(|e| e.draft.dirty) {
            self.pending = Some(action);
            self.modal_focus.focus(window, cx);
            cx.notify();
            return;
        }
        self.perform_navigation(action, window, cx);
    }
    fn perform_navigation(
        &mut self,
        action: Navigation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editor.is_some()
            && let Some(token) = &self.session
        {
            let _ = self.service.cancel_draft(token);
        }
        self.editor = None;
        self.revealed.clear();
        self.revision = None;
        self.tab = 0;
        self.pending = None;
        self.error = None;
        self.root_focus.focus(window, cx);
        match action {
            Navigation::Database(id) => {
                self.clear_content(window, cx);
                self.session = self
                    .sessions
                    .get(&id)
                    .filter(|token| self.service.is_current(token))
                    .cloned();
                self.restore = self.session.as_ref().is_some_and(|token| {
                    self.service
                        .pending_draft(token)
                        .is_ok_and(|reply| reply.value.is_some())
                });
                self.database = Some(id);
            }
            Navigation::Group(id) => {
                self.group = Some(id);
                self.selected = None;
            }
            Navigation::Entry(id) => {
                self.selected = Some(id);
            }
            Navigation::NewEntry => {
                if let (Some(token), Some(group)) = (&self.session, &self.group) {
                    match self.service.start_create_entry(token, group.clone()) {
                        Ok(reply) if self.accepts(&reply.session) => {
                            self.selected = None;
                            self.install_editor(reply.session, reply.value, window, cx);
                        }
                        _ => self.error = Some("error"),
                    }
                }
            }
            Navigation::Cancel => {}
            Navigation::Form(form, value) => self.open_form(form, &value, window, cx),
        }
        cx.notify();
    }
    fn edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let (Some(token), Some(id)) = (&self.session, &self.selected) {
            match self.service.start_edit_entry(token, id) {
                Ok(reply) if self.accepts(&reply.session) => {
                    self.install_editor(reply.session, reply.value, window, cx)
                }
                _ => self.error = Some("error"),
            }
        }
        cx.notify();
    }
    fn install_editor(
        &mut self,
        session: SessionToken,
        draft: DraftView,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.tab = 0;
        let f = &draft.fields;
        let values = [
            f.title.clone(),
            f.username.clone().unwrap_or_default(),
            f.password.clone().unwrap_or_default(),
            f.url.clone().unwrap_or_default(),
            f.notes.clone().unwrap_or_default(),
            f.tags.join("\n"),
            draft
                .expiry_input
                .clone()
                .unwrap_or_else(|| f.expires_at.map(format_date).unwrap_or_default()),
        ];
        let fields: Vec<_> = values
            .iter()
            .enumerate()
            .map(|(i, value)| input(value, i == 2, window, cx))
            .collect();
        for (index, absent) in [
            (1, f.username.is_none()),
            (2, f.password.is_none()),
            (3, f.url.is_none()),
        ] {
            if absent {
                fields[index].update(cx, |state, cx| {
                    state.set_placeholder(tr("absent"), window, cx)
                });
            }
        }
        fields[6].update(cx, |state, cx| {
            state.set_placeholder("YYYY-MM-DD HH:MM", window, cx)
        });
        let mut subscriptions: Vec<Subscription> = fields
            .iter()
            .enumerate()
            .map(|(index, state)| {
                let token = session.clone();
                cx.subscribe_in(state, window, move |this, state, event, _, cx| {
                    if matches!(event, InputEvent::Change)
                        && this.accepts(&token)
                        && this.editor.as_ref().is_some_and(|editor| {
                            editor.fields[index].entity_id() == state.entity_id()
                        })
                    {
                        let value = state.read(cx).value().to_string();
                        this.change_field(index, value, cx);
                    }
                })
            })
            .collect();
        let multiline: Vec<_> = [4, 5]
            .into_iter()
            .map(|index| {
                cx.new(|cx| {
                    let mut state = TextareaState::new(window, cx);
                    state.set_value(values[index].clone(), window, cx);
                    state
                })
            })
            .collect();
        for (position, state) in multiline.iter().enumerate() {
            let token = session.clone();
            subscriptions.push(cx.subscribe_in(
                state,
                window,
                move |this, state, event: &InputEvent, _, cx| {
                    if matches!(event, InputEvent::Change)
                        && this.accepts(&token)
                        && this.editor.as_ref().is_some_and(|editor| {
                            editor.multiline[position].entity_id() == state.entity_id()
                        })
                    {
                        this.change_field(position + 4, state.read(cx).value().to_string(), cx);
                    }
                },
            ));
        }
        self.editor = Some(Editor {
            session,
            draft,
            fields,
            multiline,
            attributes: Vec::new(),
            subscriptions,
        });
        self.install_attributes(window, cx);
        if let Some(editor) = &self.editor {
            editor.fields[0].update(cx, |state, cx| state.focus(window, cx));
        }
    }
    fn install_attributes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = &mut self.editor else {
            return;
        };
        editor.subscriptions.truncate(9);
        editor.attributes.clear();
        for (index, attr) in editor.draft.fields.attributes.iter().enumerate() {
            let name = input(&attr.name, false, window, cx);
            let value = input(&attr.value, attr.protected, window, cx);
            for (is_name, state) in [(true, &name), (false, &value)] {
                let token = editor.session.clone();
                editor.subscriptions.push(cx.subscribe_in(
                    state,
                    window,
                    move |this, state, event, _, cx| {
                        if matches!(event, InputEvent::Change)
                            && this.accepts(&token)
                            && this
                                .editor
                                .as_ref()
                                .and_then(|editor| editor.attributes.get(index))
                                .is_some_and(|attr| {
                                    if is_name {
                                        attr.name.entity_id() == state.entity_id()
                                    } else {
                                        attr.value.entity_id() == state.entity_id()
                                    }
                                })
                        {
                            let value = state.read(cx).value().to_string();
                            if let Some(editor) = &mut this.editor
                                && let Some(attr) = editor.draft.fields.attributes.get_mut(index)
                            {
                                if is_name {
                                    attr.name = value;
                                } else {
                                    attr.value = value;
                                }
                            }
                            this.push_draft(cx);
                        }
                    },
                ));
            }
            editor.attributes.push(AttributeInputs { name, value });
        }
    }
    fn change_field(&mut self, index: usize, value: String, cx: &mut Context<Self>) {
        let Some(editor) = &mut self.editor else {
            return;
        };
        match index {
            0 => editor.draft.fields.title = value,
            1 => editor.draft.fields.username = Some(value),
            2 => editor.draft.fields.password = Some(value),
            3 => editor.draft.fields.url = Some(value),
            4 => editor.draft.fields.notes = Some(value),
            5 => {
                editor.draft.fields.tags = if value.is_empty() {
                    Vec::new()
                } else {
                    value.split('\n').map(str::to_string).collect()
                }
            }
            6 => {
                editor.draft.fields.expires_at = if value.is_empty() {
                    None
                } else {
                    match chrono::NaiveDateTime::parse_from_str(&value, "%Y-%m-%d %H:%M")
                        .map(|v| v.and_utc().timestamp_millis())
                    {
                        Ok(value) => Some(value),
                        Err(_) => {
                            self.error = Some("invalid_expiry");
                            match self
                                .service
                                .set_draft_expiry_input(&editor.session, Some(value))
                            {
                                Ok(reply) => editor.draft = reply.value,
                                Err(_) => self.error = Some("error"),
                            }
                            cx.notify();
                            return;
                        }
                    }
                };
            }
            _ => return,
        }
        if index == 6
            && self
                .service
                .set_draft_expiry_input(&editor.session, None)
                .is_err()
        {
            self.error = Some("error");
            cx.notify();
            return;
        }
        self.error = None;
        self.push_draft(cx);
    }
    fn push_draft(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = &mut self.editor else {
            return;
        };
        match self
            .service
            .update_draft(&editor.session, editor.draft.fields.clone())
        {
            Ok(reply)
                if self.session.as_ref() == Some(&reply.session)
                    && self.service.is_current(&reply.session) =>
            {
                editor.draft = reply.value
            }
            _ => self.error = Some("error"),
        }
        cx.notify();
    }
    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(editor) = &self.editor else {
            return true;
        };
        if editor.draft.expiry_input.is_some() {
            self.error = Some("invalid_expiry");
            return false;
        }
        match self.service.save_draft(&editor.session) {
            Ok(reply) if self.accepts(&reply.session) => {
                self.selected = Some(reply.value);
                self.editor = None;
                self.root_focus.focus(window, cx);
                self.revealed.clear();
                self.error = None;
                cx.notify();
                true
            }
            _ => {
                self.error = Some("error");
                cx.notify();
                false
            }
        }
    }
    fn open_form(&mut self, form: Form, value: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .editor
            .as_ref()
            .is_some_and(|editor| editor.draft.dirty)
        {
            self.pending = Some(Navigation::Form(form, value.into()));
            self.modal_focus.focus(window, cx);
            cx.notify();
            return;
        }
        if self.editor.is_some() {
            if let Some(token) = &self.session
                && self.service.cancel_draft(token).is_err()
            {
                self.error = Some("error");
                cx.notify();
                return;
            }
            self.editor = None;
        }
        self.form_input = input(value, false, window, cx);
        self.form = Some(form);
        self.bind_prompt_inputs(window, cx);
        self.form_input
            .update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }
    fn commit_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.form_input.read(cx).value().to_string();
        let success = match self.form.clone() {
            Some(Form::Database) => match self.service.create_database(name) {
                Ok(id) => {
                    self.clear_content(window, cx);
                    self.session = None;
                    self.database = Some(id);
                    true
                }
                Err(_) => false,
            },
            Some(Form::Group(parent)) => self.session.clone().as_ref().is_some_and(|token| {
                match self.service.create_group(token, name, parent) {
                    Ok(reply) if self.accepts(&reply.session) => {
                        self.group = Some(reply.value.id);
                        true
                    }
                    _ => false,
                }
            }),
            Some(Form::Rename(id)) => self
                .session
                .as_ref()
                .is_some_and(|token| self.service.update_group(token, &id, name).is_ok()),
            None => false,
        };
        if success {
            self.form = None;
            self.root_focus.focus(window, cx);
            self.error = None;
        } else {
            self.error = Some("error");
        }
        cx.notify();
    }

    fn header(&self, cx: &mut Context<Self>) -> AnyElement {
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
    fn unlock_panel(&self, cx: &mut Context<Self>) -> AnyElement {
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
                    div()
                        .text_color(cx.theme().muted_foreground)
                        .child(t!("password_hint", password = DEMO_PASSWORD).to_string()),
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
    fn visible_groups(&self) -> Vec<GroupSummary> {
        let groups = self
            .session
            .as_ref()
            .and_then(|token| self.service.groups(token).ok())
            .filter(|reply| self.accepts(&reply.session))
            .map(|reply| reply.value)
            .unwrap_or_default();
        fn walk(
            groups: &[GroupSummary],
            parent: Option<&GroupId>,
            collapsed: &BTreeSet<GroupId>,
            out: &mut Vec<GroupSummary>,
        ) {
            for group in groups
                .iter()
                .filter(|group| group.parent.as_ref() == parent)
            {
                out.push(group.clone());
                if !collapsed.contains(&group.id) {
                    walk(groups, Some(&group.id), collapsed, out);
                }
            }
        }
        let mut result = Vec::new();
        walk(&groups, None, &self.collapsed, &mut result);
        result
    }
    fn visible_entries(&self, cx: &App) -> Vec<EntrySummary> {
        let query = self.search.read(cx).value().to_string();
        let mut entries = self
            .session
            .as_ref()
            .and_then(|token| {
                self.service
                    .entries(
                        token,
                        if query.is_empty() {
                            self.group.as_ref()
                        } else {
                            None
                        },
                        &query,
                    )
                    .ok()
            })
            .filter(|reply| self.accepts(&reply.session))
            .map(|reply| reply.value)
            .unwrap_or_default();
        entries.sort_by(|a, b| a.title.cmp(&b.title).then(a.id.cmp(&b.id)));
        if self.descending {
            entries.reverse();
        }
        if query.is_empty() && self.group.is_none() {
            entries.clear();
        }
        entries
    }
    fn move_group(&mut self, direction: isize, window: &mut Window, cx: &mut Context<Self>) {
        let groups = self.visible_groups();
        if groups.is_empty() {
            return;
        }
        let index = self
            .group
            .as_ref()
            .and_then(|id| groups.iter().position(|group| &group.id == id))
            .map(|index| index.saturating_add_signed(direction).min(groups.len() - 1))
            .unwrap_or(0);
        if self.group.as_ref() != Some(&groups[index].id) {
            self.navigate(Navigation::Group(groups[index].id.clone()), window, cx);
        }
        self.group_scroll.scroll_to_item(index);
    }
    fn branch_group(&mut self, expand: bool, window: &mut Window, cx: &mut Context<Self>) {
        let groups = self.visible_groups();
        let Some(selected) = self.group.clone() else {
            self.move_group(0, window, cx);
            return;
        };
        if expand {
            if !self.collapsed.remove(&selected)
                && let Some(child) = groups
                    .iter()
                    .find(|group| group.parent.as_ref() == Some(&selected))
            {
                self.navigate(Navigation::Group(child.id.clone()), window, cx);
            }
        } else if groups
            .iter()
            .any(|group| group.parent.as_ref() == Some(&selected))
            && !self.collapsed.contains(&selected)
        {
            self.collapsed.insert(selected);
        } else if let Some(parent) = groups
            .iter()
            .find(|group| group.id == selected)
            .and_then(|group| group.parent.clone())
        {
            self.navigate(Navigation::Group(parent), window, cx);
        }
        if let Some(index) = self
            .visible_groups()
            .iter()
            .position(|group| self.group.as_ref() == Some(&group.id))
        {
            self.group_scroll.scroll_to_item(index);
        }
        cx.notify();
    }
    fn move_entry(&mut self, direction: isize, window: &mut Window, cx: &mut Context<Self>) {
        let entries = self.visible_entries(cx);
        if entries.is_empty() {
            return;
        }
        let index = self
            .selected
            .as_ref()
            .and_then(|id| entries.iter().position(|entry| &entry.id == id))
            .map(|index| {
                index
                    .saturating_add_signed(direction)
                    .min(entries.len() - 1)
            })
            .unwrap_or(0);
        if self.selected.as_ref() != Some(&entries[index].id) {
            self.navigate(Navigation::Entry(entries[index].id.clone()), window, cx);
        }
        self.entry_scroll.scroll_to_item(index);
    }
    fn group_rows(
        &self,
        groups: &[GroupSummary],
        parent: Option<&GroupId>,
        depth: usize,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let mut rows = Vec::new();
        for group in groups
            .iter()
            .filter(|group| group.parent.as_ref() == parent)
        {
            let id = group.id.clone();
            let toggle = id.clone();
            let has_children = groups
                .iter()
                .any(|child| child.parent.as_ref() == Some(&id));
            rows.push(
                h_flex()
                    .h_9()
                    .pl(px((8 + depth * 16) as f32))
                    .pr_2()
                    .gap_1()
                    .when(self.group.as_ref() == Some(&id), |el| {
                        el.bg(cx.theme().selection)
                    })
                    .child(
                        Button::new(SharedString::from(format!("expand-{}", id.as_str())))
                            .label(if has_children {
                                if self.collapsed.contains(&id) {
                                    "›"
                                } else {
                                    "⌄"
                                }
                            } else {
                                "·"
                            })
                            .disabled(!has_children)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.group_focus.focus(window, cx);
                                if !this.collapsed.remove(&toggle) {
                                    this.collapsed.insert(toggle.clone());
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("group-{}", id.as_str())))
                            .label(group.name.clone())
                            .flex_1()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.navigate(Navigation::Group(id.clone()), window, cx);
                                if this.pending.is_none() {
                                    this.group_focus.focus(window, cx);
                                }
                            })),
                    )
                    .into_any_element(),
            );
            if !self.collapsed.contains(&group.id) {
                rows.extend(self.group_rows(groups, Some(&group.id), depth + 1, cx));
            }
        }
        rows
    }
    fn groups_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let groups = self
            .session
            .as_ref()
            .and_then(|token| self.service.groups(token).ok())
            .filter(|r| self.accepts(&r.session))
            .map(|r| r.value)
            .unwrap_or_default();
        v_flex()
            .size_full()
            .min_h_0()
            .child(
                h_flex()
                    .h_11()
                    .px_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        Button::new("add-group")
                            .label(tr("add_group"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_form(Form::Group(None), "", window, cx)
                            })),
                    ),
            )
            .child(
                v_flex()
                    .id("group-scroll")
                    .key_context("TaypeerGroups")
                    .track_focus(&self.group_focus)
                    .tab_index(0)
                    .role(Role::Tree)
                    .aria_label(tr("group_navigation"))
                    .on_action(
                        cx.listener(|this, _: &GroupUp, window, cx| {
                            this.move_group(-1, window, cx)
                        }),
                    )
                    .on_action(
                        cx.listener(|this, _: &GroupDown, window, cx| {
                            this.move_group(1, window, cx)
                        }),
                    )
                    .on_action(cx.listener(|this, _: &GroupLeft, window, cx| {
                        this.branch_group(false, window, cx)
                    }))
                    .on_action(cx.listener(|this, _: &GroupRight, window, cx| {
                        this.branch_group(true, window, cx)
                    }))
                    .on_action(cx.listener(|this, _: &GroupEnter, window, cx| {
                        if this.group.is_none() {
                            this.move_group(0, window, cx);
                        }
                        if this.pending.is_none() {
                            this.entry_focus.focus(window, cx);
                        }
                    }))
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.group_scroll)
                    .vertical_scrollbar(&self.group_scroll)
                    .children(self.group_rows(&groups, None, 0, cx))
                    .when(groups.is_empty(), |el| {
                        el.child(
                            div()
                                .p_3()
                                .text_color(cx.theme().muted_foreground)
                                .child(tr("empty_groups")),
                        )
                    }),
            )
            .when_some(self.group.as_ref(), |el, id| {
                let parent = id.clone();
                let rename = id.clone();
                let name = groups
                    .iter()
                    .find(|g| &g.id == id)
                    .map(|g| g.name.clone())
                    .unwrap_or_default();
                el.child(
                    v_flex()
                        .p_2()
                        .gap_1()
                        .child(Button::new("add-child").label(tr("add_child")).on_click(
                            cx.listener(move |this, _, window, cx| {
                                this.open_form(Form::Group(Some(parent.clone())), "", window, cx)
                            }),
                        ))
                        .child(
                            Button::new("rename-group")
                                .label(tr("edit_group"))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.open_form(Form::Rename(rename.clone()), &name, window, cx)
                                })),
                        ),
                )
            })
            .into_any_element()
    }
    fn entries_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let query = self.search.read(cx).value().to_string();
        let entries = self.visible_entries(cx);
        v_flex()
            .size_full()
            .min_h_0()
            .child(
                h_flex()
                    .h_11()
                    .px_2()
                    .gap_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .flex_1()
                            .child(Input::new(&self.search).aria_label(tr("search"))),
                    )
                    .child(
                        Button::new("new-entry")
                            .label(tr("new_entry"))
                            .disabled(self.group.is_none())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.navigate(Navigation::NewEntry, window, cx)
                            })),
                    ),
            )
            .child(
                h_flex()
                    .h_9()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        Button::new("sort-title")
                            .flex_1()
                            .label(format!(
                                "{} {}",
                                tr("title"),
                                if self.descending { "↓" } else { "↑" }
                            ))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.descending = !this.descending;
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .w_32()
                            .px_2()
                            .border_l_1()
                            .border_color(cx.theme().border)
                            .child(tr("username")),
                    ),
            )
            .child(
                v_flex()
                    .id("entry-scroll")
                    .key_context("TaypeerEntries")
                    .track_focus(&self.entry_focus)
                    .tab_index(0)
                    .role(Role::ListBox)
                    .aria_label(tr("entry_navigation"))
                    .on_action(
                        cx.listener(|this, _: &EntryUp, window, cx| {
                            this.move_entry(-1, window, cx)
                        }),
                    )
                    .on_action(
                        cx.listener(|this, _: &EntryDown, window, cx| {
                            this.move_entry(1, window, cx)
                        }),
                    )
                    .on_action(cx.listener(|this, _: &EntryEnter, window, cx| {
                        if this.selected.is_none() {
                            this.move_entry(0, window, cx);
                        } else if this.editor.is_none() {
                            this.edit(window, cx);
                        }
                    }))
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.entry_scroll)
                    .vertical_scrollbar(&self.entry_scroll)
                    .when(entries.is_empty(), |el| {
                        el.child(
                            div()
                                .p_3()
                                .text_color(cx.theme().muted_foreground)
                                .child(tr(if self.group.is_none() && query.is_empty() {
                                    "choose_group"
                                } else {
                                    "empty_entries"
                                })),
                        )
                    })
                    .children(entries.into_iter().map(|entry| {
                        let id = entry.id.clone();
                        h_flex()
                            .id(SharedString::from(format!("entry-{}", id.as_str())))
                            .h_9()
                            .role(Role::ListBoxOption)
                            .aria_label(entry.title.clone())
                            .aria_selected(self.selected.as_ref() == Some(&id))
                            .px_2()
                            .gap_2()
                            .cursor_pointer()
                            .when(self.selected.as_ref() == Some(&id), |el| {
                                el.bg(cx.theme().selection)
                            })
                            .child(div().flex_1().truncate().child(format!(
                                "{}{}",
                                if entry.has_conflicts { "⚠ " } else { "" },
                                entry.title
                            )))
                            .child(
                                div()
                                    .w_32()
                                    .truncate()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(entry.username.unwrap_or_default()),
                            )
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.navigate(Navigation::Entry(id.clone()), window, cx);
                                if this.pending.is_none() {
                                    this.entry_focus.focus(window, cx);
                                }
                            }))
                            .into_any_element()
                    })),
            )
            .into_any_element()
    }
    fn row(&self, label: &str, value: impl IntoElement, cx: &App) -> AnyElement {
        h_flex()
            .min_h_11()
            .gap_4()
            .border_b_1()
            .border_color(cx.theme().border)
            .items_start()
            .py_2()
            .child(
                div()
                    .w(rems(9.))
                    .flex_shrink_0()
                    .text_color(cx.theme().muted_foreground)
                    .child(tr(label)),
            )
            .child(div().flex_1().min_w_0().child(value))
            .into_any_element()
    }
    fn editor_content(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(editor) = &self.editor else {
            return div().into_any_element();
        };
        if self.tab == 1 {
            return v_flex()
                .gap_3()
                .children(editor.attributes.iter().enumerate().map(|(index, attr)| {
                    let protected = editor.draft.fields.attributes[index].protected;
                    v_flex()
                        .gap_2()
                        .border_b_1()
                        .border_color(cx.theme().border)
                        .pb_3()
                        .child(Input::new(&attr.name).aria_label(tr("name")))
                        .child(
                            Input::new(&attr.value)
                                .aria_label(tr("value"))
                                .when(protected, |el| {
                                    el.mask_toggle().content_type(InputContentType::Password)
                                }),
                        )
                        .child(
                            h_flex()
                                .gap_2()
                                .child(
                                    Button::new(("protect-attribute", index))
                                        .label(tr("protected"))
                                        .selected(protected)
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            if let Some(editor) = &mut this.editor {
                                                editor.draft.fields.attributes[index].protected =
                                                    !protected;
                                                editor.attributes[index].value.update(
                                                    cx,
                                                    |state, cx| {
                                                        state.set_masked(!protected, window, cx)
                                                    },
                                                );
                                            }
                                            this.push_draft(cx);
                                        })),
                                )
                                .child(
                                    Button::new(("remove-attribute", index))
                                        .label(tr("remove"))
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            if let Some(editor) = &mut this.editor {
                                                editor.draft.fields.attributes.remove(index);
                                            }
                                            this.push_draft(cx);
                                            this.install_attributes(window, cx);
                                        })),
                                ),
                        )
                        .into_any_element()
                }))
                .child(
                    Button::new("add-attribute")
                        .label(tr("add_attribute"))
                        .on_click(cx.listener(|this, _, window, cx| {
                            if let Some(editor) = &mut this.editor {
                                editor.draft.fields.attributes.push(EditableAttribute {
                                    id: None,
                                    name: String::new(),
                                    value: String::new(),
                                    protected: true,
                                });
                            }
                            this.push_draft(cx);
                            this.install_attributes(window, cx);
                        })),
                )
                .into_any_element();
        }
        let names = [
            "title", "username", "password", "url", "notes", "tags", "expires",
        ];
        v_flex()
            .children(editor.fields.iter().enumerate().map(|(index, state)| {
                self.row(
                    names[index],
                    v_flex()
                        .gap_1()
                        .child(if index == 4 || index == 5 {
                            Textarea::new(&editor.multiline[index - 4])
                                .aria_label(tr(names[index]))
                                .h(rems(5.))
                                .into_any_element()
                        } else {
                            Input::new(state)
                                .aria_label(tr(names[index]))
                                .when(index == 2, |el| {
                                    el.mask_toggle().content_type(InputContentType::Password)
                                })
                                .into_any_element()
                        })
                        .when((1..=4).contains(&index), |el| {
                            el.child(Button::new(("unset", index)).label(tr("clear")).on_click(
                                cx.listener(move |this, _, window, cx| {
                                    if let Some(editor) = &mut this.editor {
                                        editor.fields[index].update(cx, |state, cx| {
                                            state.set_value("", window, cx)
                                        });
                                        if index == 4 {
                                            editor.multiline[0].update(cx, |state, cx| {
                                                state.set_value("", window, cx)
                                            });
                                        }
                                        match index {
                                            1 => editor.draft.fields.username = None,
                                            2 => editor.draft.fields.password = None,
                                            3 => editor.draft.fields.url = None,
                                            4 => editor.draft.fields.notes = None,
                                            _ => {}
                                        }
                                    }
                                    this.push_draft(cx);
                                }),
                            ))
                        }),
                    cx,
                )
            }))
            .into_any_element()
    }

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
    fn detail_content(&self, view: &EntryView, cx: &mut Context<Self>) -> AnyElement {
        if self.tab == 1 {
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
        if self.tab == 2 && self.revision.is_none() {
            let history = self
                .session
                .as_ref()
                .and_then(|token| {
                    self.selected
                        .as_ref()
                        .and_then(|id| self.service.history(token, id).ok())
                })
                .filter(|r| self.accepts(&r.session))
                .map(|r| r.value)
                .unwrap_or_default();
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
                            this.root_focus.focus(window, cx);
                            this.revision = Some(id.clone());
                            this.revealed.clear();
                            this.tab = 0;
                            cx.notify();
                        }))
                }))
                .into_any_element();
        }
        v_flex()
            .child(self.row("title", view.title.clone(), cx))
            .child(
                self.row(
                    "username",
                    view.username
                        .clone()
                        .unwrap_or_else(|| tr("absent").to_string()),
                    cx,
                ),
            )
            .child(
                self.row(
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
                ),
            )
            .child(self.row(
                "url",
                view.url.clone().unwrap_or_else(|| tr("absent").to_string()),
                cx,
            ))
            .child(
                self.row(
                    "notes",
                    view.notes
                        .clone()
                        .unwrap_or_else(|| tr("absent").to_string()),
                    cx,
                ),
            )
            .child(self.row("tags", view.tags.join("\n"), cx))
            .child(
                self.row(
                    "expires",
                    view.expires_at
                        .map(format_date)
                        .unwrap_or_else(|| tr("absent").to_string()),
                    cx,
                ),
            )
            .into_any_element()
    }
    fn inspector(&self, cx: &mut Context<Self>) -> AnyElement {
        let view = self
            .session
            .as_ref()
            .and_then(|token| {
                self.selected.as_ref().and_then(|id| {
                    if let Some(revision) = &self.revision {
                        self.service.revision(token, id, revision).ok()
                    } else {
                        self.service.view_entry(token, id).ok()
                    }
                })
            })
            .filter(|r| self.accepts(&r.session))
            .map(|r| r.value);
        let title = view
            .as_ref()
            .map(|v| v.title.clone())
            .unwrap_or_else(|| tr("new_title").to_string());
        let conflict = view.as_ref().is_some_and(|v| v.has_conflicts);
        v_flex()
            .size_full()
            .min_h_0()
            .child(
                h_flex()
                    .h_11()
                    .px_3()
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
                    .when(self.editor.is_some(), |el| {
                        el.child(
                            Button::new("cancel-edit")
                                .icon(Icon::default().path("product/x.svg"))
                                .tooltip(tr("cancel"))
                                .accessibility_label(tr("cancel"))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.navigate(Navigation::Cancel, window, cx)
                                })),
                        )
                        .child(
                            Button::new("save-edit")
                                .icon(IconName::Check)
                                .tooltip(tr("save"))
                                .accessibility_label(tr("save"))
                                .primary()
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.save(window, cx);
                                })),
                        )
                    })
                    .when(self.editor.is_none() && self.revision.is_none(), |el| {
                        el.child(
                            Button::new("edit-entry")
                                .icon(Icon::default().path("product/pencil.svg"))
                                .tooltip(tr("edit"))
                                .accessibility_label(tr("edit"))
                                .disabled(conflict)
                                .on_click(cx.listener(|this, _, window, cx| this.edit(window, cx))),
                        )
                    }),
            )
            .child(
                h_flex()
                    .h_9()
                    .px_2()
                    .gap_1()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .children(
                        ["overview", "attributes", "history"]
                            .iter()
                            .enumerate()
                            .filter(|(index, _)| self.editor.is_none() || *index != 2)
                            .map(|(index, key)| {
                                Button::new(("tab", index))
                                    .label(tr(key))
                                    .selected(self.tab == index as u8)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.tab = index as u8;
                                        this.revealed.clear();
                                        cx.notify();
                                    }))
                            }),
                    ),
            )
            .when(conflict, |el| {
                el.child(
                    div()
                        .p_3()
                        .text_color(cx.theme().warning)
                        .child(tr("conflict")),
                )
            })
            .when(self.revision.is_some(), |el| {
                el.child(
                    h_flex().p_2().gap_2().child(tr("revision")).child(
                        Button::new("back-current")
                            .label(tr("back"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.root_focus.focus(window, cx);
                                this.revision = None;
                                this.revealed.clear();
                                cx.notify();
                            })),
                    ),
                )
            })
            .child(
                v_flex()
                    .id("inspector-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .p_3()
                    .child(if self.editor.is_some() {
                        self.editor_content(cx)
                    } else if let Some(view) = &view {
                        self.detail_content(view, cx)
                    } else {
                        div().into_any_element()
                    }),
            )
            .into_any_element()
    }
    fn workspace(&self, cx: &mut Context<Self>) -> AnyElement {
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
    fn overlay(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let content =
            if self.settings {
                v_flex()
                    .gap_4()
                    .child(div().text_lg().child(tr("settings")))
                    .child(tr("language"))
                    .child(
                        h_flex()
                            .gap_2()
                            .children([("en", "English"), ("ru", "Русский")].map(
                                |(locale, label)| {
                                    Button::new(locale)
                                        .label(label)
                                        .selected(self.prefs.language == locale)
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.prefs.language = locale.into();
                                            rust_i18n::set_locale(locale);
                                            this.update_placeholders(window, cx);
                                            this.save_preferences();
                                            cx.notify();
                                        }))
                                },
                            )),
                    )
                    .child(tr("theme"))
                    .child(
                        h_flex()
                            .gap_2()
                            .children(["system", "light", "dark"].map(|theme| {
                                Button::new(theme)
                                    .label(tr(theme))
                                    .selected(self.prefs.theme == theme)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.prefs.theme = theme.into();
                                        this.save_preferences();
                                        this.apply_theme(window, cx);
                                        cx.notify();
                                    }))
                            })),
                    )
                    .child(tr("font"))
                    .child(h_flex().gap_2().children([14u8, 16, 18].map(|size| {
                        Button::new(("font", size as usize))
                            .label(size.to_string())
                            .selected(self.prefs.font_size == size)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.prefs.font_size = size;
                                this.save_preferences();
                                this.apply_theme(window, cx);
                                cx.notify();
                            }))
                    })))
                    .child(
                        Button::new("close-settings")
                            .label(tr("close"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.root_focus.focus(window, cx);
                                this.settings = false;
                                cx.notify();
                            })),
                    )
                    .into_any_element()
            } else if self.pending.is_some() {
                v_flex()
                    .gap_4()
                    .child(div().text_lg().child(tr("unsaved")))
                    .child(tr("unsaved_body"))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("nav-save")
                                    .label(tr("save"))
                                    .primary()
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        if this.save(window, cx)
                                            && let Some(action) = this.pending.take()
                                        {
                                            this.perform_navigation(action, window, cx);
                                        }
                                    })),
                            )
                            .child(Button::new("nav-discard").label(tr("discard")).on_click(
                                cx.listener(|this, _, window, cx| {
                                    if let Some(action) = this.pending.take() {
                                        this.perform_navigation(action, window, cx);
                                    }
                                }),
                            ))
                            .child(Button::new("nav-stay").label(tr("stay")).on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.root_focus.focus(window, cx);
                                    this.pending = None;
                                    cx.notify();
                                }),
                            )),
                    )
                    .into_any_element()
            } else if self.restore {
                v_flex()
                    .gap_4()
                    .child(tr("restore_body"))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("restore-draft")
                                    .label(tr("restore"))
                                    .primary()
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        if let Some(token) = &this.session {
                                            match this.service.restore_draft(token) {
                                                Ok(reply) if this.accepts(&reply.session) => {
                                                    let draft = reply.value;
                                                    this.group = Some(draft.group_id.clone());
                                                    this.selected = draft.entry_id.clone();
                                                    this.install_editor(
                                                        reply.session,
                                                        draft,
                                                        window,
                                                        cx,
                                                    );
                                                }
                                                _ => this.error = Some("error"),
                                            }
                                        }
                                        this.restore = false;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("discard-restored")
                                    .label(tr("discard"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.root_focus.focus(window, cx);
                                        if let Some(token) = &this.session
                                            && this.service.cancel_draft(token).is_err()
                                        {
                                            this.error = Some("error");
                                        }
                                        this.restore = false;
                                        cx.notify();
                                    })),
                            ),
                    )
                    .into_any_element()
            } else if let Some(form) = &self.form {
                v_flex()
                    .gap_4()
                    .child(div().text_lg().child(tr(match form {
                        Form::Database => "create_db",
                        Form::Group(_) => "add_group",
                        Form::Rename(_) => "edit_group",
                    })))
                    .child(Input::new(&self.form_input).aria_label(tr("name")))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("form-commit")
                                    .label(tr(if matches!(form, Form::Rename(_)) {
                                        "save"
                                    } else {
                                        "create"
                                    }))
                                    .primary()
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.commit_form(window, cx)
                                    })),
                            )
                            .child(Button::new("form-cancel").label(tr("cancel")).on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.root_focus.focus(window, cx);
                                    this.form = None;
                                    this.error = None;
                                    cx.notify();
                                }),
                            )),
                    )
                    .into_any_element()
            } else {
                return None;
            };
        Some(
            div()
                .absolute()
                .occlude()
                .inset_0()
                .bg(gpui_kit::rgba(0x00000066))
                .flex()
                .items_center()
                .justify_center()
                .child(
                    v_flex()
                        .w(px(540.))
                        .p_6()
                        .gap_3()
                        .rounded_lg()
                        .border_1()
                        .border_color(cx.theme().border)
                        .bg(cx.theme().background)
                        .child(content)
                        .when_some(self.error, |el, error| {
                            el.child(div().text_color(cx.theme().danger).child(tr(error)))
                        })
                        .focus_trap("modal-focus", &self.modal_focus),
                )
                .into_any_element(),
        )
    }
}
impl Render for Client {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // These subscriptions own only navigation/theme listeners. Editor subscriptions are dropped on lock.
        let _ = &self.subscriptions;
        v_flex()
            .track_focus(&self.root_focus)
            .key_context("Taypeer")
            .on_action(cx.listener(|this, _: &FocusSearch, window, cx| {
                if this.session.is_some()
                    && this.form.is_none()
                    && this.pending.is_none()
                    && !this.settings
                    && !this.restore
                {
                    this.search.update(cx, |state, cx| state.focus(window, cx));
                }
            }))
            .on_action(cx.listener(|this, _: &SaveEntry, window, cx| {
                if this.form.is_none() && !this.settings && this.pending.is_none() {
                    this.save(window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &LockDatabase, window, cx| this.lock(window, cx)))
            .on_action(cx.listener(|this, _: &CancelEditing, window, cx| {
                if this.settings {
                    this.settings = false;
                    this.root_focus.focus(window, cx);
                } else if this.form.is_some() {
                    this.form = None;
                    this.root_focus.focus(window, cx);
                } else if this.pending.is_some() {
                    this.pending = None;
                    this.root_focus.focus(window, cx);
                } else if this.editor.is_some() {
                    this.navigate(Navigation::Cancel, window, cx);
                }
                cx.notify();
            }))
            .relative()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(self.header(cx))
            .child(
                div()
                    .px_3()
                    .py_1()
                    .text_xs()
                    .text_color(cx.theme().warning)
                    .child(tr("demo")),
            )
            .child(div().flex_1().min_h_0().child(if self.session.is_some() {
                self.workspace(cx)
            } else {
                self.unlock_panel(cx)
            }))
            .when_some(self.error, |el, error| {
                el.child(
                    div()
                        .px_3()
                        .py_1()
                        .text_color(cx.theme().danger)
                        .child(tr(error)),
                )
            })
            .child(
                div()
                    .px_3()
                    .py_1()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(tr("memory")),
            )
            .children(self.overlay(cx))
    }
}
struct ProductAssets;
impl AssetSource for ProductAssets {
    fn load(&self, path: &str) -> gpui_kit::Result<Option<std::borrow::Cow<'static, [u8]>>> {
        let svg: Option<&'static [u8]> = match path {
            "product/pencil.svg" => Some(br#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"><path d="M21.174 6.812a1 1 0 0 0-3.986-3.986L3.842 16.174a2 2 0 0 0-.5.83l-1.321 4.352a.5.5 0 0 0 .623.622l4.353-1.32a2 2 0 0 0 .83-.497z"/><path d="m15 5 4 4"/></svg>"#),
            "product/x.svg" => Some(br#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round"><path d="m18 6-12 12M6 6l12 12"/></svg>"#),
            _ => None,
        };
        if let Some(svg) = svg {
            Ok(Some(std::borrow::Cow::Borrowed(svg)))
        } else {
            Assets.load(path)
        }
    }
    fn list(&self, path: &str) -> gpui_kit::Result<Vec<SharedString>> {
        Assets.list(path)
    }
}

pub fn run() {
    gpui_kit::application()
        .with_assets(ProductAssets)
        .run(|cx| {
            gpui_kit::init(cx);
            cx.bind_keys([
                KeyBinding::new("cmd-f", FocusSearch, Some("Taypeer")),
                KeyBinding::new("up", GroupUp, Some("TaypeerGroups")),
                KeyBinding::new("down", GroupDown, Some("TaypeerGroups")),
                KeyBinding::new("left", GroupLeft, Some("TaypeerGroups")),
                KeyBinding::new("right", GroupRight, Some("TaypeerGroups")),
                KeyBinding::new("enter", GroupEnter, Some("TaypeerGroups")),
                KeyBinding::new("up", EntryUp, Some("TaypeerEntries")),
                KeyBinding::new("down", EntryDown, Some("TaypeerEntries")),
                KeyBinding::new("enter", EntryEnter, Some("TaypeerEntries")),
                KeyBinding::new("cmd-s", SaveEntry, Some("Taypeer")),
                KeyBinding::new("cmd-l", LockDatabase, Some("Taypeer")),
                KeyBinding::new("escape", CancelEditing, Some("Taypeer")),
            ]);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::centered(size(px(1320.), px(820.)), cx)),
                window_min_size: Some(size(px(1100.), px(720.))),
                titlebar: Some(TitlebarOptions {
                    title: Some("Taypeer · Demo".into()),
                    ..Default::default()
                }),
                ..Default::default()
            };
            cx.spawn(async move |cx| {
                cx.open_window(options, |window, cx| {
                    let view = cx.new(|cx| Client::new(window, cx));
                    cx.activate(true);
                    cx.new(|cx| Root::new(view, window, cx))
                })
                .expect("Could not open Taypeer window");
            })
            .detach();
        });
}
