//! Persistent device appearance and GPUI Kit theme adaptation.

use crate::preferences::{Language, Preferences, ThemePreference};
use gpui_kit::*;
/// Retained appearance owner; serializes background writes and applies live theme changes.
pub struct PreferencesStore {
    values: Preferences,
    path: Option<std::path::PathBuf>,
    invalid_file: bool,
    error: Option<&'static str>,
    revision: u64,
    saved_revision: u64,
    writer: Option<Task<()>>,
    stopping: bool,
    _quit: Subscription,
}
impl PreferencesStore {
    /// Load a small startup snapshot from an explicit path; invalid files remain untouched.
    pub fn load(path: Option<std::path::PathBuf>, cx: &mut Context<Self>) -> Self {
        let (values, invalid_file) = match &path {
            Some(path) => match Preferences::load_from(path) {
                Ok(values) => (values, false),
                Err(_) => (Preferences::default(), true),
            },
            None => (Preferences::default(), true),
        };
        Self {
            values,
            path,
            invalid_file,
            revision: 0,
            saved_revision: 0,
            writer: None,
            stopping: false,
            _quit: cx.on_app_quit(|this, cx| this.finish_writes(cx)),
            error: invalid_file.then_some("prefs_error"),
        }
    }
    /// Read the live appearance snapshot.
    pub fn values(&self) -> &Preferences {
        &self.values
    }
    /// Localized startup or persistence failure, if present.
    pub fn error(&self) -> Option<&'static str> {
        self.error
    }
    /// Change locale, queue persistence, and refresh the window.
    pub fn set_language(
        &mut self,
        language: Language,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.values.language = language;
        rust_i18n::set_locale(language.code());
        self.persist(cx);
        self.apply(window, cx);
        cx.notify();
    }
    /// Change appearance policy, queue persistence, and refresh the window.
    pub fn set_theme(
        &mut self,
        theme: ThemePreference,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.values.theme = theme;
        self.persist(cx);
        self.apply(window, cx);
        cx.notify();
    }
    /// Change the rem anchor, queue persistence, and refresh the window.
    pub fn set_font(&mut self, size: u8, window: &mut Window, cx: &mut Context<Self>) {
        self.values.font_size = size;
        self.persist(cx);
        self.apply(window, cx);
        cx.notify();
    }
    /// Persist measured panel widths converted back to the default base scale.
    pub fn resize(&mut self, group: f32, inspector: Option<f32>, cx: &mut Context<Self>) {
        self.values.group_width = group.clamp(192., 280.);
        if let Some(inspector) = inspector {
            self.values.inspector_width = inspector.clamp(380., 10000.);
        }
        self.persist(cx);
        cx.notify();
    }
    fn persist(&mut self, cx: &mut Context<Self>) {
        self.revision += 1;
        self.start_write(cx);
    }

    fn start_write(&mut self, cx: &mut Context<Self>) {
        if self.stopping
            || self.invalid_file
            || self.writer.is_some()
            || self.saved_revision == self.revision
        {
            return;
        }
        let Some(path) = self.path.clone() else {
            return;
        };
        let values = self.values.clone();
        let revision = self.revision;
        let task = cx
            .background_executor()
            .spawn(async move { values.save_to(&path) });
        // One writer serializes atomic replacements. Changes during a write are
        // coalesced into the next snapshot; an older completion cannot mark them saved.
        self.writer = Some(cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.writer = None;
                this.error = result.err().map(|_| "prefs_write");
                this.saved_revision = revision;
                if this.revision != revision {
                    this.start_write(cx);
                }
                cx.notify();
            });
        }));
    }
    fn finish_writes(
        &mut self,
        cx: &mut Context<Self>,
    ) -> impl std::future::Future<Output = ()> + use<> {
        self.stopping = true;
        let pending = self.writer.take();
        let latest = (!self.invalid_file && self.saved_revision != self.revision)
            .then(|| self.path.clone().map(|path| (path, self.values.clone())))
            .flatten();
        let executor = cx.background_executor().clone();
        async move {
            // GPUI gives quit hooks a bounded grace period. Finish an older write
            // before publishing the final snapshot, without spawning more UI work.
            if let Some(pending) = pending {
                pending.await;
            }
            if let Some((path, values)) = latest {
                let _ = executor.spawn(async move { values.save_to(&path) }).await;
            }
        }
    }

    /// Project the current preferences into product theme tokens and refresh Base.
    pub fn apply(&self, window: &mut Window, cx: &mut App) {
        let dark = self.values.theme.is_dark(matches!(
            window.appearance(),
            WindowAppearance::Dark | WindowAppearance::VibrantDark
        ));
        taypeer_ui::theme::apply(dark, self.values.font_size, window, cx);
    }
}
