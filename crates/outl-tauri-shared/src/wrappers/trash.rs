//! Trash commands (issue #287). Its own macro because `catalog.rs` sits
//! at its size ceiling — the same reason `wrappers/capture.rs` exists,
//! and `tests/command_parity.rs` scans every `src/wrappers/*.rs` rather
//! than `catalog.rs` alone, so a client dropping this module fails the
//! build instead of quietly shipping without a recycle bin.
//!
//! **Both** GUI clients register it. The desktop has a panel; mobile has
//! no surface yet, and that difference is recorded as `Capability::Trash`
//! rather than left to a user hunting for it — registering a command is
//! not shipping a feature (see [`super`]).

/// `list_trash` / `restore_trash_entry`: read a deletion back.
#[macro_export]
macro_rules! trash_commands {
    ($state:ty) => {
        $crate::tauri_commands! {
            state = $state;

            /// Every top-level deletion in the trash, each carrying the
            /// refusal a restore would return.
            async fn list_trash()
                -> ::std::result::Result<
                    ::std::vec::Vec<$crate::commands::trash::TrashEntryDto>,
                    String,
                >
                => $crate::commands::trash::list_trash;

            /// Put one deleted block back where it was deleted from.
            fn restore_trash_entry(id: String)
                -> ::std::result::Result<
                    $crate::commands::trash::RestoreTrashReply,
                    String,
                >
                => $crate::commands::trash::restore_trash_entry;
        }
    };
}
