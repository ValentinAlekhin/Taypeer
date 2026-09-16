//! Product theme tokens projected into GPUI Kit and Base.
use gpui_kit::{component::*, *};
use std::{collections::BTreeMap, sync::LazyLock};

const TABLE_SELECTION_ALPHA: f32 = 0.2;

static PALETTES: LazyLock<BTreeMap<String, BTreeMap<String, u32>>> = LazyLock::new(|| {
    toml::from_str(include_str!("../../../../../resources/theme.toml"))
        .expect("validated embedded theme")
});

/// Apply product colors and the rem anchor; refresh Base and the window together.
pub fn apply(dark: bool, font_size: u8, window: &mut Window, cx: &mut App) {
    Theme::change(
        if dark {
            ThemeMode::Dark
        } else {
            ThemeMode::Light
        },
        None,
        cx,
    );
    let palette = &PALETTES[if dark { "dark" } else { "light" }];
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
    theme.accent = color("selection");
    theme.accent_foreground = color("foreground");
    theme.button = color("panel");
    theme.button_foreground = color("foreground");
    theme.button_hover = color("selection");
    theme.button_primary = color("primary");
    theme.button_primary_foreground = color("on_primary");
    theme.colors.list = color("background");
    theme.list_active = color("selection");
    theme.list_active_border = color("selection");
    theme.list_hover = color("chrome");
    theme.table = color("panel");
    theme.table_head = color("chrome");
    theme.table_head_foreground = color("muted");
    theme.table_active = color("selection");
    theme.table_active_border = color("selection");
    theme.table_hover = color("chrome");
    theme.table_row_border = color("border");
    theme.popover = color("panel");
    theme.popover_foreground = color("foreground");
    theme.title_bar = color("chrome");
    theme.title_bar_border = color("border");
    theme.muted = color("chrome");
    theme.tokens = theme.colors.into();
    // DataTable paints this token over the row contents, so it must remain translucent.
    theme.tokens.table_active = color("selection").alpha(TABLE_SELECTION_ALPHA).into();
    theme.font_size = px(font_size as f32);
    Theme::sync_base(cx);
    window.refresh();
}
