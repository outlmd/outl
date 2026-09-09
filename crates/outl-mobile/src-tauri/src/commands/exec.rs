//! Code-block execution commands, generated from the shared catalog.
//!
//! Mobile used to register only `run_code_block`, so `auto-run` blocks
//! and `{{embed}}` resolution simply did not exist on the phone — not by
//! decision, but because the other two wrappers were never typed. Taking
//! the whole module is what stops that shape of gap recurring.
outl_tauri_shared::exec_commands!(crate::state::AppState);
