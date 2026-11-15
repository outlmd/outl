//! The text-inserting slash commands (`/date-today`, `/time-now`,
//! `/week-num`). Its own macro because `catalog.rs` sits at its size
//! ceiling — the same reason `wrappers/trash.rs` and
//! `wrappers/capture.rs` exist, and `tests/command_parity.rs` scans
//! every `src/wrappers/*.rs` rather than `catalog.rs` alone, so a
//! client dropping this module fails the build.
//!
//! **Both** GUI clients register it. Mobile has no `/` menu yet, and
//! that difference is a surface, not a command — registering one is
//! not shipping a feature (see [`super`]).

/// `list_insert_commands` / `resolve_insert_command`: the catalog, and
/// the text one entry produces. Pure — no workspace, no host.
///
/// Every entry is `plain` (the catalog plus a clock reading), so
/// `$state` is never expanded — kept for call-site uniformity.
#[macro_export]
macro_rules! insert_commands {
    ($state:ty) => {
        $crate::tauri_commands! {
            state = $state;

            /// Every text-inserting slash command (`/date-today`,
            /// `/time-now`, `/week-num`), for a command menu.
            fn list_insert_commands()
                -> ::std::vec::Vec<$crate::commands::insert::InsertCommandDto>
                => plain $crate::commands::insert::list_insert_commands;

            /// The text one of them inserts right now, resolved in the
            /// configured timezone.
            fn resolve_insert_command(name: String, args: Option<String>)
                -> ::std::result::Result<String, String>
                => plain $crate::commands::insert::resolve_insert_command;
        }
    };
}
