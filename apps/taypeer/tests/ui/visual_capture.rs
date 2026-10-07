// Real AppView capture guards remain active even for public fixtures.
pub(super) fn capture_guards_and_supported_viewports() {
    for (width, height) in [(1100., 720.), (1320., 820.)] {
        let directory = tempfile::tempdir().unwrap();
        let mut app = Session::with_viewport(directory.path(), Path::new(env!("CARGO_BIN_EXE_taypeer-ui-worker")), "capture-viewports", gpui_kit::size(gpui_kit::px(width), gpui_kit::px(height)));
        app.wait("welcome installed", |window, _| window.try_find("welcome-create").is_some());
        if app.captures_enabled() { assert!(app.capture_checkpoint("welcome").expect("safe welcome PNG").exists()); }
        click(&mut app, "welcome-create");
        assert_eq!(app.capture_checkpoint("dialog-forbidden").unwrap_err(), "PNG forbidden for sensitive UI or closed window");
        click(&mut app, "dialog-cancel");
        app.update(|window, cx| window.press(&taypeer_desktop_platform::primary_shortcut(","), cx));
        app.wait("settings installed", |window, _| window.try_find("device-name").is_some());
        if app.captures_enabled() { assert!(app.capture_checkpoint("settings").expect("safe settings PNG").exists()); }
    }
    let directory = tempfile::tempdir().unwrap();
    let mut app = start(directory.path(), "capture-guards");
    let path = create(&mut app, &directory.path().join("PUBLIC-capture.taypeer"));
    create_entry(&mut app);
    click(&mut app, "edit-entry");
    app.wait("editor installed", |window, _| window.try_find("field-name").is_some());
    assert_eq!(app.capture_checkpoint("editor-forbidden").unwrap_err(), "PNG forbidden for sensitive UI or closed window");
    click(&mut app, "save-entry");
    app.wait("unchanged entry confirmed", |window, _| window.try_find("edit-entry").is_some());
    click_title(&mut app);
    app.update(|window, cx| window.hover("value-None-Password", cx));
    app.pump();
    click(&mut app, "show-None-Password");
    app.wait("password revealed", |window, _| window.try_find("read-None-Password").is_some());
    assert_eq!(app.capture_checkpoint("reveal-forbidden").unwrap_err(), "PNG forbidden for sensitive UI or closed window");
    app.system_lock();
    app.wait("locked installed", |window, _| window.try_find("unlock-password").is_some());
    if app.captures_enabled() { app.capture_checkpoint("unlock-empty").expect("empty unlock PNG"); }
    fill(&mut app, "unlock-password", "PUBLIC nonempty input");
    assert_eq!(app.capture_checkpoint("unlock-filled-forbidden").unwrap_err(), "PNG forbidden for sensitive UI or closed window");
    assert!(path.exists());
}
