//! Page history read out of the op log.
//!
//! New on mobile: the command existed only on the desktop, so a phone
//! could not answer "what changed on this page" at all. Registering it
//! costs a symbol; the frontend can adopt it without a backend change.
outl_tauri_shared::timeline_commands!(crate::state::AppState);
