//! One declaration of the Tauri command surface, expanded per client.
//!
//! Every `#[tauri::command]` in `outl-desktop` and `outl-mobile` used to
//! be hand-written, and each one was the same four lines: take the
//! arguments Tauri deserialized, take `State<'_, AppState>`, call the
//! shared body in [`crate::commands`] with `state.inner()` in front,
//! return what it returned. 3,033 lines across the two clients to
//! register 72 functions that already existed here.
//!
//! The boilerplate was not the real cost. **The divergence was, and it
//! arrived by omission.** `commands/asset.rs` and `commands/theme.rs`
//! were byte-identical between the two clients; `commands/history.rs`
//! was 183 lines on the desktop and 22 on mobile, not because anyone
//! decided mobile should have less, but because nobody typed the rest.
//! Root `CLAUDE.md` invariant 12 exists to make a capability gap a
//! declared fact — and `outl_shortcuts::capability_support`'s exhaustive
//! `match` cannot see a command that was never registered.
//!
//! So the list lives here, once, and a client takes a whole module or
//! writes its own. Registering a command whose frontend does not call it
//! yet costs a symbol; *not* registering it costs a feature that silently
//! does not exist on one client. `outl-mobile`'s `commands/block.rs`
//! already said as much in prose ("the full desktop surface is
//! registered […] the frontend can adopt them without a backend
//! change") — this makes that the mechanism rather than the intention.
//!
//! # What stays hand-written
//!
//! A command that needs more from Tauri than `State<'_, AppState>` — an
//! `AppHandle` to emit an event, a second `State` for the plugin thread
//! — is not boilerplate and is not generated. Those live in the client,
//! where the extra dependency is visible.
//!
//! # Using it
//!
//! ```ignore
//! // crates/outl-desktop/src-tauri/src/commands/asset.rs
//! outl_tauri_shared::asset_commands!(crate::state::AppState);
//! ```
//!
//! # Adding a command
//!
//! Write the body in [`crate::commands`], add one line to the matching
//! `*_commands!` list below, and both clients have it. If the body needs
//! a borrowed argument, take `String` anyway: Tauri hands the wrapper an
//! owned value, and a `&str` parameter only moves an `&` into every call
//! site.

/// Generate `#[tauri::command]` wrappers over shared command bodies.
///
/// Grammar, one entry per line:
///
/// ```ignore
/// tauri_commands! {
///     state = crate::state::AppState;
///
///     /// Doc comments are forwarded.
///     fn edit_block(page_id: String, text: String) -> Result<PageView, String>
///         => outl_tauri_shared::commands::block::edit_block;
///
///     // `plain` = the body takes no host (config / pure functions).
///     fn get_theme(name: Option<String>) -> Palette
///         => plain outl_tauri_shared::commands::theme::get_theme;
///
///     // `async` is forwarded to both the wrapper and the call.
///     async fn page_backlinks(slug: String) -> Result<PageBacklinks, String>
///         => outl_tauri_shared::commands::page::page_backlinks;
/// }
/// ```
///
/// Arguments are passed through positionally, in declaration order,
/// after `state.inner()` (omitted for `plain`). Anything that needs a
/// different call shape is not boilerplate — write it by hand.
#[macro_export]
macro_rules! tauri_commands {
    (state = $state:ty; $($rest:tt)*) => {
        $crate::__tauri_command_entry!(@state $state; $($rest)*);
    };
}

/// TT-muncher behind [`tauri_commands`]. Not public API.
///
/// The `plain` marker is matched **before** the `$path:ty` fragment in
/// every arm, so a mis-typed entry falls through to the next rule
/// cleanly instead of failing inside a partially-consumed fragment.
///
/// There is no `async` + `plain` arm: every async command in the catalog
/// needs the host. Add one when a host-free async body turns up.
#[macro_export]
#[doc(hidden)]
macro_rules! __tauri_command_entry {
    (@state $state:ty;) => {};

    // async, host first
    (@state $state:ty;
        $(#[$meta:meta])*
        async fn $name:ident ( $($arg:ident : $argty:ty),* $(,)? ) -> $ret:ty
            => $($path:tt)::+ ;
        $($rest:tt)*
    ) => {
        $(#[$meta])*
        #[::tauri::command]
        pub(crate) async fn $name(
            $($arg: $argty,)*
            state: ::tauri::State<'_, $state>,
        ) -> $ret {
            $($path)::+(state.inner(), $($arg),*).await
        }
        $crate::__tauri_command_entry!(@state $state; $($rest)*);
    };

    // sync, no host
    (@state $state:ty;
        $(#[$meta:meta])*
        fn $name:ident ( $($arg:ident : $argty:ty),* $(,)? ) -> $ret:ty
            => plain $($path:tt)::+ ;
        $($rest:tt)*
    ) => {
        $(#[$meta])*
        #[::tauri::command]
        pub(crate) fn $name($($arg: $argty),*) -> $ret {
            $($path)::+($($arg),*)
        }
        $crate::__tauri_command_entry!(@state $state; $($rest)*);
    };

    // sync, host first
    (@state $state:ty;
        $(#[$meta:meta])*
        fn $name:ident ( $($arg:ident : $argty:ty),* $(,)? ) -> $ret:ty
            => $($path:tt)::+ ;
        $($rest:tt)*
    ) => {
        $(#[$meta])*
        #[::tauri::command]
        pub(crate) fn $name(
            $($arg: $argty,)*
            state: ::tauri::State<'_, $state>,
        ) -> $ret {
            $($path)::+(state.inner(), $($arg),*)
        }
        $crate::__tauri_command_entry!(@state $state; $($rest)*);
    };
}

pub mod catalog;
