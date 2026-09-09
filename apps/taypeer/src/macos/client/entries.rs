//! Entry list, search input and list keyboard navigation.

use super::{Client, Navigation};
use crate::macos::actions::{EntryDown, EntryEnter, EntryUp};
use crate::macos::common::tr;
use gpui_kit::component::input::Input;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{button::*, *};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

impl Client {
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

    pub(super) fn entries_panel(&self, cx: &mut Context<Self>) -> AnyElement {
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
}
