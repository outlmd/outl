//! What a loaded plugin contributes to a client's chrome.
//!
//! Every function here reads `manifest.contributes` and projects it into the
//! entries a client merges into its palette, chord dispatcher, toolbar or fence
//! renderer. Each projection is filtered by the [`Capability`] the client
//! actually granted, so a plugin can never surface an affordance the client
//! cannot honour — the capability check is the reason these four live together
//! rather than next to the code that runs them.
//!
//! Nothing here touches the engine or the workspace. *Listing* what a plugin
//! offers is a different job from running it: `run_command` and
//! [`PluginHost::transform_block`] stay with the host's turn machinery.

use outl_shortcuts::{ChordSequence, Mode};

use super::PluginHost;
use crate::capability::Capability;

/// A command a plugin contributes, surfaced to the client's palette.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandEntry {
    /// Owning plugin id.
    pub plugin_id: String,
    /// Command id (`contributes.commands[].id`).
    pub command_id: String,
    /// Human title.
    pub title: String,
}

/// A plugin keybinding, parsed and ready for a client to merge into its chord
/// dispatcher. The client runs `run_command(plugin_id, command_id)` when the
/// chord fires.
#[derive(Debug, Clone)]
pub struct PluginBinding {
    /// The parsed chord sequence.
    pub chord: ChordSequence,
    /// Mode the binding fires in (plugin chords are `Global`).
    pub mode: Mode,
    /// Owning plugin id.
    pub plugin_id: String,
    /// Command id to run.
    pub command_id: String,
    /// Human description (for the help overlay).
    pub description: String,
}

/// A content transformer a plugin declares for a code-fence language.
#[derive(Debug, Clone)]
pub struct TransformerEntry {
    /// Owning plugin id.
    pub plugin_id: String,
    /// Code-fence language this transformer handles.
    pub lang: String,
    /// `"text"` or `"rich"`.
    pub kind: String,
}

/// A toolbar button a plugin contributes to a GUI client's chrome.
#[derive(Debug, Clone)]
pub struct ToolbarButtonEntry {
    /// Owning plugin id.
    pub plugin_id: String,
    /// Command id to run on tap.
    pub command_id: String,
    /// Glyph/emoji to render.
    pub icon: String,
    /// Optional tooltip / accessible label.
    pub title: Option<String>,
}

impl PluginHost {
    /// Every command contributed by a loaded plugin whose `slash-command`
    /// capability is granted on this client.
    pub fn commands(&self) -> Vec<CommandEntry> {
        let mut out = Vec::new();
        for p in &self.plugins {
            if !p.has(Capability::SlashCommand) {
                continue;
            }
            for c in &p.manifest.contributes.commands {
                out.push(CommandEntry {
                    plugin_id: p.manifest.id.clone(),
                    command_id: c.id.clone(),
                    title: c.title.clone(),
                });
            }
        }
        out
    }

    /// Plugin keybindings for `client` (`"tui"` / `"desktop"` / `"mobile"`),
    /// parsed and ready to merge into the client's chord dispatcher. Only
    /// plugins granted the `keybinding` capability are included; a binding whose
    /// `when` names a different client, or whose chord string doesn't parse, is
    /// skipped.
    pub fn keybindings(&self, client: &str) -> Vec<PluginBinding> {
        let mut out = Vec::new();
        for p in &self.plugins {
            if !p.has(Capability::Keybinding) {
                continue;
            }
            for kb in &p.manifest.contributes.keybindings {
                if kb.when.as_deref().is_some_and(|w| w != client) {
                    continue;
                }
                let Some(chord) = ChordSequence::parse(&kb.key) else {
                    continue;
                };
                let description = p
                    .manifest
                    .contributes
                    .commands
                    .iter()
                    .find(|c| c.id == kb.command)
                    .map(|c| c.title.clone())
                    .unwrap_or_else(|| kb.command.clone());
                out.push(PluginBinding {
                    chord,
                    mode: Mode::Global,
                    plugin_id: p.manifest.id.clone(),
                    command_id: kb.command.clone(),
                    description,
                });
            }
        }
        out
    }

    /// Toolbar buttons for `client` (`"desktop"` / `"mobile"`). Only plugins
    /// granted the `toolbar-button` capability are included.
    pub fn toolbar_buttons(&self, client: &str) -> Vec<ToolbarButtonEntry> {
        let mut out = Vec::new();
        for p in &self.plugins {
            if !p.has(Capability::ToolbarButton) {
                continue;
            }
            for tb in &p.manifest.contributes.toolbar {
                if tb.when.as_deref().is_some_and(|w| w != client) {
                    continue;
                }
                out.push(ToolbarButtonEntry {
                    plugin_id: p.manifest.id.clone(),
                    command_id: tb.command.clone(),
                    icon: tb.icon.clone(),
                    title: tb.title.clone(),
                });
            }
        }
        out
    }

    /// Content transformers granted on this client, keyed by code-fence
    /// language. A client renders a fence by looking up its language here; if a
    /// transformer matches, it calls [`PluginHost::transform_block`]. `text`
    /// transformers need `content-transformer:text`, `rich` ones need
    /// `content-transformer:rich` — a client lacking the capability never sees
    /// the entry.
    pub fn transformers(&self) -> Vec<TransformerEntry> {
        let mut out = Vec::new();
        for p in &self.plugins {
            for t in &p.manifest.contributes.transformers {
                let cap = if t.kind == "rich" {
                    Capability::ContentTransformerRich
                } else {
                    Capability::ContentTransformerText
                };
                if !p.has(cap) {
                    continue;
                }
                out.push(TransformerEntry {
                    plugin_id: p.manifest.id.clone(),
                    lang: t.lang.clone(),
                    kind: t.kind.clone(),
                });
            }
        }
        out
    }
}

#[cfg(all(test, feature = "js"))]
mod tests {
    use super::*;
    use crate::manifest::PluginManifest;
    use crate::permission::PermissionSet;
    use serde_json::Value;

    #[test]
    fn keybindings_and_toolbar_are_parsed_and_gated() {
        let mut host = PluginHost::new(
            [Capability::Keybinding, Capability::ToolbarButton]
                .into_iter()
                .collect(),
        );
        let manifest = PluginManifest::parse(
            r#"{
            "id": "run.x.kt", "name": "KT", "version": "1.0.0", "api": "^1.0", "main": "i.js",
            "capabilities": ["keybinding", "toolbar-button"],
            "contributes": {
                "commands": [{ "id": "do-it", "title": "Do It" }],
                "keybindings": [
                    { "command": "do-it", "key": "Ctrl+Shift+D" },
                    { "command": "do-it", "key": "Cmd+T S", "when": "desktop" },
                    { "command": "do-it", "key": "Cmd+M", "when": "mobile" }
                ],
                "toolbar": [{ "command": "do-it", "icon": "📊", "title": "Stats" }]
            }
        }"#
            .as_bytes(),
        )
        .unwrap();
        host.load_plugin(
            manifest,
            "globalThis.__outl_register({activate(){}});",
            PermissionSet::new(vec![]),
            Value::Null,
        )
        .unwrap();

        // Desktop sees the unscoped + desktop-scoped chord, not the mobile one.
        let kb = host.keybindings("desktop");
        assert_eq!(
            kb.len(),
            2,
            "got: {:?}",
            kb.iter().map(|b| &b.command_id).collect::<Vec<_>>()
        );
        assert!(kb
            .iter()
            .all(|b| b.command_id == "do-it" && b.plugin_id == "run.x.kt"));
        // The 2-chord sequence parsed.
        assert!(kb.iter().any(|b| b.chord.len() == 2));

        // Toolbar button surfaces with its glyph.
        let tb = host.toolbar_buttons("desktop");
        assert_eq!(tb.len(), 1);
        assert_eq!(tb[0].icon, "📊");
        assert_eq!(tb[0].command_id, "do-it");
    }

    #[test]
    fn keybindings_dropped_without_capability() {
        // Client without the keybinding capability granted → nothing surfaces.
        let mut host = PluginHost::new([Capability::OpHook].into_iter().collect());
        let manifest = PluginManifest::parse(
            br#"{"id":"run.x.kt","name":"KT","version":"1.0.0","api":"^1.0","main":"i.js",
             "capabilities":["keybinding"],
             "contributes":{"commands":[{"id":"do-it","title":"Do It"}],
                            "keybindings":[{"command":"do-it","key":"Ctrl+D"}]}}"#,
        )
        .unwrap();
        host.load_plugin(
            manifest,
            "globalThis.__outl_register({activate(){}});",
            PermissionSet::new(vec![]),
            Value::Null,
        )
        .unwrap();
        assert!(host.keybindings("desktop").is_empty());
    }
}
