//! The property-key catalogue that feeds the Properties sheet's chips.
//!
//! The mobile sheet asks "which key?" with tappable chips instead of a
//! text field (typing `oura-date` on a phone keyboard is the interaction
//! this feature exists to avoid); the chips are this list. The ranking
//! lives in `outl_actions::known_keys`.
outl_tauri_shared::property_commands!(crate::state::AppState);
