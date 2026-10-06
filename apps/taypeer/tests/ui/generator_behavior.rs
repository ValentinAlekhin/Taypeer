// Real generator events against an isolated synthetic database; no image capture or secret diagnostics.
use super::*;

fn dialog_width(window: &gpui_kit::Window) -> f32 {
    let panel = window.find("generator-panel");
    let layer = gpui_kit::ElementId::from(0usize);
    let popup = gpui_kit::base::test_support::snapshots(window)
        .into_iter()
        .filter(|element| {
            panel.path().starts_with(element.path()) && element.path().last() == Some(&layer)
        })
        .min_by_key(|element| element.path().len())
        .expect("observed popup of the first real dialog layer");
    f32::from(popup.bounds().size.width)
}

fn select_mode(app: &mut Session, index: usize, width: f32) {
    wait_for_stable_control(app, "generator-panel");
    app.update(|window, cx| window.within("generator-tabs").click(index, cx));
    app.pump();
    app.wait("generator mode and dialog width settled", |window, _| {
        window.within("generator-tabs").find(index).selected() == Some(true)
            && (dialog_width(window) - width).abs() < 1.
    });
}

pub(super) fn generator_modes_insert_and_persist_the_generated_value() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = start(directory.path(), "generator-modes");
    let path = create(&mut app, &directory.path().join("PUBLIC-generator.taypeer"));
    create_entry(&mut app);
    click(&mut app, "edit-entry");
    click(&mut app, "generator");
    app.wait("password generator installed", |window, _| {
        window.try_find("generator-length").is_some() && (dialog_width(window) - 560.).abs() < 1.
    });
    fill(&mut app, "generator-length", "18");
    app.update(|window, _| {
        let slider = window.find("generator-length-slider");
        assert!(
            gpui_kit::base::test_support::snapshots(window)
                .iter()
                .any(|element| {
                    element.path().starts_with(slider.path())
                        && element.role() == Some(gpui_kit::Role::Slider)
                }),
            "password mode retains the native slider"
        );
        assert!(window.find("generator-use").disabled() != Some(true));
    });

    select_mode(&mut app, 1, 744.);
    app.wait("phrase controls installed", |window, _| {
        window.try_find("generator-words").is_some()
            && window.try_find("field-ui.separator").is_some()
            && window.try_find("generator-length").is_none()
    });
    fill(&mut app, "generator-words", "5");
    fill(&mut app, "field-ui.separator", "/");
    select_mode(&mut app, 0, 560.);
    app.update(|window, _| {
        assert_eq!(
            observed_input(window, "generator-length").value(),
            Some("18")
        );
    });
    select_mode(&mut app, 1, 744.);
    app.update(|window, _| {
        assert_eq!(observed_input(window, "generator-words").value(), Some("5"));
        assert_eq!(window.find("field-ui.separator").value(), Some("/"));
    });
    click(&mut app, "generator-use");
    app.wait("generated value applied to editor", |window, _| {
        window.try_find("generator-panel").is_none() && window.try_find("close-editor").is_some()
    });
    click(&mut app, "editor-password-eye");
    app.wait(
        "generated phrase revealed by explicit eye action",
        |window, _| {
            window.find("field-password").value().is_some_and(|value| {
                value.split('/').count() == 5
                    && value.split('/').all(|word| {
                        !word.is_empty() && word.bytes().all(|byte| byte.is_ascii_lowercase())
                    })
            })
        },
    );
    let generated =
        app.update(|window, _| window.find("field-password").value().unwrap().to_owned());
    click(&mut app, "editor-password-eye");
    click(&mut app, "close-editor");
    app.wait("generated entry saved", |window, _| {
        window.try_find("edit-entry").is_some()
    });
    drop(app);

    let mut reopened = start(directory.path(), "generator-modes-reopened");
    open(&mut reopened, &path);
    click_title(&mut reopened);
    click(&mut reopened, "edit-entry");
    click(&mut reopened, "editor-password-eye");
    reopened.wait("generated password durable after reopen", |window, _| {
        window.find("field-password").value() == Some(generated.as_str())
    });
}
