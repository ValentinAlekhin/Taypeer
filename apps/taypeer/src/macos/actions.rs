//! Window and collection-scoped keyboard bindings.

use gpui_kit::*;

actions!(
    taypeer,
    [
        SaveEntry,
        LockDatabase,
        CancelEditing,
        FocusSearch,
        EntryEnter
    ]
);

pub(super) fn bind(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-f", FocusSearch, Some("Taypeer")),
        KeyBinding::new("enter", EntryEnter, Some("DataTable")),
        KeyBinding::new("cmd-s", SaveEntry, Some("Taypeer")),
        KeyBinding::new("cmd-l", LockDatabase, Some("Taypeer")),
        KeyBinding::new("escape", CancelEditing, Some("Taypeer")),
    ]);
}
