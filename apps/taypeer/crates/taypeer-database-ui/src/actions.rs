//! Window and collection-scoped keyboard bindings.

use gpui_kit::*;

actions!(
    taypeer,
    [
        /// Commit the active entry draft.
        SaveEntry,
        /// Revoke access to open database sessions.
        LockDatabase,
        /// Leave editing through the dirty-state guard.
        CancelEditing,
        /// Focus the active database search input.
        FocusSearch,
        /// Activate the keyboard-selected table entry.
        EntryEnter
    ]
);

/// Bind database commands to their owning keyboard contexts.
pub fn bind(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new(
            &taypeer_desktop_platform::primary_shortcut("f"),
            FocusSearch,
            Some("Taypeer"),
        ),
        KeyBinding::new("enter", EntryEnter, Some("DataTable")),
        KeyBinding::new(
            &taypeer_desktop_platform::primary_shortcut("s"),
            SaveEntry,
            Some("Taypeer"),
        ),
        KeyBinding::new(
            &taypeer_desktop_platform::primary_shortcut("l"),
            LockDatabase,
            Some("Taypeer"),
        ),
        KeyBinding::new("escape", CancelEditing, Some("Taypeer")),
    ]);
}
