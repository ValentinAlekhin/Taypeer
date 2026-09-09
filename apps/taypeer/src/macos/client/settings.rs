//! Non-secret preferences and their GPUI appearance adapter.

use super::Client;
use crate::macos::common::tr;
use crate::preferences::{FONT_SIZES, Language, ThemePreference};
use gpui_kit::component::{button::*, *};
use gpui_kit::*;
use std::collections::BTreeMap;

impl Client {
    pub(super) fn apply_theme(&self, window: &mut Window, cx: &mut App) {
        let dark = self.prefs.theme.is_dark(matches!(
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
        let palettes: BTreeMap<String, BTreeMap<String, u32>> =
            toml::from_str(include_str!("../../../../../resources/theme.toml"))
                .expect("validated embedded palette");
        let palette_key = if dark {
            ThemePreference::Dark
        } else {
            ThemePreference::Light
        };
        let palette = &palettes[palette_key.key()];
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

    pub(super) fn save_preferences(&mut self) {
        if self.invalid_prefs {
            self.error = Some("prefs_error");
        } else if self.prefs.save().is_err() {
            self.error = Some("prefs_write");
        }
    }

    pub(super) fn settings_content(&self, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .gap_4()
            .child(div().text_lg().child(tr("settings")))
            .child(tr("language"))
            .child(h_flex().gap_2().children(Language::ALL.map(|locale| {
                Button::new(locale.code())
                    .label(locale.label())
                    .selected(self.prefs.language == locale)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.set_language(locale, window, cx);
                    }))
            })))
            .child(tr("theme"))
            .child(h_flex().gap_2().children(ThemePreference::ALL.map(|theme| {
                Button::new(theme.key())
                    .label(tr(theme.key()))
                    .selected(self.prefs.theme == theme)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.set_theme(theme, window, cx);
                    }))
            })))
            .child(tr("font"))
            .child(h_flex().gap_2().children(FONT_SIZES.map(|size| {
                Button::new(("font", size as usize))
                    .label(size.to_string())
                    .selected(self.prefs.font_size == size)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.set_font_size(size, window, cx);
                    }))
            })))
            .child(
                Button::new("close-settings")
                    .label(tr("close"))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.close_settings(window, cx);
                    })),
            )
            .into_any_element()
    }

    pub(super) fn set_language(
        &mut self,
        locale: Language,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.prefs.language = locale;
        rust_i18n::set_locale(locale.code());
        self.update_placeholders(window, cx);
        self.save_preferences();
        cx.notify();
    }

    pub(super) fn set_theme(
        &mut self,
        theme: ThemePreference,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.prefs.theme = theme;
        self.save_preferences();
        self.apply_theme(window, cx);
        cx.notify();
    }

    pub(super) fn set_font_size(&mut self, size: u8, window: &mut Window, cx: &mut Context<Self>) {
        self.prefs.font_size = size;
        self.save_preferences();
        self.apply_theme(window, cx);
        cx.notify();
    }

    pub(super) fn close_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.root_focus.focus(window, cx);
        self.settings = false;
        cx.notify();
    }
}
