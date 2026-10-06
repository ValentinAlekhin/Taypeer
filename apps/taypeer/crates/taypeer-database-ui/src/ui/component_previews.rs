//! Public synthetic inputs composed through the same retained product builders.

use super::{card, details, editor, history, read_value, sidebar, style::*};
use crate::ui_state::{EntryContent, EntryField, EntryTab};
use gpui_kit::{
    component::{
        button::*,
        checkbox::Checkbox,
        color_picker::ColorPickerState,
        input::{InputState, TextareaState},
        list::ListItem,
        tree::*,
        *,
    },
    prelude::FluentBuilder,
    *,
};

/// Database component groups rendered without a workspace or runtime session.
#[derive(Clone, Copy)]
pub enum DatabasePreview {
    /// Complete editable overview with native retained fields.
    EditorCard,
    /// Complete read-only overview with selectable ordinary values.
    EntryCard,
    /// Group tree, toolbar and secondary navigation.
    Sidebar,
    /// Confirmed revision rows and common version commands.
    History,
    /// Read-only custom attributes and attachments.
    AdvancedView,
    /// Editable custom attributes and attachment commands.
    AdvancedEdit,
    /// Read-only appearance with stored color preview.
    AppearanceView,
    /// Native appearance controls and stored color preview.
    AppearanceEdit,
    /// Immutable entry timestamps and identity.
    Properties,
    /// Historical overview with navigation to the current entry.
    HistorySnapshot,
}

/// Construct a retained preview from public synthetic data; never opens a database.
pub fn create(kind: DatabasePreview, window: &mut Window, cx: &mut App) -> AnyView {
    cx.new(|cx| Preview::new(kind, window, cx)).into()
}

struct Preview {
    kind: DatabasePreview,
    fields: Vec<(EntryField, Entity<InputState>)>,
    reads: Vec<(EntryField, Entity<TextareaState>)>,
    notes: Entity<TextareaState>,
    tree: Entity<TreeState>,
    extra_reads: Vec<(&'static str, Entity<TextareaState>)>,
    appearance: EntryContent,
    foreground: Entity<ColorPickerState>,
    background: Entity<ColorPickerState>,
}

fn sample(field: EntryField) -> &'static str {
    match field {
        EntryField::Title => "GitHub",
        EntryField::Username => "valentin",
        EntryField::Password => "PUBLIC visual password",
        EntryField::Url => "https://github.com",
        EntryField::Tags => "работа, разработка",
        EntryField::Expires => "Бессрочно",
        EntryField::Notes => "Основной рабочий аккаунт",
    }
}

impl Preview {
    fn new(kind: DatabasePreview, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let fields = EntryField::ALL
            .into_iter()
            .filter(|field| *field != EntryField::Notes)
            .map(|field| {
                (
                    field,
                    input(sample(field), field == EntryField::Password, window, cx),
                )
            })
            .collect();
        let reads = EntryField::ALL
            .into_iter()
            .filter(|field| *field != EntryField::Password)
            .map(|field| {
                (
                    field,
                    cx.new(|cx| {
                        let mut state = TextareaState::new(window, cx).auto_grow(1, 20);
                        state.set_value(sample(field), window, cx);
                        state
                    }),
                )
            })
            .collect();
        let notes = cx.new(|cx| {
            let mut state = TextareaState::new(window, cx).auto_grow(1, 20);
            state.set_value(sample(EntryField::Notes), window, cx);
            state
        });
        let tree = cx.new(|cx| TreeState::new(cx));
        tree.update(cx, |tree, cx| {
            let work = TreeItem::new("work", "Работа").expanded(true).children([
                TreeItem::new("services", "Сервисы"),
                TreeItem::new("infra", "Инфраструктура"),
            ]);
            tree.set_items(
                [
                    work,
                    TreeItem::new("personal", "Личное"),
                    TreeItem::new("archive", "Архив"),
                ],
                cx,
            );
            tree.set_selected_index(Some(0), cx);
        });
        let extra_reads = [
            ("attribute", "Studio".to_owned()),
            ("foreground", "#CBD2E3".to_owned()),
            ("background", "#242830".to_owned()),
            ("created", stamp(1786531200000)),
            ("modified", stamp(1788772320000)),
            ("id", "e2194a38-679e-49f0-8fc2-a087bc8a9124".to_owned()),
        ]
        .into_iter()
        .map(|(key, value)| {
            (
                key,
                cx.new(|cx| {
                    let mut input = TextareaState::new(window, cx).auto_grow(1, 20);
                    input.set_value(value, window, cx);
                    input
                }),
            )
        })
        .collect();
        let mut appearance = EntryContent::default();
        appearance.title = "GitHub".into();
        appearance.icon = "key-round".into();
        let foreground = cx.new(|cx| ColorPickerState::new(window, cx));
        let background = cx.new(|cx| ColorPickerState::new(window, cx));
        Self {
            kind,
            fields,
            reads,
            notes,
            tree,
            extra_reads,
            appearance,
            foreground,
            background,
        }
    }

    fn editor_body(&self, cx: &App) -> AnyElement {
        v_flex()
            .children(self.fields.iter().map(|(field, input)| {
                let control = entry_field(input, field.key())
                    .when(*field == EntryField::Password, |field| {
                        field.suffix(icon_button("editor-password-eye", "eye", "show"))
                    });
                let actions = match field {
                    EntryField::Password => {
                        vec![icon_button("generator", "dice-5", "ui.generator").into_any_element()]
                    }
                    EntryField::Url => {
                        vec![icon_button("favicon", "download", "ui.favicon").into_any_element()]
                    }
                    _ => Vec::new(),
                };
                editor::field_row(*field, input, control, actions, cx)
            }))
            .child(editor::notes_row(&self.notes, false, cx))
            .into_any_element()
    }

    fn read_body(&self, cx: &App) -> AnyElement {
        v_flex()
            .children(EntryField::ALL.map(|field| {
                let value = if field == EntryField::Password {
                    div()
                        .font_family(cx.theme().mono_font_family.clone())
                        .child("••••••••••••")
                        .into_any_element()
                } else {
                    let input = &self
                        .reads
                        .iter()
                        .find(|(candidate, _)| *candidate == field)
                        .expect("preview creates every ordinary field")
                        .1;
                    read_value::control(
                        input,
                        format!("read-preview-{}", field.key()).into(),
                        sample(field).into(),
                    )
                    .into_any_element()
                };
                let actions = h_flex()
                    .gap_2()
                    .when(field == EntryField::Password, |el| {
                        el.child(icon_button("preview-password-eye", "eye", "show"))
                    })
                    .child(icon_button(
                        SharedString::from(format!("preview-copy-{}", field.key())),
                        "copy",
                        "ui.copy",
                    ))
                    .into_any_element();
                card::overview_row(
                    field,
                    row(
                        field.key(),
                        read_value::frame(
                            SharedString::from(format!("value-preview-{}", field.key())),
                            value,
                            field == EntryField::Password,
                            Some(actions),
                            cx,
                        ),
                        cx,
                    ),
                    cx,
                )
            }))
            .into_any_element()
    }

    fn extra_value(&self, key: &'static str, cx: &App) -> AnyElement {
        let input = &self
            .extra_reads
            .iter()
            .find(|(candidate, _)| *candidate == key)
            .expect("preview creates every extra read control")
            .1;
        let text = input.read(cx).value().to_string();
        read_value::frame(
            SharedString::from(format!("value-preview-{key}")),
            read_value::control(input, format!("read-preview-{key}").into(), text),
            false,
            Some(
                icon_button(
                    SharedString::from(format!("copy-preview-{key}")),
                    "copy",
                    "ui.copy",
                )
                .into_any_element(),
            ),
            cx,
        )
    }

    fn advanced(&self, editing: bool, cx: &App) -> AnyElement {
        let ordinary = if editing {
            div().child("Studio").into_any_element()
        } else {
            self.extra_value("attribute", cx)
        };
        let secret = if editing {
            div().child("••••••••••••").into_any_element()
        } else {
            read_value::frame(
                "value-preview-api",
                div().child("••••••••••••"),
                true,
                Some(
                    h_flex()
                        .gap_2()
                        .child(icon_button("preview-api-eye", "eye", "show"))
                        .child(icon_button("preview-api-copy", "copy", "ui.copy"))
                        .into_any_element(),
                ),
                cx,
            )
        };
        let attribute_actions = |index: usize, protected| {
            editing.then(|| {
                h_flex()
                    .gap_2()
                    .when(protected, |el| {
                        el.child(icon_button(
                            ("reveal-attribute", index),
                            "eye",
                            "ui.reveal_current",
                        ))
                    })
                    .child(
                        Checkbox::new(("protect", index))
                            .checked(protected)
                            .accessibility_label(tr("protected"))
                            .tooltip(tr("protected")),
                    )
                    .child(icon_button(
                        ("edit-attribute", index),
                        "pencil",
                        "ui.edit_attribute",
                    ))
                    .child(icon_button(("remove-attribute", index), "x", "remove"))
                    .into_any_element()
            })
        };
        let commands = if editing {
            h_flex()
                .gap_2()
                .child(icon_button(
                    ("rename-attachment", 0usize),
                    "pencil",
                    "ui.rename_attachment",
                ))
                .child(icon_button(("remove-attachment", 0usize), "x", "remove"))
                .into_any_element()
        } else {
            icon_button("export-preview-attachment", "download", "ui.download").into_any_element()
        };
        v_flex()
            .child(details::section_header(
                "attributes",
                editing.then(|| {
                    icon_button("add-attribute", "plus", "add_attribute").into_any_element()
                }),
            ))
            .child(details::attribute_row(
                "Recovery code",
                secret,
                attribute_actions(0, true),
                cx,
            ))
            .child(details::attribute_row(
                "Организация",
                ordinary,
                attribute_actions(1, false),
                cx,
            ))
            .child(details::section_header(
                "ui.attachments",
                editing.then(|| {
                    icon_button("add-attachment", "file-plus-2", "ui.add_attachment")
                        .into_any_element()
                }),
            ))
            .child(details::attachment_row(
                "recovery-codes.txt",
                editing.then(|| "20 KiB".to_owned()),
                commands,
                cx,
            ))
            .when(editing, |el| el.child(details::attachments_hint(cx)))
            .into_any_element()
    }

    fn appearance(&self, editing: bool, cx: &App) -> AnyElement {
        let icon = details::icon_control(
            &self.appearance,
            if editing {
                vec![
                    details::choose_icon(default_button("choose-icon")).into_any_element(),
                    icon_button("image-file", "image", "ui.image_file").into_any_element(),
                    icon_button("image-url", "link", "ui.image_url").into_any_element(),
                ]
            } else {
                Vec::new()
            },
            cx,
        );
        let foreground = if editing {
            details::color_control(
                &self.foreground,
                false,
                self.appearance.foreground,
                self.appearance.foreground.map(|_| {
                    icon_button("reset-foreground", "x", "ui.clear_value").into_any_element()
                }),
            )
        } else {
            read_value::frame(
                "value-preview-foreground",
                div().child("—"),
                false,
                None,
                cx,
            )
        };
        let background = if editing {
            details::color_control(
                &self.background,
                true,
                self.appearance.background,
                self.appearance.background.map(|_| {
                    icon_button("reset-background", "x", "ui.clear_value").into_any_element()
                }),
            )
        } else {
            read_value::frame(
                "value-preview-background",
                div().child("—"),
                false,
                None,
                cx,
            )
        };
        details::appearance(&self.appearance, icon, foreground, background, cx)
    }

    fn properties(&self, cx: &App) -> AnyElement {
        details::properties(
            ["created", "modified", "id"].map(|key| self.extra_value(key, cx)),
            cx,
        )
    }

    fn card(&self, editing: bool, tab: EntryTab, body: AnyElement, cx: &App) -> AnyElement {
        let actions = if editing {
            h_flex()
                .gap_2()
                .child(
                    div()
                        .id("autosave-status")
                        .test_support()
                        .w(rems(6.))
                        .text_size(rems(12. / 16.))
                        .text_color(cx.theme().muted_foreground)
                        .child(tr("ui.autosave_saved")),
                )
                .child(icon_button("close-editor", "arrow-left", "back"))
                .child(icon_button("entry-menu", "ellipsis", "ui.entry_actions"))
        } else {
            h_flex()
                .gap_2()
                .when(
                    !matches!(self.kind, DatabasePreview::HistorySnapshot),
                    |el| el.child(icon_button("edit-entry", "pencil", "edit")),
                )
                .child(icon_button("entry-menu", "ellipsis", "ui.entry_actions"))
        };
        card::frame(cx)
            .child(card::header("GitHub", actions, cx))
            .when(
                matches!(self.kind, DatabasePreview::HistorySnapshot),
                |el| el.child(card::current_entry()),
            )
            .child(tabs(
                "entry-tabs",
                &EntryTab::ALL.map(EntryTab::key),
                EntryTab::ALL
                    .iter()
                    .position(|candidate| *candidate == tab)
                    .expect("entry tabs contain every preview tab"),
                cx,
            ))
            .child(card::body(body))
            .into_any_element()
    }

    fn sidebar(&self, cx: &App) -> AnyElement {
        sidebar::frame(cx)
            .h(rems(47.3125))
            .child(sidebar::toolbar(
                h_flex()
                    .gap_1()
                    .child(icon_button("group-menu", "ellipsis", "ui.group_actions"))
                    .child(icon_button("add-group", "folder-plus", "add_group")),
                cx,
            ))
            .child(sidebar::tree_viewport(Tree::new(
                &self.tree,
                |_, entry, selected, _, cx| {
                    let count = match entry.item().id.as_ref() {
                        "work" => 6,
                        "services" | "infra" => 2,
                        "personal" => 3,
                        _ => 0,
                    };
                    sidebar::group_row(
                        SharedString::from(format!("preview-group-{}", entry.item().id)),
                        sidebar::GroupPresentation {
                            label: entry.item().label.clone(),
                            count,
                            depth: entry.depth(),
                            selected,
                        },
                        icon("folder"),
                        div().w_4().flex_shrink_0().when(entry.is_folder(), |el| {
                            el.child(icon(if entry.is_expanded() {
                                "chevron-down"
                            } else {
                                "chevron-right"
                            }))
                        }),
                        cx,
                    )
                },
            )))
            .child(
                sidebar::navigation(cx)
                    .child(
                        ListItem::new("ungrouped").h(rems(2.25)).child(
                            h_flex()
                                .gap_3()
                                .child(icon("file"))
                                .child(tr("ui.ungrouped")),
                        ),
                    )
                    .child(
                        Button::new("trash")
                            .ghost()
                            .justify_start()
                            .icon(icon("trash-2"))
                            .label(tr("ui.trash"))
                            .disabled(true),
                    )
                    .child(
                        Button::new("devices")
                            .ghost()
                            .justify_start()
                            .icon(icon("laptop"))
                            .label(tr("ui.devices")),
                    ),
            )
            .into_any_element()
    }

    fn history(&self, cx: &App) -> AnyElement {
        let body = v_flex()
            .child(history::header(icon_button(
                "history-menu",
                "ellipsis",
                "ui.entry_actions",
            )))
            .child(history::column_header(cx))
            .children(
                [1788719520000, 1788344040000, 1786531200000]
                    .into_iter()
                    .enumerate()
                    .map(|(index, saved)| {
                        history::version(
                            ("preview-history", index),
                            saved,
                            "GitHub",
                            index == 0,
                            !index.is_multiple_of(2),
                            cx,
                        )
                    }),
            )
            .child(history::commands(
                history::command("view-version", "ui.view_revision"),
                history::command("compare", "ui.compare"),
                history::command("restore-version", "ui.restore_revision"),
            ))
            .into_any_element();
        self.card(false, EntryTab::History, body, cx)
    }
}

impl Render for Preview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        match self.kind {
            DatabasePreview::EditorCard => {
                self.card(true, EntryTab::Overview, self.editor_body(cx), cx)
            }
            DatabasePreview::EntryCard => {
                self.card(false, EntryTab::Overview, self.read_body(cx), cx)
            }
            DatabasePreview::Sidebar => self.sidebar(cx),
            DatabasePreview::History => self.history(cx),
            DatabasePreview::AdvancedView => {
                self.card(false, EntryTab::Advanced, self.advanced(false, cx), cx)
            }
            DatabasePreview::AdvancedEdit => {
                self.card(true, EntryTab::Advanced, self.advanced(true, cx), cx)
            }
            DatabasePreview::AppearanceView => {
                self.card(false, EntryTab::Appearance, self.appearance(false, cx), cx)
            }
            DatabasePreview::AppearanceEdit => {
                self.card(true, EntryTab::Appearance, self.appearance(true, cx), cx)
            }
            DatabasePreview::Properties => {
                self.card(false, EntryTab::Properties, self.properties(cx), cx)
            }
            DatabasePreview::HistorySnapshot => {
                self.card(false, EntryTab::Overview, self.read_body(cx), cx)
            }
        }
    }
}
