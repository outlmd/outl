//! Plugin keybinding dispatch for Normal mode.
//!
//! A plugin declares chords in its manifest (`contributes.keybindings`);
//! `outl-plugins` parses them into [`outl_shortcuts::ChordSequence`] and
//! the TUI compares a live keystroke against them here. The matching
//! keystroke runs the plugin's command through the existing
//! [`crate::state::App::run_plugin_command`].
//!
//! ## Where this sits in the input pipeline
//!
//! [`try_plugin_binding`] is called **first** inside
//! [`super::normal::handle_normal_key`], before any native key handling,
//! but only from Normal mode and only when no help popup / sidebar focus
//! / pending native chord is active. It never fires while the user is
//! typing (Insert) or in an overlay — plugin chords are `Mode::Global`
//! in the catalog but the TUI deliberately scopes them to Normal so a
//! plugin can't steal keys mid-edit.
//!
//! ## Why a separate chord buffer
//!
//! Two-chord plugin sequences (`Ctrl+T A`) need a one-keystroke buffer.
//! We keep that in `App::pending_plugin_chord`, distinct from the native
//! `pending_chord` vim accumulator, so the two never interfere: a plugin
//! sequence can't be swallowed by `d`/`g`/`y`/`z`, and arming a plugin
//! prefix can't break a native chord.
//!
//! ## Never shadow a native action
//!
//! [`native_normal_chord`] mirrors the chords `handle_normal_key`
//! consumes. A plugin binding only fires when its chord maps to nothing
//! native, so a plugin can't rebind `j`, `dd`, `Ctrl+P`, etc. out from
//! under the user.

use super::chord_adapter::chord_from_key;
use crate::state::App;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Try to dispatch a plugin keybinding for this Normal-mode keystroke.
///
/// Returns `true` when the key was consumed by the plugin layer (a
/// command fired, or the first chord of a two-chord plugin sequence was
/// buffered). On `true` the caller must stop and **not** fall through to
/// native key handling. Returns `false` to let native handling proceed.
pub(super) fn try_plugin_binding(app: &mut App, key: KeyEvent) -> bool {
    // Cheapest possible early-out: no host loaded.
    if app.plugin_host.is_none() {
        // A buffered first chord can't be completed without a host;
        // drop it so it doesn't linger.
        app.pending_plugin_chord = None;
        return false;
    }

    let Some(this) = chord_from_key(key) else {
        // Unmappable key (bare modifier, etc.) — abandon any pending
        // plugin sequence and let native handling have it.
        app.pending_plugin_chord = None;
        return false;
    };

    let bindings = {
        let host = app.plugin_host.as_ref().expect("host present");
        host.keybindings("tui")
    };
    if bindings.is_empty() {
        app.pending_plugin_chord = None;
        return false;
    }

    // Completing a two-chord plugin sequence: a previous keystroke
    // armed `pending_plugin_chord`.
    if let Some(first) = app.pending_plugin_chord.take() {
        for b in &bindings {
            if b.chord.0.len() == 2 && b.chord.0[0] == first && b.chord.0[1] == this {
                let (plugin_id, command_id) = (b.plugin_id.clone(), b.command_id.clone());
                app.run_plugin_command(&plugin_id, &command_id);
                return true;
            }
        }
        // The second key didn't complete any sequence. Fall through so
        // the key still does its native thing (e.g. the user typed
        // `Ctrl+T` then `j` — `j` should still move down).
    }

    // A native binding wins the chord outright — never let a plugin
    // shadow it.
    if native_normal_chord(key) {
        return false;
    }

    // Single-chord plugin binding matches this key exactly → fire.
    for b in &bindings {
        if b.chord.0.len() == 1 && b.chord.0[0] == this {
            let (plugin_id, command_id) = (b.plugin_id.clone(), b.command_id.clone());
            app.run_plugin_command(&plugin_id, &command_id);
            return true;
        }
    }

    // This key is the first chord of some two-chord plugin binding →
    // arm and wait for the second keypress.
    if bindings
        .iter()
        .any(|b| b.chord.0.len() == 2 && b.chord.0[0] == this)
    {
        app.pending_plugin_chord = Some(this);
        app.status = "plugin chord…".into();
        return true;
    }

    false
}

/// Does this key resolve to a built-in Normal-mode action?
///
/// Mirror of the chords `handle_normal_key`'s `match` consumes, used to
/// keep a plugin keybinding from shadowing a native one. We compare on
/// the same `(KeyCode, modifier-predicate)` shape the match arms use
/// rather than rebuilding the whole table in `outl-shortcuts`, because
/// the TUI's Normal handler isn't catalog-driven yet (it pattern-matches
/// `KeyEvent` directly).
///
/// Conservative by design: when unsure, treat the key as native so the
/// plugin layer yields. A plugin author who wants a guaranteed-free
/// chord should reach for a modified or two-chord binding (`Ctrl+T A`),
/// which the bare-vim Normal map leaves open.
fn native_normal_chord(key: KeyEvent) -> bool {
    use KeyCode::*;
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);

    // The specific modified combos `handle_normal_key` special-cases.
    // Everything else carrying Ctrl/Alt is *free* for plugins — we list
    // these exhaustively rather than blanket-claiming every Ctrl/Alt
    // chord, so a plugin can bind e.g. `Ctrl+G` without being shadowed.
    if ctrl {
        return matches!(
            key.code,
            // Ctrl+T (TODO), Ctrl+D / Ctrl+U (half-page), Ctrl+R (redo),
            // Ctrl+P (picker), Ctrl+B (backlinks), Ctrl+E (sidebar).
            Char('t' | 'd' | 'u' | 'r' | 'p' | 'b' | 'e' | 'T' | 'D' | 'U' | 'R' | 'P' | 'B' | 'E')
            // Ctrl+Enter (TODO toggle).
            | Enter
        );
        // Ctrl+S / Ctrl+L / Ctrl+C are intercepted upstream in
        // `runtime.rs` before Normal handling, so they never reach here;
        // treating them as free below is harmless.
    }
    if alt {
        // Alt+Up / Alt+Down drag the current block.
        return matches!(key.code, Up | Down);
    }

    // Bare (unmodified) keys the Normal handler owns: vim navigation,
    // chord-arming letters, and the named keys with bindings.
    matches!(
        key.code,
        Char(
            'q' | 'Z'
                | '?'
                | 't'
                | '['
                | ']'
                | 'g'
                | 'z'
                | 'd'
                | 'y'
                | 'c'
                | 'p'
                | 'P'
                | 'i'
                | 'I'
                | 'a'
                | 'A'
                | 'o'
                | 'O'
                | 'x'
                | 'X'
                | 'D'
                | 'C'
                | 'S'
                | 's'
                | 'r'
                | 'f'
                | 'F'
                | '~'
                | 'Y'
                | 'e'
                | '*'
                | '#'
                | 'j'
                | 'k'
                | 'G'
                | 'h'
                | 'l'
                | '0'
                | '$'
                | 'b'
                | 'B'
                | 'w'
                | 'u'
                | 'n'
                | 'N'
                | 'V'
                | '/'
                | ':'
                | 'E'
        ) | Enter
            | Tab
            | BackTab
            | Up
            | Down
            | Left
            | Right
            | PageUp
            | PageDown
            | Home
            | End
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::normal::handle_normal_key;
    use crate::state::App;
    use outl_core::id::ActorId;
    use outl_core::workspace::Workspace;
    use std::path::Path;
    use tempfile::TempDir;

    /// A dev plugin (no lockfile, permissions implicitly granted) that
    /// contributes a command bound to a TUI keybinding. The command
    /// creates a page so we can assert the chord actually fired by
    /// checking the op log grew.
    const BUNDLE: &str = r#"
        globalThis.__outl_register({
            activate(ctx) {
                ctx.commands.register('mark', () => {
                    // `page.create` emits an `ensure-page` intent the host
                    // applies through outl-actions, so the op log grows —
                    // a side effect the test can observe to prove the
                    // chord actually dispatched the command.
                    ctx.page.create('plugin-marked');
                });
            }
        });
    "#;

    fn write_dev_plugin(root: &Path, key: &str) {
        let dir = root.join(".outl/plugins/_dev/kb");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("plugin.json"),
            format!(
                r#"{{
                    "id": "run.avelino.kb",
                    "name": "KB",
                    "version": "1.0.0",
                    "api": "^1.0",
                    "main": "index.js",
                    "capabilities": ["slash-command", "keybinding"],
                    "permissions": ["read-page", "write-page", "submit-op", "read-op-log"],
                    "contributes": {{
                        "commands": [{{ "id": "mark", "title": "Mark" }}],
                        "keybindings": [{{ "command": "mark", "key": "{key}", "when": "tui" }}]
                    }}
                }}"#
            ),
        )
        .unwrap();
        std::fs::write(dir.join("index.js"), BUNDLE).unwrap();
    }

    fn app_with(root: &TempDir) -> App {
        let actor = ActorId::new();
        let ws = Workspace::open_in_memory(actor).unwrap();
        App::new(
            root.path().to_path_buf(),
            ws,
            actor,
            crate::theme::default_theme(),
            false,
        )
        .unwrap()
    }

    fn log_len(app: &App) -> usize {
        app.workspace.log().len()
    }

    fn press(app: &mut App, code: KeyCode, mods: KeyModifiers) {
        handle_normal_key(app, KeyEvent::new(code, mods)).unwrap();
    }

    #[test]
    fn single_chord_plugin_binding_fires() {
        let dir = TempDir::new().unwrap();
        write_dev_plugin(dir.path(), "Ctrl+G");
        let mut app = app_with(&dir);
        app.load_plugins();

        let before = log_len(&app);
        // Ctrl+G is not a native Normal action, so the plugin binding fires.
        press(&mut app, KeyCode::Char('g'), KeyModifiers::CONTROL);
        assert!(
            log_len(&app) > before,
            "plugin command should have mutated the workspace"
        );
    }

    #[test]
    fn two_chord_plugin_binding_fires_on_completion() {
        let dir = TempDir::new().unwrap();
        // `Ctrl+G` is not a native Normal chord, so its first press is
        // free to arm a plugin sequence. (`Ctrl+T`, by contrast, is the
        // native TODO toggle and would correctly *not* arm — that's the
        // never-shadow guard, exercised separately below.)
        write_dev_plugin(dir.path(), "Ctrl+G A");
        let mut app = app_with(&dir);
        app.load_plugins();

        let before = log_len(&app);
        // First chord arms; nothing fires yet.
        press(&mut app, KeyCode::Char('g'), KeyModifiers::CONTROL);
        assert!(
            app.pending_plugin_chord.is_some(),
            "first chord should arm the plugin sequence"
        );
        assert_eq!(
            log_len(&app),
            before,
            "nothing should fire on the first chord"
        );

        // Second chord completes the sequence and fires.
        press(&mut app, KeyCode::Char('a'), KeyModifiers::NONE);
        assert!(app.pending_plugin_chord.is_none(), "sequence consumed");
        assert!(log_len(&app) > before, "completed chord should mutate");
    }

    #[test]
    fn native_chord_is_not_stolen_as_a_plugin_prefix() {
        let dir = TempDir::new().unwrap();
        // `Ctrl+T` is the native TODO toggle. Even though the plugin
        // wants it as the prefix of a two-chord sequence, the guard
        // must let the native action win and never arm the plugin.
        write_dev_plugin(dir.path(), "Ctrl+T A");
        let mut app = app_with(&dir);
        app.load_plugins();

        press(&mut app, KeyCode::Char('t'), KeyModifiers::CONTROL);
        assert!(
            app.pending_plugin_chord.is_none(),
            "native Ctrl+T must not arm a plugin sequence"
        );
    }

    #[test]
    fn plugin_never_shadows_a_native_chord() {
        let dir = TempDir::new().unwrap();
        // Bind the plugin to bare `j` — a native "move down". The guard
        // must refuse to let the plugin steal it.
        write_dev_plugin(dir.path(), "j");
        let mut app = app_with(&dir);
        app.load_plugins();

        let before = log_len(&app);
        press(&mut app, KeyCode::Char('j'), KeyModifiers::NONE);
        // `j` moved the selection (a pure UI op, no op-log mutation), and
        // crucially did NOT run the plugin's `mark` command.
        assert_eq!(
            log_len(&app),
            before,
            "native `j` must not trigger the plugin command"
        );
    }

    #[test]
    fn abandoned_two_chord_prefix_falls_through_to_native() {
        let dir = TempDir::new().unwrap();
        write_dev_plugin(dir.path(), "Ctrl+G A");
        let mut app = app_with(&dir);
        app.load_plugins();

        // Arm with Ctrl+G, then press a key that doesn't complete the
        // sequence: the plugin chord is dropped and the key still does
        // its native thing.
        press(&mut app, KeyCode::Char('g'), KeyModifiers::CONTROL);
        assert!(app.pending_plugin_chord.is_some());

        let before = log_len(&app);
        // `k` doesn't complete `Ctrl+G A`; it should move selection up,
        // not run the plugin.
        press(&mut app, KeyCode::Char('k'), KeyModifiers::NONE);
        assert!(app.pending_plugin_chord.is_none(), "stale prefix cleared");
        assert_eq!(log_len(&app), before, "plugin must not have fired");
    }

    #[test]
    fn no_plugins_leaves_native_handling_untouched() {
        let dir = TempDir::new().unwrap();
        let mut app = app_with(&dir);
        // No plugins installed: dispatch is a no-op, `j` still navigates.
        let before = log_len(&app);
        press(&mut app, KeyCode::Char('j'), KeyModifiers::NONE);
        assert_eq!(log_len(&app), before);
    }
}
