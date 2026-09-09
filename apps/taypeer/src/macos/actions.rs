//! Window and collection-scoped keyboard bindings.

use gpui_kit::*;

actions!(
    taypeer,
    [
        SaveEntry,
        LockDatabase,
        CancelEditing,
        FocusSearch,
        GroupUp,
        GroupDown,
        GroupLeft,
        GroupRight,
        GroupEnter,
        EntryUp,
        EntryDown,
        EntryEnter
    ]
);

pub(super) fn bind(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-f", FocusSearch, Some("Taypeer")),
        KeyBinding::new("up", GroupUp, Some("TaypeerGroups")),
        KeyBinding::new("down", GroupDown, Some("TaypeerGroups")),
        KeyBinding::new("left", GroupLeft, Some("TaypeerGroups")),
        KeyBinding::new("right", GroupRight, Some("TaypeerGroups")),
        KeyBinding::new("enter", GroupEnter, Some("TaypeerGroups")),
        KeyBinding::new("up", EntryUp, Some("TaypeerEntries")),
        KeyBinding::new("down", EntryDown, Some("TaypeerEntries")),
        KeyBinding::new("enter", EntryEnter, Some("TaypeerEntries")),
        KeyBinding::new("cmd-s", SaveEntry, Some("Taypeer")),
        KeyBinding::new("cmd-l", LockDatabase, Some("Taypeer")),
        KeyBinding::new("escape", CancelEditing, Some("Taypeer")),
    ]);
}
