// Real settings events, with an isolated profile owned by the scenario.
use super::*;

pub(super) fn inline_settings_apply_and_keep_invalid_input() {
    let directory = tempfile::tempdir().unwrap();
    let profile = directory.path().join("profile");
    let mut app = start(directory.path(), "inline-settings");
    app.update(|window, cx| window.press(&taypeer_desktop_platform::primary_shortcut(","), cx));
    app.wait("inline device name ready", |window, _| {
        window
            .try_find("device-name")
            .is_some_and(|field| field.visible() && field.disabled() != Some(true))
    });
    let original = taypeer_settings_ui::local_settings::LocalSettings::load(&profile)
        .unwrap()
        .device_name;
    fill(&mut app, "device-name", "");
    app.update(|window, cx| window.press("enter", cx));
    app.wait("invalid device name stays local", |window, _| {
        window.try_find("device-name-error").is_some()
    });
    click(&mut app, "autolock");
    app.update(|window, cx| window.press("escape", cx));
    app.pump();
    app.update(|window, _| {
        assert_eq!(window.find("device-name").value(), Some(""));
        assert!(window.try_find("device-name-error").is_some());
        assert!(window.try_find("dialog-confirm").is_none());
    });
    assert_eq!(
        taypeer_settings_ui::local_settings::LocalSettings::load(&profile)
            .unwrap()
            .device_name,
        original
    );
    fill(&mut app, "device-name", "PUBLIC Enter device");
    app.update(|window, cx| window.press("enter", cx));
    app.wait("Enter persisted device name", |window, _| {
        window.try_find("device-name-error").is_none()
            && taypeer_settings_ui::local_settings::LocalSettings::load(&profile)
                .is_ok_and(|settings| settings.device_name == "PUBLIC Enter device")
    });
    app.wait("device name accepts next edit", |window, _| {
        window.find("device-name").disabled() != Some(true)
    });
    fill(&mut app, "device-name", "PUBLIC Blur device");
    click(&mut app, "autolock");
    app.wait("device name lost focus", |window, _| {
        window.find("device-name").focused() == Some(false)
    });
    app.wait("blur persisted device name", |_, _| {
        taypeer_settings_ui::local_settings::LocalSettings::load(&profile)
            .is_ok_and(|settings| settings.device_name == "PUBLIC Blur device")
    });
    app.update(|window, cx| window.press("escape", cx));
    drop(app);
    let mut reopened = start(directory.path(), "inline-settings-reopened");
    reopened
        .update(|window, cx| window.press(&taypeer_desktop_platform::primary_shortcut(","), cx));
    reopened.wait("inline name survives reopening", |window, _| {
        window
            .try_find("device-name")
            .is_some_and(|field| field.value() == Some("PUBLIC Blur device"))
    });
}
