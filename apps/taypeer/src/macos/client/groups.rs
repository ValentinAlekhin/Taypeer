//! Group tree rendering and keyboard navigation in visible tree order.

use super::{Client, Form, Navigation};
use crate::macos::actions::{GroupDown, GroupEnter, GroupLeft, GroupRight, GroupUp};
use crate::macos::common::tr;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{button::*, *};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use taypeer_services::{GroupId, GroupSummary};

impl Client {
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

    pub(super) fn groups_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let groups = self.groups();
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
}
