//! Real UI events against isolated public fixtures and process workers.
fn main() {
    #[cfg(target_os = "macos")]
    macos::run();
    #[cfg(not(target_os = "macos"))]
    println!("UI scenarios require macOS; portable components are checked separately");
}

#[cfg(target_os = "macos")]
mod macos {
    use gpui_kit::test::TestWindowExt;
    use std::path::Path;
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
    fn start(directory: &Path, scenario: &'static str) -> Session {
        Session::new(
            directory,
            Path::new(env!("CARGO_BIN_EXE_taypeer-ui-worker")),
            scenario,
        )
    }
    fn click(app: &mut Session, id: &'static str) {
        app.wait_idle();
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
    fn fill(app: &mut Session, id: &'static str, value: &str) {
        app.step(id);
        app.update(|window, cx| {
            assert_ne!(
                window.find(id).disabled(),
                Some(true),
                "input enabled: {id}"
            );
            window.click(id, cx);
            window.press("cmd-a", cx);
        });
        app.pump();
        app.update(|window, cx| {
            window.input(value, cx);
            assert_eq!(window.find(id).focused(), Some(true));
            if !id.contains("password") {
                assert_eq!(window.find(id).value(), Some(value), "public input: {id}");
            }
        });
        app.pump();
        app.wait_idle();
    }
    fn create(app: &mut Session, path: &Path) {
        click(app, "welcome-create");
        fill(app, "field-name", "PUBLIC database");
        fill(app, "field-password", PASSWORD);
        fill(app, "field-confirm_password", PASSWORD);
        fill(app, "field-ui.kdf_seconds", "0.5");
        app.save_path(Some(path.to_owned()));
        click(app, "dialog-confirm");
        app.assert_dialogs_consumed();
        app.wait("database created", |window, _| {
            window.try_find("empty-add-group").is_some()
        });
        assert!(path.is_file(), "creation must publish a file");
    }
    fn create_entry(app: &mut Session) {
        click(app, "empty-add-group");
        fill(app, "field-name", "PUBLIC group");
        click(app, "dialog-confirm");
        app.wait("group saved", |window, _| {
            window.try_find("dialog-confirm").is_none()
        });
        click(app, "new-entry");
        app.wait("editor opened", |window, _| {
            window.try_find("field-name").is_some()
        });
        fill(app, "field-name", "PUBLIC entry");
        fill(app, "field-username", "PUBLIC user");
        fill(app, "field-password", "PUBLIC secret");
        click(app, "save-entry");
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
        create(&mut app, &path);
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
            assert!(window.find("welcome-import").visible());
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
            assert!(window.find("unlock-touch-id").visible());
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
        create(&mut app, &path);
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
        click(&mut app, "save-entry");
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
        app.update(|window, cx| window.press("cmd-o", cx));
        app.pump();
        app.assert_dialogs_consumed();
        unlock(app);
    }
    fn locking_preserves_only_an_explicitly_restored_draft() {
        let directory = tempfile::tempdir().unwrap();
        let mut app = start(directory.path(), "lock");
        create(&mut app, &directory.path().join("PUBLIC.taypeer"));
        create_entry(&mut app);
        click(&mut app, "edit-entry");
        app.wait("edit ready", |window, _| {
            window.try_find("field-name").is_some()
        });
        fill(&mut app, "field-name", "PUBLIC draft");
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
        unlock(&mut app);
        app.wait("draft offered", |window, _| {
            window.try_find("restore-draft").is_some()
        });
        click(&mut app, "restore-draft");
        app.wait("draft restored", |window, _| {
            window
                .try_find("field-name")
                .is_some_and(|e| e.value() == Some("PUBLIC draft"))
        });
        click(&mut app, "save-entry");
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
        b.update(|window, cx| {
            window.click("invitation-code", cx);
            window.press("cmd-v", cx);
        });
        b.pump();
        let received = second.path().join("PUBLIC-received.taypeer");
        b.save_path(Some(received.clone()));
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
        click(app, "save-entry");
        app.wait("local edit persisted", |window, _| {
            title_cell(window).is_some_and(|e| e.label() == Some(title))
        });
    }

    fn zoom_and_edit_preserve_entry_identity() {
        let directory = tempfile::tempdir().unwrap();
        let mut app = start(directory.path(), "zoom");
        create(&mut app, &directory.path().join("PUBLIC.taypeer"));
        create_entry(&mut app);
        let identity =
            app.update(|window, _| title_cell(window).unwrap().path().last().unwrap().clone());
        rename_entry(&mut app, "PUBLIC renamed");
        app.update(|window, _| {
            assert_eq!(title_cell(window).unwrap().path().last(), Some(&identity))
        });
        app.update(|window, cx| window.press("cmd-,", cx));
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
        app.wait("zoom persisted", |_, _| {
            std::fs::read_to_string(directory.path().join("preferences.toml"))
                .is_ok_and(|text| text.contains("font_size = 18"))
        });
        app.update(|window, cx| window.press("escape", cx));
        app.wait("zoomed workspace", |window, _| title_cell(window).is_some());
        app.update(|window, cx| {
            assert_eq!(title_cell(window).unwrap().path().last(), Some(&identity));
            window.press("cmd-f", cx);
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

    pub fn run() {
        let filters: Vec<_> = std::env::args()
            .skip(1)
            .filter(|arg| !arg.starts_with("--"))
            .collect();
        let scenarios: [(&str, fn()); 6] = [
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
                "locking_preserves_only_an_explicitly_restored_draft",
                locking_preserves_only_an_explicitly_restored_draft,
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
