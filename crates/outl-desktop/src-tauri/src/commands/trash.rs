//! Reading a deletion back (issue #287).
//!
//! Named `trash`, not `history` or `timeline`: those two read the past,
//! this one undoes a piece of it. `Cmd+Z` reaches this session's
//! mutations; the trash reaches a deletion from any device, on any day,
//! because it reads the op log rather than a stack.
outl_tauri_shared::trash_commands!(crate::state::AppState);
