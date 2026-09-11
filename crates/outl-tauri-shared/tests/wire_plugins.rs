//! The plugin surface, pinned.
//!
//! These eight carried an `UNPINNED` row each in `wire_types.rs`, all
//! saying the same thing: *"needs a live PluginService to construct"*.
//! None of them did. Seven are plain `pub` structs with `pub` fields and
//! the eighth holds an `Option<PageView>`; the premise belonged to
//! `PluginRunDto`, a **ninth** type with a `pub(crate)` field — and that
//! one never crosses the wire, so it has no TypeScript mirror to pin in
//! the first place.
//!
//! That is root `CLAUDE.md` invariant 10's "exemptions that outlived
//! their premise", eight rows deep. Nobody re-checked the argument,
//! because writing a reason down reads the same as the reason being
//! true.

mod ts_parser;
mod wire_fixtures;
mod wire_pin;

use outl_tauri_shared::commands::plugin::{PluginRunReply, PluginSyncHooksReply};
use outl_tauri_shared::plugin_dto::{
    PluginCommandDto, ToolbarButtonDto, TransformResultDto, TransformerDto,
};

use wire_fixtures::page_view;
use wire_pin::assert_wire_shape;

#[test]
fn plugin_command_matches_its_interface() {
    let dto = PluginCommandDto {
        plugin_id: "app.outl.demo".into(),
        command_id: "demo.run".into(),
        title: "Run the demo".into(),
    };
    assert_wire_shape(&dto, "PluginCommand", &[]);
}

#[test]
fn plugin_toolbar_button_matches_its_interface() {
    let dto = ToolbarButtonDto {
        plugin_id: "app.outl.demo".into(),
        command_id: "demo.run".into(),
        icon: "✨".into(),
        title: Some("Run the demo".into()),
    };
    assert_wire_shape(&dto, "PluginToolbarButton", &[]);
}

#[test]
fn plugin_transformer_matches_its_interface() {
    let dto = TransformerDto {
        plugin_id: "app.outl.demo".into(),
        lang: "mermaid".into(),
        kind: "rich".into(),
    };
    assert_wire_shape(&dto, "PluginTransformer", &[]);
}

#[test]
fn plugin_transform_result_matches_its_interface() {
    let dto = TransformResultDto {
        kind: "rich".into(),
        content: "<svg/>".into(),
    };
    assert_wire_shape(&dto, "PluginTransformResult", &[]);
}

#[test]
fn plugin_run_reply_matches_its_interface() {
    let dto = PluginRunReply {
        applied: 2,
        notifications: vec!["done".into()],
        errors: Vec::new(),
        view: Some(page_view()),
        views: vec!["<p>hi</p>".into()],
    };
    assert_wire_shape(&dto, "PluginRunReply", &[]);
}

#[test]
fn plugin_sync_hooks_reply_matches_its_interface() {
    let dto = PluginSyncHooksReply {
        view: Some(page_view()),
        views: vec!["<p>hi</p>".into()],
        errors: vec!["projection refused".into()],
    };
    assert_wire_shape(&dto, "PluginSyncHooksReply", &[]);
}

#[test]
fn plugin_settings_field_matches_its_interface() {
    let dto = outl_plugins::SettingsField {
        key: "relay_url".into(),
        title: "Relay URL".into(),
        description: Some("Where the plugin posts".into()),
        kind: outl_plugins::FieldKind::String,
        secret: false,
        default: Some(serde_json::json!("https://example.test")),
        value: Some(serde_json::json!("https://relay.test")),
        is_set: false,
    };
    assert_wire_shape(&dto, "PluginSettingsField", &[]);
}

#[test]
fn registry_item_matches_its_interface() {
    let dto = outl_plugins::MarketplaceItem {
        id: "app.outl.demo".into(),
        name: "Demo".into(),
        description: "A demo plugin".into(),
        author: Some("avelino".into()),
        category: Some("utilities".into()),
        capabilities: vec!["ui-render".into()],
        permissions: vec!["workspace:read".into()],
        latest: Some("0.1.0".into()),
        installed: true,
        enabled: true,
    };
    assert_wire_shape(&dto, "RegistryItem", &[]);
}
