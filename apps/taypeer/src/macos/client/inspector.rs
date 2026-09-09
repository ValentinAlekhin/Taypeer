//! Inspector frame and tab navigation; content is rendered by editor/detail modules.

use super::{Client, EntryTab, Navigation};
use crate::macos::common::tr;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{button::*, *};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

impl Client {
    pub(super) fn inspector(&self, cx: &mut Context<Self>) -> AnyElement {
        let view = self.selected_view();
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
                        EntryTab::ALL
                            .into_iter()
                            .filter(|tab| self.editor.is_none() || *tab != EntryTab::History)
                            .map(|tab| {
                                Button::new(SharedString::from(format!("tab-{}", tab.key())))
                                    .label(tr(tab.key()))
                                    .selected(self.tab == tab)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.select_tab(tab, cx);
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
                                this.show_current(window, cx);
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

    pub(super) fn select_tab(&mut self, tab: EntryTab, cx: &mut Context<Self>) {
        self.tab = tab;
        self.revealed.clear();
        cx.notify();
    }

    pub(super) fn show_current(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.root_focus.focus(window, cx);
        self.revision = None;
        self.revealed.clear();
        cx.notify();
    }
}
