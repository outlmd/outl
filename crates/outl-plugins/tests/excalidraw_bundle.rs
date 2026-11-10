//! The Excalidraw example's **shipped bundle**, through the real Boa engine.
//!
//! `real_example_bundle_archives_done_blocks` already proves the authoring →
//! bundle → engine → host → outl-actions chain for a command. This covers the
//! other half, which no command test reaches: a `content-transformer:rich`
//! that does real work *inside* the interpreter.
//!
//! That matters because the renderer is the one place in `examples/` that
//! leans on the JS engine for more than control flow — a seeded PRNG, string
//! building, regex, `Math.*`, class fields. Node running the same bundle
//! proves the logic; only Boa proves the logic runs *here*.

use outl_plugins::{Capability, Permission, PermissionSet, PluginHost, PluginManifest};

const MANIFEST: &[u8] = include_bytes!("../../../examples/excalidraw/plugin.json");
const BUNDLE: &str = include_str!("../../../examples/excalidraw/index.js");
const PLUGIN_ID: &str = "app.outl.examples.excalidraw";

/// A scene exercising one of each path through the renderer: a filled and
/// rounded polygon, an ellipse, a dashed cross-hatched diamond, an arrow with
/// a head, a rotated element, text, and a freehand stroke.
const SCENE: &str = r##"```excalidraw
{"type":"excalidraw","appState":{"viewBackgroundColor":"#ffffff"},"elements":[
 {"id":"r","type":"rectangle","x":0,"y":0,"width":120,"height":80,"strokeColor":"#1971c2",
  "backgroundColor":"#a5d8ff","fillStyle":"hachure","strokeWidth":2,"roughness":1,
  "seed":1234,"roundness":{"type":3}},
 {"id":"e","type":"ellipse","x":150,"y":0,"width":90,"height":80,"backgroundColor":"#b2f2bb",
  "fillStyle":"solid","roughness":2,"seed":77},
 {"id":"d","type":"diamond","x":260,"y":0,"width":90,"height":80,"backgroundColor":"#ffc9c9",
  "fillStyle":"cross-hatch","strokeStyle":"dashed","seed":42,"roundness":{"type":2}},
 {"id":"a","type":"arrow","x":0,"y":120,"width":200,"height":30,"strokeWidth":2,"seed":9,
  "points":[[0,0],[100,20],[200,30]],"endArrowhead":"arrow"},
 {"id":"f","type":"freedraw","x":0,"y":180,"width":120,"height":40,"seed":8,
  "points":[[0,20],[30,0],[60,40],[90,5],[120,30]]},
 {"id":"t","type":"text","x":0,"y":240,"width":200,"height":50,"text":"two\nlines",
  "fontSize":16,"fontFamily":1,"textAlign":"left","verticalAlign":"top","seed":3},
 {"id":"rot","type":"rectangle","x":260,"y":180,"width":80,"height":50,"angle":0.45,
  "opacity":60,"strokeStyle":"dotted","seed":1515},
 {"id":"gone","type":"rectangle","x":0,"y":0,"width":10,"height":10,"isDeleted":true,"seed":1}
]}
```"##;

/// Load the plugin with the permissions a normal install grants.
fn host() -> PluginHost {
    host_with(vec![Permission::ReadPage, Permission::StorageLocal])
}

/// Load it with `permissions` and every capability it declares.
fn host_with(permissions: Vec<Permission>) -> PluginHost {
    let mut host = PluginHost::new(
        [
            Capability::ContentTransformerRich,
            Capability::SlashCommand,
            Capability::ToolbarButton,
            Capability::ConfigSchema,
        ]
        .into_iter()
        .collect(),
    );
    let manifest = PluginManifest::parse(MANIFEST).expect("the example manifest parses");
    host.load_plugin(
        manifest,
        BUNDLE,
        PermissionSet::new(permissions),
        serde_json::json!({}),
    )
    .expect("the shipped bundle loads into Boa");
    host
}

/// Strip the fence markers the host hands a transformer: it passes the body.
fn body(fence: &str) -> String {
    fence
        .trim_start_matches("```excalidraw\n")
        .trim_end_matches("```")
        .to_string()
}

#[test]
fn the_shipped_bundle_renders_a_pasted_scene_under_boa() {
    let mut host = host();

    let result = host
        .transform_block(PLUGIN_ID, "excalidraw", &body(SCENE))
        .expect("the transformer runs")
        .expect("it does not decline a scene it can draw");

    assert_eq!(result.kind, "rich");
    let html = &result.content;

    assert!(
        html.starts_with("<!doctype html>"),
        "not a document: {html:.80}"
    );
    assert!(html.contains("<svg "), "no SVG in the output");

    // Every element kind reached its renderer. Counting is what catches a
    // branch that threw and was swallowed into an empty string.
    assert!(html.matches("<path ").count() >= 9, "too few paths: {html}");
    assert!(html.contains("<pattern "), "no hachure pattern");
    assert!(html.contains("<tspan "), "no text");
    assert!(
        html.contains("rotate("),
        "the rotated element lost its transform"
    );
    assert!(
        html.contains("stroke-dasharray"),
        "dashed/dotted strokes lost"
    );
    assert!(html.contains("opacity=\"0.6\""), "opacity lost");

    // A coordinate that came out as NaN still renders as a document, and the
    // drawing is silently wrong — so the numbers are asserted, not just the tags.
    assert!(!html.contains("NaN"), "a coordinate is NaN");
    assert!(!html.contains("undefined"), "a value is undefined");
    assert!(!html.contains("Infinity"), "a coordinate is Infinity");
}

#[test]
fn a_fence_it_cannot_read_renders_the_reason_not_an_empty_box() {
    let mut host = host();

    for (body_text, expected) in [
        ("", "Empty fence"),
        ("{ not json", "Not valid JSON"),
        ("scene: abc\ntheme: neon", "Unknown theme"),
        (
            "scene: https://excalidraw.com/#room=a,b",
            "live collaboration",
        ),
        (
            "scene: https://excalidraw.com/#json=not-a-pair",
            "malformed",
        ),
        (
            "scene: https://excalidraw.com/#url=https://x.test/a",
            "other host",
        ),
    ] {
        let result = host
            .transform_block(PLUGIN_ID, "excalidraw", body_text)
            .expect("the transformer runs")
            .expect("a bad fence still produces a descriptor");

        assert_eq!(result.kind, "rich");
        assert!(
            result.content.contains(expected),
            "body {body_text:?} should explain {expected:?}, got: {}",
            result.content
        );
    }
}

#[test]
fn a_scene_reference_with_no_cache_says_what_to_run() {
    let mut host = host();

    let result = host
        .transform_block(PLUGIN_ID, "excalidraw", "scene: abc123")
        .expect("the transformer runs")
        .expect("an unsynced scene still produces a descriptor");

    assert!(
        result.content.contains("Not synced yet"),
        "got: {}",
        result.content
    );
    assert!(
        result.content.contains("excalidraw-sync"),
        "the message has to name the command that fixes it"
    );
}

/// The regression this file bought: `ctx.storage.get` **throws** without
/// `storage:local`, and an uncaught throw costs the whole descriptor — the
/// fence then draws an empty box and never says a permission is missing.
/// A pasted scene must keep working in the same host, since it needs no cache.
#[test]
fn a_scene_fence_without_storage_permission_explains_itself() {
    let mut host = host_with(vec![Permission::ReadPage]);

    let result = host
        .transform_block(PLUGIN_ID, "excalidraw", "scene: abc123")
        .expect("the transformer runs")
        .expect("a throwing host call must not cost the descriptor");

    assert!(
        result.content.contains("Cache unavailable"),
        "got: {}",
        result.content
    );
    assert!(
        result.content.contains("storage:local"),
        "the message has to name the permission: {}",
        result.content
    );

    let pasted = host
        .transform_block(PLUGIN_ID, "excalidraw", &body(SCENE))
        .expect("the transformer runs")
        .expect("a pasted scene needs no cache");
    assert!(
        pasted.content.contains("<svg "),
        "a pasted scene still draws"
    );
}

/// The loop the plugin is built around, end to end through the engine: a sync
/// writes the cache, and a later render reads it back out of `storage.json`.
///
/// The halves run in different turns and in different host entry points, which
/// is exactly where this used to break — a command could write a cache that
/// the transformer could never read.
#[test]
fn a_cached_scene_is_drawn_from_storage_on_disk() {
    let dir = std::env::temp_dir().join(format!("outl-excal-{}", std::process::id()));
    let plugin_dir = dir.join(PLUGIN_ID);
    std::fs::create_dir_all(&plugin_dir).expect("temp dir");
    std::fs::write(
        plugin_dir.join("storage.json"),
        serde_json::json!({
            "scenes": ["abc123"],
            "scene/abc123": {
                "id": "abc123",
                "name": "Synced scene",
                "sceneVersion": "4",
                "background": "#ffffff",
                "files": {},
                "filesDropped": false,
                "at": "2026-11-09T12:00:00Z",
                "elements": [{
                    "id": "r", "type": "rectangle", "x": 0.0, "y": 0.0,
                    "width": 100.0, "height": 60.0, "seed": 7,
                    "backgroundColor": "#a5d8ff", "fillStyle": "hachure"
                }]
            }
        })
        .to_string(),
    )
    .expect("seed the cache the way a sync would");

    let mut host = host();
    host.set_storage_dir(dir.clone());

    let result = host
        .transform_block(PLUGIN_ID, "excalidraw", "scene: abc123")
        .expect("the transformer runs")
        .expect("a cached scene draws");

    assert!(result.content.contains("<svg "), "got: {}", result.content);
    assert!(
        result.content.contains("<pattern "),
        "the hachure fill was lost"
    );
    assert!(
        result.content.contains("Synced scene"),
        "the caption should name the scene: {}",
        result.content
    );
    assert!(
        !result.content.contains("Not synced yet"),
        "the cache on disk was not read"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// A share link is a *reference*, not an error: the sync fetches and decrypts
/// it, and the transformer reads the cache like it does for any other scene.
/// So an unsynced one has to read as "not synced", with the id taken from the
/// link's `#json=<id>,<key>` rather than from a path segment.
#[test]
fn a_share_link_is_accepted_as_a_scene_reference() {
    let mut host = host();

    let result = host
        .transform_block(
            PLUGIN_ID,
            "excalidraw",
            "scene: https://excalidraw.com/#json=A1b2C3,xYz-_123",
        )
        .expect("the transformer runs")
        .expect("a share link is a scene it can name");

    assert!(
        result.content.contains("Not synced yet"),
        "a share link should be syncable, got: {}",
        result.content
    );
    assert!(
        result.content.contains("A1b2C3"),
        "the backend id is the cache key: {}",
        result.content
    );
    assert!(
        !result.content.contains("xYz-_123"),
        "the decryption key must not be echoed into the page"
    );
}

/// Every shape the README tells a user to write, through the real engine.
///
/// The bare-URL row is why this exists: a pasted link holds a colon of its
/// own (`https:`), the parser split on the first one, and the shorthand the
/// Quick start documents came back as "unknown key `https`". Every other row
/// worked, so nothing caught it.
#[test]
fn every_documented_fence_shape_resolves_to_a_scene() {
    let mut host = host();

    for body in [
        "abc123",
        "#json=A1b2C3,xYz-_123",
        "https://excalidraw.com/#json=A1b2C3,xYz-_123",
        "scene: https://excalidraw.com/#json=A1b2C3,xYz-_123",
        "scene: abc123\ntheme: dark\nheight: 200",
        "# a comment\nscene: abc123",
    ] {
        let result = host
            .transform_block(PLUGIN_ID, "excalidraw", body)
            .expect("the transformer runs")
            .expect("a descriptor comes back");

        assert!(
            result.content.contains("Not synced yet"),
            "{body:?} should name a syncable scene, got: {}",
            result.content
        );
    }

    // And the shorthand must not swallow a typo: an unknown key is still an
    // error rather than a line read as a scene reference.
    let typo = host
        .transform_block(PLUGIN_ID, "excalidraw", "scene: abc123\nwidht: 40")
        .expect("the transformer runs")
        .expect("a descriptor comes back");
    assert!(
        typo.content.contains("Unknown key"),
        "got: {}",
        typo.content
    );
}

/// A scene file is JSON somebody else wrote, so a field can hold anything.
/// Each row below threw before: `(1 || "").trim()` is not a function, and
/// `FONTS["constructor"]` resolves through `Object.prototype` to something
/// truthy that escaping then calls `.replace` on. A throw costs the whole
/// descriptor, so the fence drew nothing and said nothing.
#[test]
fn a_scene_with_hostile_field_types_still_draws() {
    let mut host = host();

    for (label, element) in [
        ("numeric stroke", r#""strokeColor":1"#),
        ("boolean stroke", r#""strokeColor":true"#),
        ("object background", r#""backgroundColor":{}"#),
        (
            "prototype font",
            r#""type":"text","text":"x","fontFamily":"constructor""#,
        ),
        (
            "prototype font 2",
            r#""type":"text","text":"x","fontFamily":"__proto__""#,
        ),
        ("array name", r#""type":"frame","name":[1,2]"#),
        ("huge coordinate", r#""x":1e308,"width":1e308"#),
    ] {
        let body = format!(
            r#"{{"type":"excalidraw","elements":[{{"type":"rectangle","x":0,"y":0,"width":10,"height":10,"seed":1,{element}}}]}}"#
        );

        let result = host
            .transform_block(PLUGIN_ID, "excalidraw", &body)
            .expect("the transformer runs")
            .unwrap_or_else(|| panic!("{label}: the transformer declined"));

        assert!(
            result.content.starts_with("<!doctype html>"),
            "{label}: no document"
        );
        assert!(
            !result.content.contains("Infinity") && !result.content.contains("NaN"),
            "{label}: a coordinate escaped into the output: {}",
            result.content
        );
    }
}

/// A `dataURL` is whatever the scene's author put there, and it lands in an
/// `<image href>`. A remote one would make the webview fetch it on every
/// repaint — a request the `network:` allowlist never sees, because it never
/// goes through `ctx.net.fetch`.
#[test]
fn a_remote_image_url_in_a_scene_is_never_emitted() {
    let mut host = host();

    let body = r#"{"type":"excalidraw",
        "elements":[{"type":"image","x":0,"y":0,"width":200,"height":80,"fileId":"a","seed":1}],
        "files":{"a":{"dataURL":"https://attacker.example/beacon.png?note=victim"}}}"#;

    let result = host
        .transform_block(PLUGIN_ID, "excalidraw", body)
        .expect("the transformer runs")
        .expect("the scene still draws");

    assert!(
        !result.content.contains("attacker.example"),
        "a remote href reached the page: {}",
        result.content
    );
    assert!(
        result.content.contains("image not cached"),
        "the dropped image should be visible as a placeholder: {}",
        result.content
    );
    // And the page carries its own policy, since the desktop ships none.
    assert!(
        result.content.contains("Content-Security-Policy"),
        "the rendered page declares no CSP"
    );
    assert!(
        result.content.contains("img-src data:"),
        "the CSP allows remote images"
    );
}

#[test]
fn an_unknown_language_is_declined() {
    let mut host = host();
    let result = host
        .transform_block(PLUGIN_ID, "mermaid", "graph TD;")
        .expect("the transformer runs");
    assert!(
        result.is_none(),
        "the plugin claimed a language it does not handle"
    );
}
