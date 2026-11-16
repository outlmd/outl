//! Keeps the browser mock's palette fixture in step with the presets.
//!
//! `@outl/shared/mock` paints the UI outside Tauri, where there is no
//! `get_theme` to ask. It reads every preset from a JSON file instead,
//! and this test is what stops that file from drifting: change a preset
//! and it fails until you regenerate the file with
//!
//! ```sh
//! OUTL_UPDATE_FIXTURES=1 cargo test -p outl-theme --test presets_fixture
//! ```

use std::path::PathBuf;

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../outl-frontend-shared/src/mock/presets.json")
}

fn rendered() -> String {
    let presets: Vec<_> = outl_theme::preset_names()
        .iter()
        .map(|name| outl_theme::by_name(name).expect("every listed preset resolves"))
        .collect();
    let mut json = serde_json::to_string_pretty(&presets).expect("palette serialises");
    json.push('\n');
    json
}

#[test]
fn the_mock_palette_fixture_matches_the_presets() {
    let path = fixture_path();
    let want = rendered();
    if std::env::var_os("OUTL_UPDATE_FIXTURES").is_some() {
        std::fs::write(&path, &want).expect("write fixture");
        return;
    }
    let got = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        got == want,
        "{} is stale; regenerate it with OUTL_UPDATE_FIXTURES=1 cargo test -p outl-theme --test presets_fixture",
        path.display()
    );
}
