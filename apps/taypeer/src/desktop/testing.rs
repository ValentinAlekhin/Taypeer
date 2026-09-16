//! Feature-gated sessions for public synthetic UI scenarios. No store mutation API.
use super::{LaunchProfile, TestLaunch, actions, platform::Platform, ui};
use gpui_kit::{component::Root, test::TestWindowExt, *};
use std::{
    path::Path,
    time::{Duration, Instant},
};

/// A real AppView hosted by GPUI's invisible test platform.
pub struct Session {
    /// GPUI context for native test events and file-dialog responses.
    cx: HeadlessAppContext,
    picker: super::file_picker::fixture::ScriptedPicker,
    png: bool,
    artifact: std::path::PathBuf,
    /// Root window; use GPUI Kit's test extension for interactions.
    pub window: AnyWindowHandle,
    view: WeakEntity<ui::AppView>,
    step: &'static str,
}
impl Session {
    /// Start using explicit isolated paths; callers retain ownership of their temporary directory.
    pub fn new(directory: &Path, worker: &Path, scenario: &'static str) -> Self {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        assert!(
            scenario
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        );
        let artifact = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../artifacts/ui")
            .join(scenario)
            .join(format!("session-{}", NEXT.fetch_add(1, Ordering::Relaxed)));
        let profile = directory.join("profile");
        if !profile.join("device-ui.json").exists() {
            {
                let mut settings = crate::local_settings::LocalSettings::default();
                settings.relay = crate::local_settings::RelayPreference::Disabled;
                settings.save(&profile)
            }
            .expect("prepare isolated device settings");
        }
        let png = std::env::var("TAYPEER_UI_PNG").is_ok_and(|value| value == "1");
        let mut cx = HeadlessAppContext::with_platform(
            gpui_kit::platform::current_platform(true).text_system(),
            std::sync::Arc::new(super::assets::ProductAssets),
            move || {
                if png {
                    gpui_kit::platform::current_headless_renderer()
                } else {
                    None
                }
            },
        );
        let picker = super::file_picker::fixture::ScriptedPicker::default();
        cx.update(|cx| {
            cx.set_global(LaunchProfile::new(Some(profile)));
            cx.set_global(TestLaunch::new(
                directory.join("preferences.toml"),
                worker.to_owned(),
            ));
            gpui_kit::init(cx);
            cx.set_global(Platform::fixture());
            picker.install(cx);
            actions::bind(cx);
            ui::bind(cx);
        });
        let mut view_handle = None;
        let window = cx
            .open_window(size(px(1320.), px(820.)), |window, cx| {
                let view = cx.new(|cx| ui::AppView::new(window, cx));
                view_handle = Some(view.downgrade());
                cx.new(|cx| Root::new(view, window, cx))
            })
            .expect("headless window")
            .into();
        let mut session = Self {
            cx,
            picker,
            png,
            artifact,
            window,
            view: view_handle.expect("created AppView"),
            step: "launch",
        };
        session.pump();
        if png {
            session
                .cx
                .capture_screenshot(session.window)
                .expect("PNG requested, but offscreen Metal rendering is unavailable");
        }
        session
    }
    /// Queue an explicit Save response before the corresponding UI action.
    pub fn save_path(&self, path: Option<std::path::PathBuf>) {
        self.picker.save(path);
    }
    /// Queue an explicit Open response before the corresponding UI action.
    pub fn open_paths(&self, paths: Option<Vec<std::path::PathBuf>>) {
        self.picker.open(paths);
    }
    /// Assert that the UI actually requested the expected file dialog.
    pub fn assert_dialogs_consumed(&self) {
        self.picker.assert_consumed();
    }
    /// Run queued GPUI events and real background work without treating virtual time as I/O completion.
    pub fn pump(&mut self) {
        self.cx.advance_clock(Duration::from_millis(10));
        self.cx.run_until_parked();
        self.update(|window, cx| window.render_frame(cx));
    }
    /// Perform an operation on the real window, without exposing its stores.
    pub fn update<R>(&mut self, operation: impl FnOnce(&mut Window, &mut App) -> R) -> R {
        self.cx
            .update_window(self.window, |_, window, cx| operation(window, cx))
            .expect("UI test window remains open")
    }
    /// Wait for observable UI state with a wall-clock deadline. Step names must contain no data.
    pub fn wait(
        &mut self,
        step: &'static str,
        mut condition: impl FnMut(&mut Window, &mut App) -> bool,
    ) {
        self.step = step;
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            self.pump();
            if self.update(|window, cx| condition(window, cx)) {
                return;
            }
            if Instant::now() >= deadline {
                panic!("{}", self.diagnostics());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    /// Identify an interaction for failure diagnostics. Never pass fixture values here.
    pub fn step(&mut self, step: &'static str) {
        self.step = step;
    }

    fn diagnostics(&mut self) -> String {
        let paths = self
            .cx
            .update_window(self.window, |_, window, _| {
                let mut paths: Vec<_> = gpui_kit::base::test_support::snapshots(window)
                    .iter()
                    .map(|element| {
                        format!(
                            "{:?}: visible={}, disabled={:?}",
                            element.path().last(),
                            element.visible(),
                            element.disabled()
                        )
                    })
                    .collect();
                paths.sort();
                paths.join("\n")
            })
            .unwrap_or_else(|_| "window closed".to_owned());
        let status = self.cx.update(|cx| {
            self.view
                .upgrade()
                .map(|view| view.read(cx).test_status(cx))
                .unwrap_or_else(|| "AppView closed".to_owned())
        });
        format!("UI step: {}; {status}; values omitted\n{paths}", self.step)
    }
    fn save_failure(&mut self) {
        let diagnostics = self.diagnostics();
        if std::fs::create_dir_all(self.artifact.parent().expect("artifact parent"))
            .and_then(|()| std::fs::write(self.artifact.with_extension("txt"), diagnostics))
            .is_err()
        {
            eprintln!("UI failure diagnostics could not be saved");
        }
    }
    fn save_png(&mut self) {
        if !self.png {
            return;
        }
        let allowed = self
            .cx
            .update_window(self.window, |_, window, cx| {
                self.view
                    .upgrade()
                    .is_some_and(|view| view.read(cx).test_capture_allowed(cx))
                    && [
                        "dialog-confirm",
                        "invitation-code",
                        "unlock-password",
                        "field-password",
                    ]
                    .iter()
                    .all(|id| window.try_find(*id).is_none())
            })
            .unwrap_or(false);
        if !allowed {
            eprintln!("PNG withheld: sensitive UI or closed window; use safe text diagnostics");
            return;
        }
        let result = self.cx.capture_screenshot(self.window).and_then(|image| {
            std::fs::create_dir_all(self.artifact.parent().expect("artifact parent"))?;
            image.save(self.artifact.with_extension("png"))?;
            Ok(())
        });
        if result.is_err() {
            eprintln!("PNG capture failed: Metal rendering or artifact output unavailable");
        }
    }
    /// Wait until the UI has acknowledged queued editor work; this does not assert persistence.
    pub fn wait_idle(&mut self) {
        let view = self.view.clone();
        self.wait("editor acknowledgments", |_, cx| {
            view.upgrade().expect("AppView").read(cx).test_idle(cx)
        });
    }
    /// Capture only worker liveness handles, without database contents or mutation commands.
    pub fn worker_controls(&mut self) -> Vec<taypeer_runtime::WorkerControl> {
        self.cx.update(|cx| {
            self.view
                .upgrade()
                .map(|view| view.read(cx).test_controls(cx))
                .unwrap_or_default()
        })
    }
    /// Deliver an OS lock event through the platform boundary.
    pub fn system_lock(&mut self) {
        self.cx.update(|cx| cx.global::<Platform>().fixture_lock());
    }
    /// Transfer the fixture helper's clipboard to another GPUI test platform.
    pub fn transfer_clipboard_to(&mut self, other: &mut Session) {
        let text = self
            .cx
            .update(|cx| cx.global::<Platform>().fixture_clipboard());
        other
            .cx
            .update(|cx| cx.write_to_clipboard(ClipboardItem::new_string(text)));
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        if std::thread::panicking() {
            self.save_failure();
        }
        self.save_png();
        let controls = self.worker_controls();
        self.cx.update(App::shutdown);
        for control in controls {
            // Teardown revokes and reaps even when a scenario unwinds after an assertion.
            control.invalidate(taypeer_runtime::session::LockReason::HostExited);
            let _ = control.wait_closed();
        }
    }
}
