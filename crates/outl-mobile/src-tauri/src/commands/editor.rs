//! `editor.diagrams` for the mobile settings sheet.
//!
//! The desktop writes the same field through its flat `Settings` DTO.
//! This pair only reads and writes that one key, so the sheet does
//! not round-trip the rest of `config.toml`.

use outl_config::DiagramsMode;

fn wire(mode: DiagramsMode) -> String {
    match mode {
        DiagramsMode::Off => "off".to_string(),
        DiagramsMode::Pele => "pele".to_string(),
    }
}

fn parse(value: &str) -> DiagramsMode {
    match value {
        "off" => DiagramsMode::Off,
        _ => DiagramsMode::Pele,
    }
}

#[tauri::command]
pub fn get_editor_diagrams() -> String {
    wire(outl_config::load().editor.diagrams)
}

#[tauri::command]
pub fn set_editor_diagrams(diagrams: String) -> Result<(), String> {
    let mut cfg = outl_config::load();
    cfg.editor.diagrams = parse(&diagrams);
    outl_config::save(&cfg).map_err(|err| err.to_string())
}
