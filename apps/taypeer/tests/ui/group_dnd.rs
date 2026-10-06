// Drive the actual tree gestures; inspect stable IDs and geometry, never stores.
use gpui_kit::{ElementId, InputEvent, SharedString};

fn groups(window: &gpui_kit::Window) -> Vec<SharedString> {
    gpui_kit::base::test_support::snapshots(window)
        .iter()
        .filter_map(|element| match element.path().last()? {
            ElementId::Name(name) if name.starts_with("group-node-") => Some(name.clone()),
            _ => None,
        })
        .collect()
}

fn add_group(app: &mut Session, name: &str) -> SharedString {
    let before = app.update(|window, _| groups(window));
    click(app, "add-group");
    fill_nowait(app, "field-name", name);
    app.update(|window, cx| window.click("close-metadata", cx));
    app.wait("group durably created", |window, _| {
        window.try_find("close-metadata").is_none() && groups(window).len() == before.len() + 1
    });
    app.update(|window, _| {
        groups(window)
            .into_iter()
            .find(|id| !before.contains(id))
            .expect("new group identity")
    })
}

fn choose(app: &mut Session, id: &SharedString) {
    app.wait_idle();
    app.update(|window, cx| window.click(id.clone(), cx));
    app.pump();
    app.wait_idle();
    app.wait("clicked group selected", |window, _| selected(window, id));
}

fn drop_zone(row: &SharedString, zone: &str) -> SharedString {
    let group = row.rsplit('-').next().expect("group identity");
    format!("group-drop-{zone}-{group}").into()
}

fn drag(app: &mut Session, from: &SharedString, to: SharedString) {
    app.wait_idle();
    app.step("drag group");
    app.update(|window, cx| {
        assert!(window.find(from.clone()).visible());
        assert!(window.find(to.clone()).visible());
        window.drag_to(from.clone(), to, cx);
    });
    app.pump();
    app.wait_idle();
}

fn coordinates(
    window: &gpui_kit::Window,
    row: &SharedString,
) -> (gpui_kit::Pixels, gpui_kit::Pixels) {
    let expand: SharedString = row.replacen("group-node-", "expand-group-", 1).into();
    (
        window.find(expand).bounds().origin.x,
        window.find(row.clone()).bounds().origin.y,
    )
}

fn selected(window: &gpui_kit::Window, row: &SharedString) -> bool {
    let path = window.find(row.clone());
    gpui_kit::base::test_support::snapshots(window)
        .iter()
        .any(|element| path.path().starts_with(element.path()) && element.selected() == Some(true))
}

fn order(app: &mut Session, rows: &[&SharedString]) {
    app.wait("confirmed tree order", |window, _| {
        if rows
            .iter()
            .any(|row| window.try_find((*row).clone()).is_none())
        {
            return false;
        }
        rows.windows(2)
            .all(|pair| coordinates(window, pair[0]).1 < coordinates(window, pair[1]).1)
    });
}

fn child(app: &mut Session, parent: &SharedString, child: &SharedString, nested: bool) {
    app.wait("confirmed group nesting", |window, _| {
        let (parent_x, _) = coordinates(window, parent);
        let (child_x, _) = coordinates(window, child);
        if nested {
            child_x > parent_x
        } else {
            child_x == parent_x
        }
    });
}

pub(super) fn group_drag_changes_order_and_parent_durably() {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempfile::tempdir().unwrap();
    let mut app = start(directory.path(), "group-dnd");
    let path = create(&mut app, &directory.path().join("PUBLIC.taypeer"));
    let a = add_group(&mut app, "PUBLIC A");
    let b = add_group(&mut app, "PUBLIC B");
    let c = add_group(&mut app, "PUBLIC C");
    click(&mut app, "ungrouped");
    app.wait("ungrouped clears tree selection", |window, _| {
        [&a, &b, &c].iter().all(|row| !selected(window, row))
    });

    drag(&mut app, &c, drop_zone(&a, "before"));
    order(&mut app, &[&c, &a, &b]);
    app.update(|window, _| {
        assert!(
            [&a, &b, &c].iter().all(|row| !selected(window, row)),
            "reorder retains ungrouped selection"
        );
    });
    choose(&mut app, &b);
    drag(&mut app, &b, drop_zone(&c, "after"));
    order(&mut app, &[&c, &b, &a]);
    drag(&mut app, &a, drop_zone(&b, "into"));
    child(&mut app, &b, &a, true);

    // The blank space within the viewport is also a root drop target.
    app.update(|window, cx| {
        let from = window.find(a.clone()).bounds().center();
        let bounds = window.find("sidebar-tree").bounds();
        window.drag(
            from,
            gpui_kit::point(bounds.center().x, bounds.origin.y + gpui_kit::px(180.)),
            cx,
        );
    });
    app.pump();
    child(&mut app, &b, &a, false);
    drag(&mut app, &a, drop_zone(&b, "into"));
    child(&mut app, &b, &a, true);
    order(&mut app, &[&c, &b, &a]);
    // A drag does not select its source; the current selection remains B.
    app.update(|window, _| assert!(selected(window, &b), "drag retains selection"));

    let confirmed = std::fs::read(&path).unwrap();
    drag(&mut app, &b, drop_zone(&a, "into"));
    drag(&mut app, &a, drop_zone(&a, "into"));
    child(&mut app, &b, &a, true);
    assert_eq!(
        std::fs::read(&path).unwrap(),
        confirmed,
        "invalid drops do not write"
    );

    drag(&mut app, &a, "group-drop-root".into());
    child(&mut app, &b, &a, false);
    order(&mut app, &[&c, &b, &a]);
    drag(&mut app, &a, drop_zone(&b, "into"));
    child(&mut app, &b, &a, true);

    choose(&mut app, &a);
    click(&mut app, "new-entry");
    fill_nowait(&mut app, "field-name", "PUBLIC input before group move");
    // Do not wait for autosave: the gesture must flush the latest input itself.
    app.update(|window, cx| window.drag_to(c.clone(), drop_zone(&b, "after"), cx));
    app.pump();
    app.wait("move saves the open editor first", |window, _| {
        window.try_find("close-editor").is_none()
            && title_cell(window)
                .is_some_and(|cell| cell.label() == Some("PUBLIC input before group move"))
    });
    order(&mut app, &[&b, &a, &c]);
    app.update(|window, _| assert!(selected(window, &a), "move retains editor group"));

    // A rejected disk write must leave the confirmed tree and file intact.
    let confirmed = std::fs::read(&path).unwrap();
    let parent = path.parent().unwrap();
    let permissions = std::fs::metadata(parent).unwrap().permissions();
    std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o500)).unwrap();
    drag(&mut app, &c, drop_zone(&b, "into"));
    app.wait("failed move is reported", |window, _| {
        gpui_kit::base::test_support::snapshots(window)
            .iter()
            .any(|element| {
                element.label()
                    == Some("The file could not be read or saved. Check the path and access.")
            })
    });
    child(&mut app, &b, &c, false);
    assert_eq!(
        std::fs::read(&path).unwrap(),
        confirmed,
        "failed drop does not publish"
    );
    std::fs::set_permissions(parent, permissions).unwrap();
    drag(&mut app, &c, drop_zone(&b, "before"));
    order(&mut app, &[&c, &b, &a]);
    if app.captures_enabled() {
        app.capture_checkpoint("confirmed-tree")
            .expect("safe group tree PNG");
    }

    // Revocation ends an in-flight gesture and removes its name from the screen.
    let confirmed = std::fs::read(&path).unwrap();
    app.update(|window, cx| {
        let from = window.find(c.clone()).bounds().center();
        window.dispatch_event(
            gpui_kit::MouseMoveEvent {
                position: from,
                pressed_button: None,
                modifiers: Default::default(),
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
        window.dispatch_event(
            gpui_kit::MouseDownEvent {
                button: gpui_kit::MouseButton::Left,
                position: from,
                modifiers: Default::default(),
                click_count: 1,
                first_mouse: false,
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
        window.dispatch_event(
            gpui_kit::MouseMoveEvent {
                position: from + gpui_kit::point(gpui_kit::px(20.), gpui_kit::px(20.)),
                pressed_button: Some(gpui_kit::MouseButton::Left),
                modifiers: Default::default(),
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
        assert!(cx.has_active_drag(), "real gesture starts a drag");
    });
    app.pump();
    app.wait("drag preview visible", |window, _| {
        window
            .try_find("group-drag-preview")
            .is_some_and(|el| el.visible())
    });
    if app.captures_enabled() {
        app.capture_checkpoint("drag-preview")
            .expect("safe drag PNG");
    }
    app.system_lock();
    app.wait("lock clears active drag", |window, cx| {
        window.try_find("unlock").is_some()
            && window.try_find("group-drag-preview").is_none()
            && !cx.has_active_drag()
    });
    assert_eq!(
        std::fs::read(&path).unwrap(),
        confirmed,
        "lock aborts an unfinished gesture"
    );

    drop(app);
    let mut reopened = start(directory.path(), "group-dnd-reopened");
    open(&mut reopened, &path);
    reopened.wait("reopened root groups", |window, _| {
        [&b, &c].iter().all(|row| {
            window
                .try_find((*row).clone())
                .is_some_and(|row| row.visible())
        })
    });
    if reopened.update(|window, _| window.try_find(a.clone()).is_none()) {
        let expand: SharedString = b.replacen("group-node-", "expand-group-", 1).into();
        reopened.update(|window, cx| window.click(expand, cx));
        reopened.pump();
    }
    order(&mut reopened, &[&c, &b, &a]);
    child(&mut reopened, &b, &a, true);
    child(&mut reopened, &b, &c, false);
    choose(&mut reopened, &a);
    reopened.wait("pre-move input is durable", |window, _| {
        title_cell(window)
            .is_some_and(|cell| cell.label() == Some("PUBLIC input before group move"))
    });
}
