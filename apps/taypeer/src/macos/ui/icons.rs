//! Shared local icon picker for entries and groups.
use super::style::*;
use gpui_kit::{
    component::{button::*, *},
    *,
};
use std::rc::Rc;

pub(super) const ICONS: [&str; 18] = [
    "key-round",
    "globe",
    "folder",
    "server",
    "mail",
    "user",
    "heart",
    "star",
    "shield",
    "laptop",
    "terminal",
    "credit-card",
    "wallet",
    "home",
    "bookmark",
    "smartphone",
    "users",
    "database",
];
pub(super) fn choose(
    selected: String,
    on_select: impl Fn(&str, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    let on_select = Rc::new(on_select);
    window.open_dialog(cx, move |dialog, _, _| {
        dialog
            .title(tr("ui.choose_icon"))
            .width(px(480.))
            .child(h_flex().flex_wrap().gap_2().children(ICONS.map(|name| {
                let on_select = on_select.clone();
                let label = tr(&format!("ui.icon.{name}"));
                Button::new(name)
                    .ghost()
                    .selected(name == selected)
                    .icon(icon(name))
                    .tooltip(label.clone())
                    .accessibility_label(label)
                    .w(rems(3.))
                    .h(rems(3.))
                    .on_click(move |_, window, cx| {
                        on_select(name, cx);
                        window.close_dialog(cx);
                    })
            })))
            .footer(
                Button::new("cancel-icon")
                    .label(tr("cancel"))
                    .on_click(|_, window, cx| window.close_dialog(cx)),
            )
    });
}
