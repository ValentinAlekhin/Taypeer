// Included in ui.rs's desktop::card_behavior module to use the real Session helpers.

pub(super) fn editor_password_commands_preserve_keep_clear_and_typed_input() {
    use taypeer_database_ui::actions::ClearPassword;

    let directory = tempfile::tempdir().unwrap();
    let mut app = start(directory.path(), "editor-password-commands");
    let path = create(
        &mut app,
        &directory.path().join("PUBLIC-password-commands.taypeer"),
    );
    create_entry(&mut app);
    click(&mut app, "edit-entry");
    app.wait("editable overview installed", |window, _| {
        window.try_find("field-name").is_some()
    });
    let before_reveal = std::fs::read(&path).unwrap();
    click(&mut app, "editor-password-eye");
    app.wait("existing protected password loaded", |window, _| {
        window.find("field-password").value() == Some("PUBLIC secret")
    });
    click(&mut app, "editor-password-eye");
    app.wait_idle();
    assert_eq!(
        std::fs::read(&path).unwrap(),
        before_reveal,
        "reveal must leave the durable draft unchanged"
    );

    fill(&mut app, "field-password", "PUBLIC replacement password");
    click(&mut app, "editor-password-eye");
    app.update(|window, _| {
        assert!(
            window.find("field-password").value() == Some("PUBLIC replacement password"),
            "the eye must preserve typed text"
        );
    });
    click(&mut app, "editor-password-eye");
    click(&mut app, "save-entry");
    app.wait("replacement saved", |window, _| {
        window.try_find("edit-entry").is_some()
    });
    drop(app);

    let mut app = start(directory.path(), "editor-password-commands-reopened");
    open(&mut app, &path);
    click_title(&mut app);
    click(&mut app, "edit-entry");
    click(&mut app, "editor-password-eye");
    app.wait("replacement retained after reopen", |window, _| {
        window.find("field-password").value() == Some("PUBLIC replacement password")
    });
    app.update(|window, cx| {
        window.click("field-password", cx);
        window.dispatch_action(Box::new(ClearPassword), cx);
    });
    app.pump();
    app.wait(
        "explicit password clear updates native input",
        |window, _| window.find("field-password").value() == Some(""),
    );
    click(&mut app, "save-entry");
    app.wait("clear saved", |window, _| {
        window.try_find("edit-entry").is_some()
    });
    drop(app);

    let mut app = start(directory.path(), "editor-password-clear-reopened");
    open(&mut app, &path);
    click_title(&mut app);
    click(&mut app, "edit-entry");
    click(&mut app, "editor-password-eye");
    app.wait_idle();
    app.update(|window, _| {
        assert!(
            window.find("field-password").value() == Some(""),
            "explicit clear must remove the protected value durably"
        );
    });
}

pub(super) fn labels_focus_fields_without_moving_values() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = start(directory.path(), "label-focus-grid");
    let path = create(
        &mut app,
        &directory.path().join("PUBLIC-label-focus.taypeer"),
    );
    create_entry(&mut app);
    click(&mut app, "edit-entry");
    app.wait("label controls installed", |window, _| {
        window.try_find("field-name").is_some()
    });
    for (label, field, expected) in [
        ("label-name", "field-name", "PUBLIC entry"),
        ("label-username", "field-username", "PUBLIC user"),
        ("label-url", "field-url", ""),
    ] {
        let before = app.update(|window, _| window.find(field).bounds());
        click(&mut app, label);
        app.update(|window, _| {
            assert_eq!(
                window.find(field).focused(),
                Some(true),
                "label focuses its retained input"
            );
            assert_eq!(window.find(field).value(), Some(expected));
            assert_eq!(
                window.find(field).bounds(),
                before,
                "focus must preserve the shared row geometry"
            );
        });
    }
    click(&mut app, "label-notes");
    app.update(|window, _| {
        assert_eq!(observed_input(window, "field-notes").focused(), Some(true))
    });
    const NOTES: &str = "PUBLIC first line\nPUBLIC second line";
    fill(&mut app, "field-notes", NOTES);
    click(&mut app, "save-entry");
    app.wait("notes saved", |window, _| {
        window.try_find("edit-entry").is_some()
    });
    drop(app);

    let mut app = start(directory.path(), "label-focus-notes-reopened");
    open(&mut app, &path);
    click_title(&mut app);
    app.wait("multiline notes retained after reopen", |window, _| {
        window
            .try_find("read-None-Notes")
            .is_some_and(|element| element.label() == Some(NOTES))
    });
    app.update(|window, cx| {
        window.click("read-None-Notes", cx);
        window.press(&taypeer_desktop_platform::primary_shortcut("a"), cx);
        window.press(&taypeer_desktop_platform::primary_shortcut("c"), cx);
        assert_eq!(
            cx.global::<taypeer_desktop_platform::Platform>()
                .fixture_clipboard(),
            NOTES,
            "ordinary read-only notes remain selectable and copyable"
        );
        window.input("PUBLIC rejected replacement", cx);
        assert_eq!(
            observed_input(window, "read-None-Notes").value(),
            Some(NOTES),
            "read-only values reject user edits"
        );
    });
}

pub(super) fn visual_workspace_checkpoints() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = start(directory.path(), "visual-workspace-checkpoints");
    if !app.captures_enabled() {
        return;
    }
    create(
        &mut app,
        &directory.path().join("PUBLIC-table-capture.taypeer"),
    );
    create_entry(&mut app);
    app.wait_idle();
    app.capture_checkpoint("table-selected")
        .expect("masked public table checkpoint");
    fill(&mut app, "search", "PUBLIC entry");
    app.capture_checkpoint("table-search")
        .expect("public search checkpoint");
    fill(&mut app, "search", "PUBLIC absent result");
    app.wait("empty table search settled", |window, _| {
        title_cell(window).is_none()
    });
    app.capture_checkpoint("table-empty")
        .expect("empty public table checkpoint");
}
