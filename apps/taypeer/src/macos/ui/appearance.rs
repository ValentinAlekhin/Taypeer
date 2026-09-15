//! Persistent device appearance and GPUI Kit theme adaptation.

use crate::preferences::{Language, Preferences, ThemePreference};
use gpui_kit::{component::*, *};
use std::{collections::BTreeMap, sync::LazyLock};

static PALETTES: LazyLock<BTreeMap<String, BTreeMap<String, u32>>> = LazyLock::new(|| {
    toml::from_str(include_str!("../../../../../resources/theme.toml"))
        .expect("validated embedded theme")
});

pub(super) struct PreferencesStore {
    values: Preferences,
    invalid_file: bool,
    error: Option<&'static str>,
}
impl PreferencesStore {
    pub fn load() -> Self {
        let (values, invalid_file) = Preferences::load();
        Self {
            values,
            invalid_file,
            error: invalid_file.then_some("prefs_error"),
        }
    }
    pub fn values(&self) -> &Preferences {
        &self.values
    }
    pub fn error(&self) -> Option<&'static str> {
        self.error
    }
    pub fn set_language(
        &mut self,
        language: Language,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.values.language = language;
        rust_i18n::set_locale(language.code());
        self.persist();
        self.apply(window, cx);
        cx.notify();
    }
    pub fn set_theme(
        &mut self,
        theme: ThemePreference,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.values.theme = theme;
        self.persist();
        self.apply(window, cx);
        cx.notify();
    }
    pub fn set_font(&mut self, size: u8, window: &mut Window, cx: &mut Context<Self>) {
        self.values.font_size = size;
        self.persist();
        self.apply(window, cx);
        cx.notify();
    }
    pub fn resize(&mut self, group: f32, entry: Option<f32>, cx: &mut Context<Self>) {
        self.values.group_width = group.clamp(192., 280.);
        if let Some(entry) = entry {
            self.values.entry_width = entry.clamp(380., 560.);
        }
        self.persist();
        cx.notify();
    }
    fn persist(&mut self) {
        self.error = if self.invalid_file {
            Some("prefs_error")
        } else if self.values.save().is_err() {
            Some("prefs_write")
        } else {
            None
        };
    }
    pub fn apply(&self, window: &mut Window, cx: &mut App) {
        let dark = self.values.theme.is_dark(matches!(
            window.appearance(),
            WindowAppearance::Dark | WindowAppearance::VibrantDark
        ));
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
        theme.font_size = px(self.values.font_size as f32);
        theme.font_family = ".AppleSystemUIFont".into();
        Theme::sync_base(cx);
        window.refresh();
    }
}
