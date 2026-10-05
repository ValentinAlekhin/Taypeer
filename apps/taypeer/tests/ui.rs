//! Real UI events against isolated public fixtures and process workers.
fn main() {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    desktop::run();
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    println!("UI scenarios require macOS or Linux; portable components are checked separately");
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod desktop {
    use gpui_kit::test::TestWindowExt;
    use std::path::Path;
    use std::time::{Duration, Instant};
    use taypeer::testing::Session;

    fn title_cell(
        window: &gpui_kit::Window,
    ) -> Option<gpui_kit::base::test_support::ElementSnapshot> {
        gpui_kit::base::test_support::snapshots(window).into_iter().find(|e| {
            e.path().last().is_some_and(|id| matches!(id,
                gpui_kit::ElementId::Name(name) if name.starts_with("entry-cell-") && name.ends_with("-name")))
        })
    }

    fn click_title(app: &mut Session) {
        app.update(|window, cx| {
            let cell = title_cell(window).expect("entry title cell");
            window.click(cell.path().last().unwrap().clone(), cx);
        });
        app.pump();
    }

    const PASSWORD: &str = "PUBLIC-UI-password-42";
    fn assert_inter_typography(window: &gpui_kit::Window, cx: &gpui_kit::App) {
        use gpui_kit::{Font, FontWeight, TextRun, component::ActiveTheme, px};

        assert_eq!(cx.theme().font_family.as_ref(), "Inter");
        let text = "Taypeer · Настройки 0123456789";
        for weight in [
            FontWeight::NORMAL,
            FontWeight::MEDIUM,
            FontWeight::SEMIBOLD,
            FontWeight::BOLD,
        ] {
            let font = Font {
                family: cx.theme().font_family.clone(),
                weight,
                ..Default::default()
            };
            let font_id = cx.text_system().resolve_font(&font);
            assert_eq!(
                cx.text_system()
                    .get_font_for_id(font_id)
                    .unwrap()
                    .family
                    .as_ref(),
                "Inter",
                "UI font must resolve without falling back to the system family"
            );
            let line = window.text_system().shape_line(
                text.into(),
                px(16.),
                &[TextRun {
                    len: text.len(),
                    font,
                    ..Default::default()
                }],
                None,
            );
            assert!(!line.runs.is_empty());
            assert!(line.width() > px(0.));
            assert!(line.runs.iter().all(|run| run.font_id == font_id));
            assert!(
                line.runs
                    .iter()
                    .flat_map(|run| &run.glyphs)
                    .all(|glyph| glyph.id.0 != 0)
            );
        }
    }

    fn start(directory: &Path, scenario: &'static str) -> Session {
        Session::new(
            directory,
            Path::new(env!("CARGO_BIN_EXE_taypeer-ui-worker")),
            scenario,
        )
    }
    fn wait_for_stable_control(app: &mut Session, id: &'static str) {
        let mut previous = None;
        let mut stable_since = Instant::now();
        // Kit's click renders between locating the target and mouse-down. Its
        // dialog animation uses wall time, not Session's virtual executor clock.
        app.wait(id, |window, _| {
            let bounds = window
                .try_find(id)
                .filter(|item| item.visible())
                .map(|item| item.bounds());
            if bounds.is_none() || bounds != previous {
                previous = bounds;
                stable_since = Instant::now();
                return false;
            }
            stable_since.elapsed() >= Duration::from_millis(50)
        });
    }
    fn click(app: &mut Session, id: &'static str) {
        app.wait_idle();
        wait_for_stable_control(app, id);
        app.step(id);
        app.update(|window, cx| {
            assert!(window.find(id).visible(), "control must be visible: {id}");
            assert_ne!(
                window.find(id).disabled(),
                Some(true),
                "control must accept input: {id}"
            );
            window.click(id, cx);
        });
        app.pump();
    }
    fn fill_nowait(app: &mut Session, id: &'static str, value: &str) {
        wait_for_stable_control(app, id);
        app.step(id);
        app.update(|window, cx| {
            assert_ne!(
                window.find(id).disabled(),
                Some(true),
                "input enabled: {id}"
            );
            window.click(id, cx);
            window.press(&taypeer_desktop_platform::primary_shortcut("a"), cx);
        });
        app.pump();
        app.update(|window, cx| {
            if value.is_empty() {
                window.press("backspace", cx);
            } else {
                window.input(value, cx);
            }
            assert_eq!(window.find(id).focused(), Some(true));
            if !id.contains("password") {
                assert_eq!(window.find(id).value(), Some(value), "public input: {id}");
            }
        });
        app.pump();
    }
    fn fill(app: &mut Session, id: &'static str, value: &str) {
        fill_nowait(app, id, value);
        app.wait_idle();
    }
    fn create(app: &mut Session, path: &Path) -> std::path::PathBuf {
        click(app, "welcome-create");
        fill(app, "field-name", "PUBLIC database");
        fill(app, "field-password", PASSWORD);
        fill(app, "field-confirm_password", PASSWORD);
        fill(app, "field-ui.kdf_seconds", "0.5");
        click(app, "dialog-confirm");
        app.assert_dialogs_consumed();
        app.wait("database created", |window, _| {
            window.try_find("empty-add-group").is_some()
        });
        let profile = path.parent().unwrap().join("profile");
        app.wait("created path persisted", |_, _| {
            taypeer_settings_ui::local_settings::LocalSettings::load(&profile)
                .is_ok_and(|settings| !settings.recent.is_empty())
        });
        let settings = taypeer_settings_ui::local_settings::LocalSettings::load(&profile).unwrap();
        let actual = settings
            .recent
            .last()
            .expect("created catalog item")
            .path
            .clone();
        assert!(
            actual.starts_with(profile.canonicalize().unwrap()),
            "host selects internal working directory"
        );
        assert!(actual.is_file(), "creation must publish a file");
        assert!(
            !path.exists(),
            "creation must not ask for an external destination"
        );
        actual
    }
    fn create_entry(app: &mut Session) {
        click(app, "empty-add-group");
        fill(app, "field-name", "PUBLIC group");
        click(app, "close-metadata");
        app.wait("group saved", |window, _| {
            window.try_find("close-metadata").is_none()
        });
        click(app, "new-entry");
        app.wait("editor opened", |window, _| {
            window.try_find("field-name").is_some()
        });
        fill(app, "field-name", "PUBLIC entry");
        fill(app, "field-username", "PUBLIC user");
        fill(app, "field-password", "PUBLIC secret");
        click(app, "close-editor");
        app.wait("entry saved", |window, _| {
            window.try_find("edit-entry").is_some() && title_cell(window).is_some()
        });
        app.update(|window, _| {
            assert_eq!(
                title_cell(window).expect("entry title cell").label(),
                Some("PUBLIC entry")
            )
        });
    }
    fn recent_databases_survive_restart() {
        use taypeer_settings_ui::local_settings::LocalSettings;

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("PUBLIC-recent.taypeer");
        let profile = directory.path().join("profile");
        let mut app = start(directory.path(), "recent-databases");
        let path = create(&mut app, &path);
        app.wait("recent database persisted", |_, _| {
            LocalSettings::load(&profile)
                .is_ok_and(|settings| settings.recent.iter().any(|item| item.path == path))
        });
        drop(app);

        let settings = LocalSettings::load(&profile).unwrap();
        assert_eq!(settings.recent.len(), 1);
        let id = format!("recent-database-{}", settings.recent[0].database.as_str());
        std::fs::write(
            directory.path().join("preferences.toml"),
            "language = \"ru\"\ntheme = \"dark\"\nfont_size = 16\ngroup_width = 224\nentry_width = 480\n",
        )
        .unwrap();
        let mut welcome = start(directory.path(), "recent-databases-welcome");
        welcome.wait("saved recent database shown on welcome", |window, _| {
            window
                .try_find(id.clone())
                .is_some_and(|item| item.visible())
                && window.try_find("welcome-open").is_some()
                && window.try_find("unlock").is_none()
        });
        welcome.update(|window, _| {
            let open = window.find("welcome-open").bounds();
            let create = window.find("welcome-create").bounds();
            let recent = window.find(id.clone()).bounds();
            let receive = window.find("welcome-receive").bounds();
            assert_eq!(open.origin.y, create.origin.y);
            assert!(open.origin.x < create.origin.x);
            assert!(recent.origin.y > open.origin.y + open.size.height);
            assert!(receive.origin.y > recent.origin.y + recent.size.height);
            assert!(window.try_find("welcome-import").is_none());
        });
        drop(welcome);
        let mut locked = start(directory.path(), "unlock-layout");
        locked.wait("recent file ready for unlock layout", |window, _| {
            window.try_find(id.clone()).is_some()
        });
        locked.update(|window, cx| window.click(id.clone(), cx));
        locked.wait("unlock layout shown", |window, _| {
            window.try_find("unlock-back").is_some()
        });
        locked.update(|window, _| {
            let input = window.find("unlock-password").bounds();
            let back = window.find("unlock-back").bounds();
            let unlock = window.find("unlock").bounds();
            assert_eq!(window.find("unlock-password").focused(), Some(true));
            #[cfg(target_os = "macos")]
            assert!(window.find("unlock-touch-id").visible());
            #[cfg(target_os = "linux")]
            assert!(window.try_find("unlock-touch-id").is_none());
            assert!(back.origin.y > input.origin.y + input.size.height);
            assert_eq!(back.origin.y, unlock.origin.y);
            assert!(back.origin.x < unlock.origin.x);
        });
        drop(locked);
        let mut reopened = start(directory.path(), "recent-databases");
        reopened.wait("recent database visible after restart", |window, _| {
            window
                .try_find(id.clone())
                .is_some_and(|item| item.visible())
        });
        assert!(reopened.worker_controls().is_empty());
        reopened.update(|window, cx| {
            assert!(window.find("welcome-open").visible());
            assert!(window.try_find("unlock").is_none());
            assert_eq!(
                window.find(id.clone()).label(),
                Some(path.to_str().unwrap())
            );
            // Kit reports no explicit enabled flag; the real click and unlock below
            // verify that this visible control accepts input.
            window.click(id.clone(), cx);
        });
        reopened.wait("recent database requires password", |window, _| {
            window.try_find("unlock").is_some()
        });
        assert!(reopened.worker_controls().is_empty());
        fill(&mut reopened, "unlock-password", "PUBLIC discarded input");
        click(&mut reopened, "unlock-back");
        reopened.wait("back returns to recent files", |window, _| {
            window.try_find("welcome-open").is_some()
        });
        reopened.update(|window, cx| window.click(id.clone(), cx));
        reopened.wait("fresh unlock input after back", |window, _| {
            window.try_find("unlock-password").is_some()
        });
        reopened.assert_capture_safe();
        unlock(&mut reopened);
        reopened.wait("recent database opened", |window, _| {
            window.try_find("empty-add-group").is_some()
        });
    }

    fn creation_and_editing_survive_reopening() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("PUBLIC.taypeer");
        let mut app = start(directory.path(), "editing");
        let path = create(&mut app, &path);
        create_entry(&mut app);
        click(&mut app, "edit-entry");
        app.wait("edit ready", |window, _| {
            window.try_find("field-name").is_some()
        });
        fill(&mut app, "field-name", "PUBLIC edited");
        app.update(|window, cx| window.within("entry-tabs").click(1usize, cx));
        app.pump();
        app.wait("attribute action enabled", |window, _| {
            window
                .try_find("add-attribute")
                .is_some_and(|e| e.disabled() != Some(true))
        });
        click(&mut app, "add-attribute");
        app.wait("attribute form opened", |window, _| {
            window.try_find("field-ui.key").is_some()
        });
        fill(&mut app, "field-ui.key", "PUBLIC attribute");
        fill(&mut app, "field-value", "PUBLIC value");
        click(&mut app, "dialog-confirm");
        app.wait("attribute accepted", |window, _| {
            window.try_find("dialog-confirm").is_none()
        });
        click(&mut app, "close-editor");
        app.wait("edited entry saved", |window, _| {
            title_cell(window).is_some_and(|e| e.label() == Some("PUBLIC edited"))
        });
        drop(app);
        let mut reopened = start(directory.path(), "editing");
        open(&mut reopened, &path);
        reopened.wait("reopened saved entry", |window, _| {
            title_cell(window).is_some_and(|e| e.label() == Some("PUBLIC edited"))
        });
        click_title(&mut reopened);
        reopened.wait("saved details", |window, _| {
            window
                .try_find("read-None-Username")
                .is_some_and(|e| e.label() == Some("PUBLIC user"))
        });
        reopened.update(|window, cx| window.within("entry-tabs").click(1usize, cx));
        reopened.wait("saved attribute", |window, _| {
            gpui_kit::base::test_support::snapshots(window)
                .iter()
                .any(|e| e.label() == Some("PUBLIC value"))
        });
    }
    fn unlock(app: &mut Session) {
        app.wait("unlock screen", |window, _| {
            window.try_find("unlock-password").is_some()
        });
        fill(app, "unlock-password", PASSWORD);
        click(app, "unlock");
        app.wait("unlocked", |window, _| {
            window.try_find("new-entry").is_some()
        });
    }
    fn open(app: &mut Session, path: &Path) {
        app.open_paths(Some(vec![path.to_owned()]));
        app.update(|window, cx| window.press(&taypeer_desktop_platform::primary_shortcut("o"), cx));
        app.pump();
        app.assert_dialogs_consumed();
        unlock(app);
    }
    fn locking_automatically_resumes_the_encrypted_draft() {
        let directory = tempfile::tempdir().unwrap();
        let mut app = start(directory.path(), "lock");
        create(&mut app, &directory.path().join("PUBLIC.taypeer"));
        create_entry(&mut app);
        click(&mut app, "edit-entry");
        app.wait("edit ready", |window, _| {
            window.try_find("field-name").is_some()
        });
        fill(&mut app, "field-name", "PUBLIC draft");
        fill(&mut app, "field-ui.expires", "unfinished");
        click(&mut app, "load-password");
        app.update(|window, cx| window.within("field-password").click("toggle-mask", cx));
        app.wait("protected value revealed", |window, _| {
            window
                .try_find("field-password")
                .is_some_and(|e| e.value() == Some("PUBLIC secret"))
        });
        // Enqueue another real reveal response, but do not poll it before revoking access.
        app.update(|window, cx| window.click("load-password", cx));
        let controls = app.worker_controls();
        assert_eq!(controls.len(), 1);
        app.system_lock();
        app.wait("locked editor removed", |window, _| {
            window.try_find("unlock").is_some()
                && window.try_find("field-name").is_none()
                && window.try_find("field-password").is_none()
        });
        for control in controls {
            assert!(!control.is_open());
            assert!(control.wait_closed().unwrap().error.is_none());
        }
        app.system_active();
        unlock(&mut app);

        app.wait("draft restored", |window, _| {
            window
                .try_find("field-name")
                .is_some_and(|e| e.value() == Some("PUBLIC draft"))
        });
        fill(&mut app, "field-ui.expires", "");
        click(&mut app, "close-editor");
        app.wait("restored draft saved", |window, _| {
            title_cell(window).is_some_and(|e| e.label() == Some("PUBLIC draft"))
        });
    }

    fn two_devices_join_and_exchange() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let mut a = start(first.path(), "exchange-a");
        let mut b = start(second.path(), "exchange-b");
        create(&mut a, &first.path().join("PUBLIC.taypeer"));
        create_entry(&mut a);
        click(&mut a, "devices");
        a.wait("sharing available", |window, _| {
            window
                .try_find("share-database")
                .is_some_and(|e| e.disabled() != Some(true))
        });
        click(&mut a, "share-database");
        a.wait("invitation created", |window, _| {
            window.try_find("copy-invitation").is_some()
        });
        click(&mut a, "copy-invitation");
        a.transfer_clipboard_to(&mut b);
        click(&mut b, "welcome-receive");
        fill(&mut b, "invitation-password", PASSWORD);
        click(&mut b, "invitation-code");
        b.update(|window, cx| {
            window.press(&taypeer_desktop_platform::primary_shortcut("v"), cx);
        });
        b.pump();

        click(&mut b, "connect-invitation");
        b.assert_dialogs_consumed();
        a.wait("join request received", |window, _| {
            b.pump();
            window
                .try_find("approve-invite")
                .is_some_and(|e| e.disabled() != Some(true))
        });
        click(&mut a, "approve-invite");
        b.wait("received database stays locked", |window, _| {
            a.pump();
            window.try_find("unlock").is_some()
        });
        b.wait("received internal path persisted", |_, _| {
            taypeer_settings_ui::local_settings::LocalSettings::load(&second.path().join("profile"))
                .is_ok_and(|settings| !settings.recent.is_empty())
        });
        let received = taypeer_settings_ui::local_settings::LocalSettings::load(
            &second.path().join("profile"),
        )
        .unwrap()
        .recent[0]
            .path
            .clone();
        assert!(received.is_file());
        assert!(b.worker_controls().is_empty());
        unlock(&mut b);
        b.wait("received entry applied", |window, _| {
            title_cell(window).is_some_and(|e| e.label() == Some("PUBLIC entry"))
        });
        a.update(|window, cx| window.press("escape", cx));
        a.wait("clipboard confirmation cleared", |window, _| {
            window.try_find("notification").is_none()
        });
        a.pump();
        a.update(|window, cx| { let group = gpui_kit::base::test_support::snapshots(window).into_iter().find(|e| e.path().last().is_some_and(|id| matches!(id, gpui_kit::ElementId::Name(name) if name.starts_with("group-node-")))).expect("database group"); window.click(group.path().last().unwrap().clone(), cx); });
        a.wait("sender workspace", |window, _| title_cell(window).is_some());
        rename_entry(&mut a, "PUBLIC from A");
        b.wait("A edit observed by B", |window, _| {
            a.pump();
            title_cell(window).is_some_and(|e| e.label() == Some("PUBLIC from A"))
        });
        rename_entry(&mut b, "PUBLIC from B");
        a.wait("B edit observed by A", |window, _| {
            b.pump();
            title_cell(window).is_some_and(|e| e.label() == Some("PUBLIC from B"))
        });
        let controls = b.worker_controls();
        b.system_lock();
        b.wait("recipient locked", |window, _| {
            window.try_find("unlock").is_some()
        });
        for control in &controls {
            assert!(control.wait_closed().unwrap().error.is_none());
        }
        let before = std::fs::read(&received).unwrap();
        rename_entry(&mut a, "PUBLIC while locked");
        b.wait("locked ciphertext durably received", |window, _| {
            a.pump();
            window.try_find("unlock").is_some() && std::fs::read(&received).unwrap() != before
        });
        assert!(controls.iter().all(|control| !control.is_open()));
        b.update(|window, _| assert!(title_cell(window).is_none()));
        b.system_active();
        unlock(&mut b);
        b.wait("received ciphertext applied on unlock", |window, _| {
            title_cell(window).is_some_and(|e| e.label() == Some("PUBLIC while locked"))
        });
    }

    fn rename_entry(app: &mut Session, title: &str) {
        click_title(app);
        app.wait("entry details ready", |window, _| {
            window.try_find("read-None-Title").is_some()
        });
        click(app, "edit-entry");
        app.wait("edit ready", |window, _| {
            window.try_find("field-name").is_some()
        });
        fill(app, "field-name", title);
        click(app, "close-editor");
        app.wait("local edit persisted", |window, _| {
            title_cell(window).is_some_and(|e| e.label() == Some(title))
        });
    }

    fn zoom_and_edit_preserve_entry_identity() {
        let directory = tempfile::tempdir().unwrap();
        let mut app = start(directory.path(), "zoom");
        app.update(|window, cx| {
            assert!(window.find("welcome-create").visible());
            assert_inter_typography(window, cx);
        });
        create(&mut app, &directory.path().join("PUBLIC.taypeer"));
        create_entry(&mut app);
        let identity =
            app.update(|window, _| title_cell(window).unwrap().path().last().unwrap().clone());
        rename_entry(&mut app, "PUBLIC renamed");
        app.update(|window, _| {
            assert_eq!(title_cell(window).unwrap().path().last(), Some(&identity))
        });
        app.update(|window, cx| window.press(&taypeer_desktop_platform::primary_shortcut(","), cx));
        app.wait("settings opened", |window, _| {
            window.try_find("font-size").is_some()
        });
        click(&mut app, "font-size");
        app.update(|window, cx| {
            window.press("up", cx);
            window.press("enter", cx);
        });
        app.wait("zoom applied", |window, _| {
            f32::from(window.rem_size()) == 18.
        });
        app.update(|window, cx| {
            assert!(window.find("font-size").visible());
            assert_inter_typography(window, cx);
        });
        app.wait("zoom persisted", |_, _| {
            std::fs::read_to_string(directory.path().join("preferences.toml"))
                .is_ok_and(|text| text.contains("font_size = 18"))
        });
        app.update(|window, cx| window.press("escape", cx));
        app.wait("zoomed workspace", |window, _| title_cell(window).is_some());
        app.update(|window, cx| {
            assert_eq!(title_cell(window).unwrap().path().last(), Some(&identity));
            window.press(&taypeer_desktop_platform::primary_shortcut("f"), cx);
        });
        app.pump();
        app.update(|window, _| assert_eq!(window.find("search").focused(), Some(true)));
    }

    fn inspector_reveal_can_be_hidden_and_revoked() {
        let directory = tempfile::tempdir().unwrap();
        let mut app = start(directory.path(), "inspector-reveal");
        create(&mut app, &directory.path().join("PUBLIC.taypeer"));
        create_entry(&mut app);
        for _ in 0..2 {
            app.update(|window, cx| window.hover("value-None-Password", cx));
            app.pump();
            click(&mut app, "show-None-Password");
            app.wait("inspector password revealed", |window, _| {
                window
                    .try_find("read-None-Password")
                    .is_some_and(|e| e.label() == Some("PUBLIC secret"))
            });
            click(&mut app, "show-None-Password");
            app.wait("inspector password hidden", |window, _| {
                window.try_find("read-None-Password").is_none()
            });
        }
        // Revocation between the request and its completion must discard the value.
        app.update(|window, cx| window.click("show-None-Password", cx));
        app.system_lock();
        app.wait("pending inspector reveal revoked", |window, _| {
            window.try_find("unlock").is_some() && window.try_find("read-None-Password").is_none()
        });
        #[cfg(target_os = "linux")]
        {
            let revoked = app.worker_controls();
            fill(&mut app, "unlock-password", PASSWORD);
            click(&mut app, "unlock");
            app.wait_idle();
            app.update(|window, _| {
                assert!(window.find("unlock").visible());
                assert!(title_cell(window).is_none());
                assert!(window.try_find("read-None-Password").is_none());
            });
            let controls = app.worker_controls();
            assert_eq!(
                controls.len(),
                revoked.len(),
                "inactive OS must not publish a new session"
            );
            assert!(
                controls.iter().all(|control| !control.is_open()),
                "revoked workers remain closed"
            );
        }
        app.system_active();
        unlock(&mut app);
        app.wait("unlocked rows loaded", |window, _| {
            title_cell(window).is_some()
        });
        click_title(&mut app);
        app.wait("reopened inspector stays masked", |window, _| {
            window.try_find("show-None-Password").is_some()
                && window.try_find("read-None-Password").is_none()
        });
    }

    fn save_failure_can_be_retried_or_edited() {
        for changed in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("PUBLIC-retry.taypeer");
            let mut app = start(directory.path(), "save-retry");
            let path = create(&mut app, &path);
            create_entry(&mut app);
            click(&mut app, "edit-entry");
            app.wait("editor available", |window, _| {
                window.try_find("field-name").is_some()
            });
            let before = std::fs::read(&path).unwrap();
            std::fs::write(path.with_extension("fail-before"), b"PUBLIC fault").unwrap();
            fill_nowait(&mut app, "field-name", "PUBLIC first attempt");
            app.update(|window, cx| {
                window.press(&taypeer_desktop_platform::primary_shortcut("s"), cx)
            });
            app.pump();
            app.wait_idle();
            app.wait_idle();
            app.update(|window, _| {
                assert!(window.find("close-editor").visible());
                assert_ne!(window.find("close-editor").disabled(), Some(true));
                assert_eq!(
                    window.find("field-name").value(),
                    Some("PUBLIC first attempt")
                );
            });
            assert_eq!(std::fs::read(&path).unwrap(), before);
            std::fs::remove_file(path.with_extension("fail-before")).unwrap();
            let expected = if changed {
                fill(&mut app, "field-name", "PUBLIC corrected attempt");
                "PUBLIC corrected attempt"
            } else {
                "PUBLIC first attempt"
            };
            click(&mut app, "close-editor");
            app.wait("retry saved", |window, _| {
                window.try_find("edit-entry").is_some()
            });
            drop(app);
            let mut reopened = start(directory.path(), "save-retry-reopened");
            open(&mut reopened, &path);
            reopened.wait("retry persisted", |window, _| {
                title_cell(window).is_some_and(|cell| cell.label() == Some(expected))
            });
        }
    }
    fn lock_during_save_reconciles_the_committed_draft() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("PUBLIC-lock-save.taypeer");
        let mut app = start(directory.path(), "lock-save");
        let path = create(&mut app, &path);
        create_entry(&mut app);
        click(&mut app, "edit-entry");
        app.wait("editor available", |window, _| {
            window.try_find("field-name").is_some()
        });
        std::fs::write(path.with_extension("hold-after"), b"PUBLIC hold").unwrap();
        fill_nowait(&mut app, "field-name", "PUBLIC committed during lock");
        app.update(|window, cx| window.press(&taypeer_desktop_platform::primary_shortcut("s"), cx));
        app.pump();
        app.wait("storage committed before response", |_, _| {
            path.with_extension("committed").exists()
        });
        app.system_lock();
        app.wait("lock clears editor", |window, _| {
            window.try_find("unlock").is_some() && window.try_find("close-editor").is_none()
        });
        app.assert_capture_safe();
        let committed = std::fs::read(&path).unwrap();
        drop(app);
        std::fs::remove_file(path.with_extension("hold-after")).unwrap();
        let mut reopened = start(directory.path(), "lock-save-reopened");
        open(&mut reopened, &path);
        reopened.wait("committed state reopened", |window, _| {
            title_cell(window)
                .is_some_and(|cell| cell.label() == Some("PUBLIC committed during lock"))
        });
        reopened.update(|window, _| {
            assert!(window.try_find("restore-draft").is_none());
        });
        assert_eq!(std::fs::read(&path).unwrap(), committed);
    }

    fn autosave_keeps_focus_and_newer_input() {
        let directory = tempfile::tempdir().unwrap();
        let mut app = start(directory.path(), "autosave-newer-input");
        let path = create(&mut app, &directory.path().join("PUBLIC.taypeer"));
        create_entry(&mut app);
        click(&mut app, "edit-entry");
        std::fs::write(path.with_extension("hold-after"), b"PUBLIC hold").unwrap();
        fill_nowait(&mut app, "field-name", "PUBLIC captured input");
        app.update(|window, cx| window.press(&taypeer_desktop_platform::primary_shortcut("s"), cx));
        app.wait(
            "first snapshot committed while response is delayed",
            |_, _| path.with_extension("committed").exists(),
        );
        fill_nowait(&mut app, "field-name", "PUBLIC newer input");
        app.update(|window, _| {
            assert_eq!(
                window.find("field-name").value(),
                Some("PUBLIC newer input")
            );
            assert_eq!(window.find("field-name").focused(), Some(true));
            assert_ne!(window.find("field-name").disabled(), Some(true));
            assert!(window.try_find("save-entry").is_none());
        });
        std::fs::remove_file(path.with_extension("hold-after")).unwrap();
        app.wait_idle();
        app.wait("newer snapshot saved", |window, _| {
            title_cell(window).is_some_and(|cell| cell.label() == Some("PUBLIC newer input"))
        });
        app.update(|window, _| {
            assert_eq!(
                window.find("field-name").value(),
                Some("PUBLIC newer input")
            );
            assert_eq!(window.find("field-name").focused(), Some(true));
        });
        click(&mut app, "close-editor");
        drop(app);
        let confirmed = std::fs::read(&path).unwrap();
        for _ in 0..2 {
            let mut reopened = start(directory.path(), "autosave-newer-input-reopened");
            open(&mut reopened, &path);
            reopened.wait("latest input retained across reopen", |window, _| {
                title_cell(window).is_some_and(|cell| cell.label() == Some("PUBLIC newer input"))
            });
            drop(reopened);
            assert_eq!(
                std::fs::read(&path).unwrap(),
                confirmed,
                "opening must not create history or rewrite the archive"
            );
        }
    }

    fn navigation_resumes_independent_ungrouped_drafts() {
        let directory = tempfile::tempdir().unwrap();
        let mut app = start(directory.path(), "multiple-drafts");
        let path = create(&mut app, &directory.path().join("PUBLIC.taypeer"));
        click(&mut app, "new-entry");
        fill_nowait(&mut app, "field-username", "PUBLIC draft A");
        app.update(|window, cx| window.click("new-entry", cx));
        app.wait("independent empty form opened", |window, _| {
            window
                .try_find("field-username")
                .is_some_and(|field| field.value() == Some(""))
        });
        let first = app.update(|window, _| {
            gpui_kit::base::test_support::snapshots(window)
                .iter()
                .find_map(|element| match element.path().last() {
                    Some(gpui_kit::ElementId::Name(name)) if name.starts_with("draft-") => {
                        Some(name.to_string())
                    }
                    _ => None,
                })
                .expect("first retained draft")
        });
        fill_nowait(&mut app, "field-username", "PUBLIC draft B");
        app.update(|window, cx| window.click("ungrouped", cx));
        app.wait("navigation durably parked both forms without a dialog", |window, _| {
            window.try_find("field-name").is_none() && title_cell(window).is_none() && window.try_find("dialog-confirm").is_none()
                && gpui_kit::base::test_support::snapshots(window).iter().filter(|element| element.path().last().is_some_and(|id| matches!(id, gpui_kit::ElementId::Name(name) if name.starts_with("draft-")))).count() == 2
        });
        drop(app);
        let mut reopened = start(directory.path(), "multiple-drafts-reopened");
        open(&mut reopened, &path);
        reopened.wait(
            "last active encrypted form resumes automatically",
            |window, _| {
                window
                    .try_find("field-username")
                    .is_some_and(|field| field.value() == Some("PUBLIC draft B"))
            },
        );
        reopened.update(|window, cx| window.click(first.clone(), cx));
        reopened.wait("first independent form resumes", |window, _| {
            window
                .try_find("field-username")
                .is_some_and(|field| field.value() == Some("PUBLIC draft A"))
        });
        fill_nowait(&mut reopened, "field-name", "PUBLIC A");
        reopened.update(|window, cx| window.click("ungrouped", cx));
        reopened.wait(
            "valid form committed immediately on navigation",
            |window, _| {
                window.try_find("field-name").is_none()
                    && title_cell(window).is_some_and(|cell| cell.label() == Some("PUBLIC A"))
            },
        );
        let second = reopened.update(|window, _| {
            gpui_kit::base::test_support::snapshots(window)
                .iter()
                .find_map(|element| match element.path().last() {
                    Some(gpui_kit::ElementId::Name(name)) if name.starts_with("draft-") => {
                        Some(name.to_string())
                    }
                    _ => None,
                })
                .expect("second retained draft")
        });
        reopened.update(|window, cx| window.click(second.clone(), cx));
        reopened.wait("second form keeps its independent input", |window, _| {
            window
                .try_find("field-username")
                .is_some_and(|field| field.value() == Some("PUBLIC draft B"))
        });
        fill_nowait(&mut reopened, "field-name", "PUBLIC B");
        reopened.update(|window, cx| window.click("close-editor", cx));
        reopened.wait("second form committed", |window, _| {
            window.try_find("field-name").is_none()
        });
        drop(reopened);
        let mut final_open = start(directory.path(), "multiple-drafts-confirmed");
        open(&mut final_open, &path);
        final_open.wait("both ungrouped entries retained", |window, _| {
            let snapshots = gpui_kit::base::test_support::snapshots(window);
            ["PUBLIC A", "PUBLIC B"].iter().all(|label| {
                snapshots
                    .iter()
                    .any(|element| element.label() == Some(*label))
            })
        });
        final_open.update(|window, _| assert!(window.try_find("restore-draft").is_none()));
    }

    fn metadata_forms_resume_and_save_atomically() {
        let directory = tempfile::tempdir().unwrap();
        let mut app = start(directory.path(), "metadata-drafts");
        let path = create(&mut app, &directory.path().join("PUBLIC.taypeer"));
        click(&mut app, "empty-add-group");
        fill(
            &mut app,
            "field-ui.description",
            "PUBLIC unfinished group description",
        );
        click(&mut app, "close-metadata");
        app.wait(
            "incomplete group retained locally without creating an object",
            |window, _| {
                window.try_find("close-metadata").is_none()
                    && window.try_find("empty-add-group").is_some()
            },
        );
        drop(app);
        let mut resumed = start(directory.path(), "metadata-drafts-resumed");
        open(&mut resumed, &path);
        resumed.wait(
            "metadata form resumes without a restore prompt",
            |window, _| {
                window
                    .try_find("field-ui.description")
                    .is_some_and(|field| {
                        field.value() == Some("PUBLIC unfinished group description")
                    })
            },
        );
        fill_nowait(&mut resumed, "field-name", "PUBLIC resumed group");
        resumed.update(|window, cx| window.click("close-metadata", cx));
        resumed.wait("complete group durably created on close", |window, _| {
            window.try_find("close-metadata").is_none()
                && gpui_kit::base::test_support::snapshots(window)
                    .iter()
                    .any(|element| element.label() == Some("PUBLIC resumed group"))
        });
        click(&mut resumed, "settings");
        resumed.wait("device settings opened", |window, _| {
            window.try_find("font-size").is_some()
        });
        resumed.update(|window, cx| window.within("settings-tabs").click(1usize, cx));
        resumed.pump();
        resumed.wait("database settings available", |window, _| {
            window.try_find("database-info").is_some()
        });
        click(&mut resumed, "database-info");
        fill_nowait(&mut resumed, "field-name", "PUBLIC renamed database");
        fill_nowait(
            &mut resumed,
            "field-ui.description",
            "PUBLIC renamed description",
        );
        resumed.update(|window, cx| window.click("close-metadata", cx));
        resumed.wait(
            "database form closed after durable grouped patch",
            |window, _| window.try_find("close-metadata").is_none(),
        );
        click(&mut resumed, "database-history");
        resumed.wait("confirmed database history shown", |window, _| {
            gpui_kit::base::test_support::snapshots(window)
                .iter()
                .any(|element| element.label() == Some("PUBLIC renamed description"))
        });
        click(&mut resumed, "close-history");
        drop(resumed);
        let mut final_open = start(directory.path(), "metadata-drafts-confirmed");
        open(&mut final_open, &path);
        click(&mut final_open, "settings");
        final_open.wait("device settings reopened", |window, _| {
            window.try_find("font-size").is_some()
        });
        final_open.update(|window, cx| window.within("settings-tabs").click(1usize, cx));
        final_open.pump();
        final_open.wait("database metadata survives reopening", |window, _| {
            let snapshots = gpui_kit::base::test_support::snapshots(window);
            ["PUBLIC renamed database", "PUBLIC renamed description"]
                .iter()
                .all(|label| {
                    snapshots
                        .iter()
                        .any(|element| element.label() == Some(*label))
                })
        });
    }

    fn trash_is_immediate_and_undo_restores_the_selection() {
        let directory = tempfile::tempdir().unwrap();
        let mut app = start(directory.path(), "trash-undo");
        let path = create(&mut app, &directory.path().join("PUBLIC.taypeer"));
        create_entry(&mut app);
        click(&mut app, "trash-entry");
        app.wait(
            "trash takes effect without a confirmation dialog",
            |window, _| {
                title_cell(window).is_none()
                    && window.try_find("dialog-confirm").is_none()
                    && window.try_find("undo-trash").is_some()
            },
        );
        click(&mut app, "new-entry");
        fill_nowait(&mut app, "field-username", "PUBLIC input before undo");
        app.update(|window, cx| window.click("undo-trash", cx));
        app.wait("undo restores the exact entry", |window, _| {
            title_cell(window).is_some_and(|cell| cell.label() == Some("PUBLIC entry"))
        });
        drop(app);
        let mut reopened = start(directory.path(), "trash-undo-confirmed");
        open(&mut reopened, &path);
        reopened.wait("restoration is durable", |window, _| {
            title_cell(window).is_some_and(|cell| cell.label() == Some("PUBLIC entry"))
        });
    }

    fn double_write_failure_blocks_navigation_but_never_lock() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let mut app = start(directory.path(), "double-write-failure");
        let path = create(&mut app, &directory.path().join("PUBLIC.taypeer"));
        create_entry(&mut app);
        click(&mut app, "edit-entry");
        let original = std::fs::read(&path).unwrap();
        let parent = path.parent().unwrap();
        let permissions = std::fs::metadata(parent).unwrap().permissions();
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o500)).unwrap();
        fill_nowait(&mut app, "field-name", "PUBLIC unconfirmed input");
        app.update(|window, cx| window.click("close-editor", cx));
        app.wait("double failure keeps the editor and its input", |window, _| {
            window.try_find("field-name").is_some_and(|field| field.value() == Some("PUBLIC unconfirmed input"))
                && gpui_kit::base::test_support::snapshots(window).iter().any(|element| element.label() == Some("The file could not be read or saved. Check the path and access."))
        });
        app.wait_idle();
        app.update(|window, _| {
            assert!(window.find("close-editor").visible());
            assert_ne!(window.find("field-name").disabled(), Some(true));
            assert!(window.try_find("dialog-confirm").is_none());
        });
        assert_eq!(std::fs::read(&path).unwrap(), original);
        app.system_lock();
        app.wait(
            "OS lock removes plaintext even when the draft cannot be published",
            |window, _| {
                window.try_find("unlock").is_some() && window.try_find("field-name").is_none()
            },
        );
        app.assert_capture_safe();
        std::fs::set_permissions(parent, permissions).unwrap();
        drop(app);
        let mut reopened = start(directory.path(), "double-write-failure-confirmed");
        open(&mut reopened, &path);
        reopened.wait(
            "failed save never changed confirmed content",
            |window, _| title_cell(window).is_some_and(|cell| cell.label() == Some("PUBLIC entry")),
        );
    }

    fn opening_external_source_preserves_it_and_uses_the_catalog() {
        let directory = tempfile::tempdir().unwrap();
        let mut app = start(directory.path(), "external-working-copy");
        let working = create(&mut app, &directory.path().join("PUBLIC.taypeer"));
        create_entry(&mut app);
        drop(app);
        let source = directory.path().join("PUBLIC-external.taypeer");
        std::fs::copy(&working, &source).unwrap();
        let original = std::fs::read(&source).unwrap();
        let mut reopened = start(directory.path(), "external-working-copy-reopened");
        open(&mut reopened, &source);
        reopened.wait(
            "external archive opens through its confirmed working copy",
            |window, _| title_cell(window).is_some_and(|cell| cell.label() == Some("PUBLIC entry")),
        );
        rename_entry(&mut reopened, "PUBLIC internal edit");
        assert_eq!(
            std::fs::read(&source).unwrap(),
            original,
            "source must remain unchanged after editing"
        );
        reopened.wait("catalog retains the working path", |_, _| {
            taypeer_settings_ui::local_settings::LocalSettings::load(
                &directory.path().join("profile"),
            )
            .is_ok_and(|settings| settings.recent.len() == 1 && settings.recent[0].path == working)
        });
        drop(reopened);
        let mut confirmed = start(directory.path(), "external-working-copy-confirmed");
        open(&mut confirmed, &working);
        confirmed.wait("internal edit survives reopening", |window, _| {
            title_cell(window).is_some_and(|cell| cell.label() == Some("PUBLIC internal edit"))
        });
        assert_eq!(std::fs::read(&source).unwrap(), original);
    }

    pub fn run() {
        let filters: Vec<_> = std::env::args()
            .skip(1)
            .filter(|arg| !arg.starts_with("--"))
            .collect();
        let scenarios: [(&str, fn()); 14] = [
            (
                "opening_external_source_preserves_it_and_uses_the_catalog",
                opening_external_source_preserves_it_and_uses_the_catalog,
            ),
            (
                "double_write_failure_blocks_navigation_but_never_lock",
                double_write_failure_blocks_navigation_but_never_lock,
            ),
            (
                "metadata_forms_resume_and_save_atomically",
                metadata_forms_resume_and_save_atomically,
            ),
            (
                "trash_is_immediate_and_undo_restores_the_selection",
                trash_is_immediate_and_undo_restores_the_selection,
            ),
            (
                "autosave_keeps_focus_and_newer_input",
                autosave_keeps_focus_and_newer_input,
            ),
            (
                "navigation_resumes_independent_ungrouped_drafts",
                navigation_resumes_independent_ungrouped_drafts,
            ),
            (
                "save_failure_can_be_retried_or_edited",
                save_failure_can_be_retried_or_edited,
            ),
            (
                "lock_during_save_reconciles_the_committed_draft",
                lock_during_save_reconciles_the_committed_draft,
            ),
            (
                "recent_databases_survive_restart",
                recent_databases_survive_restart,
            ),
            (
                "inspector_reveal_can_be_hidden_and_revoked",
                inspector_reveal_can_be_hidden_and_revoked,
            ),
            (
                "zoom_and_edit_preserve_entry_identity",
                zoom_and_edit_preserve_entry_identity,
            ),
            (
                "creation_and_editing_survive_reopening",
                creation_and_editing_survive_reopening,
            ),
            (
                "locking_automatically_resumes_the_encrypted_draft",
                locking_automatically_resumes_the_encrypted_draft,
            ),
            (
                "two_devices_join_and_exchange",
                two_devices_join_and_exchange,
            ),
        ];
        let mut passed = 0;
        for (name, scenario) in scenarios {
            if !filters.is_empty() && !filters.iter().any(|filter| name.contains(filter)) {
                continue;
            }
            println!("running {name}");
            scenario();
            println!("passed {name}");
            passed += 1;
        }
        assert!(passed > 0, "no matching UI scenarios");
        println!("UI scenarios: {passed} passed");
    }
}
