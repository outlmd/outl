use super::*;
use crate::permission::Permission;
use outl_core::id::ActorId;

const PLUGIN: &str = r#"
        globalThis.__outl_register({
            activate(ctx) {
                ctx.commands.register('archive-done', () => {
                    const done = ctx.blocks.query({ todo: 'DONE' });
                    for (const b of done) {
                        ctx.blocks.move(b.id, { toPage: ctx.config.get().archivePage });
                    }
                    ctx.ui.notify(done.length + ' archived');
                });
                ctx.ops.onOp((op) => {
                    if (op.kind === 'Edit' && (op.text || '').startsWith('flag')) {
                        ctx.log.info('flagged ' + op.node);
                    }
                });
            }
        });
    "#;

fn ws() -> (Workspace, HlcGenerator) {
    let actor = ActorId::new();
    let ws = Workspace::open_in_memory(actor).unwrap();
    (ws, HlcGenerator::new(actor))
}

fn host_with_plugin(caps: &[Capability], perms: &[Permission]) -> PluginHost {
    let mut host = PluginHost::new(caps.iter().copied().collect());
    let manifest = PluginManifest::parse(
        br#"{
                "id": "app.outl.examples.todo-archiver",
                "name": "Todo Archiver",
                "version": "1.0.0",
                "api": "^1.0",
                "main": "index.js",
                "capabilities": ["op-hook", "slash-command"],
                "permissions": ["read-page", "write-page", "submit-op"],
                "contributes": { "commands": [{ "id": "archive-done", "title": "Archive DONE" }] }
            }"#,
    )
    .unwrap();
    host.load_plugin(
        manifest,
        PLUGIN,
        PermissionSet::new(perms.to_vec()),
        serde_json::json!({ "archivePage": "archive" }),
    )
    .unwrap();
    host
}

#[test]
fn command_archives_done_blocks_to_a_page() {
    let (mut ws, hlc) = ws();
    let page = page::open_or_create(&mut ws, &hlc, "inbox", "Inbox", PageKind::Page).unwrap();
    block::create_under(&mut ws, &hlc, page, Some("DONE task one")).unwrap();
    block::create_under(&mut ws, &hlc, page, Some("TODO task two")).unwrap();
    block::create_under(&mut ws, &hlc, page, Some("DONE task three")).unwrap();

    let mut host = host_with_plugin(
        &[Capability::SlashCommand, Capability::OpHook],
        &[
            Permission::ReadPage,
            Permission::WritePage,
            Permission::SubmitOp,
        ],
    );

    let run = host
        .run_command(
            &mut ws,
            &hlc,
            "app.outl.examples.todo-archiver",
            "archive-done",
        )
        .unwrap();
    assert_eq!(run.applied, 2, "two DONE blocks moved");
    assert_eq!(run.notifications, vec!["2 archived"]);
    assert!(run.errors.is_empty(), "errors: {:?}", run.errors);

    // The archive page now exists and holds the two DONE blocks.
    let archive = page::find_by_slug(&ws, "archive").expect("archive page created");
    let rm = build_read_model(&ws);
    let archived: Vec<_> = rm.blocks.iter().filter(|b| b.page == "archive").collect();
    assert_eq!(archived.len(), 2);
    let _ = archive;
}

#[test]
fn commands_lists_contributed_command() {
    let host = host_with_plugin(&[Capability::SlashCommand], &[]);
    let cmds = host.commands();
    assert_eq!(cmds.len(), 1);
    assert_eq!(cmds[0].command_id, "archive-done");
    assert_eq!(cmds[0].plugin_id, "app.outl.examples.todo-archiver");
}

#[test]
fn missing_capability_drops_command_from_palette() {
    // Client without slash-command support: no commands surface.
    let host = host_with_plugin(&[Capability::OpHook], &[]);
    assert!(host.commands().is_empty());
    assert!(host
        .missing_capabilities("app.outl.examples.todo-archiver")
        .contains(&Capability::SlashCommand));
}

#[test]
fn denied_permission_blocks_mutation() {
    let (mut ws, hlc) = ws();
    let page = page::open_or_create(&mut ws, &hlc, "inbox", "Inbox", PageKind::Page).unwrap();
    block::create_under(&mut ws, &hlc, page, Some("DONE x")).unwrap();
    // No write-page / submit-op approved.
    let mut host = host_with_plugin(&[Capability::SlashCommand], &[Permission::ReadPage]);
    let run = host
        .run_command(
            &mut ws,
            &hlc,
            "app.outl.examples.todo-archiver",
            "archive-done",
        )
        .unwrap();
    assert_eq!(run.applied, 0);
    assert!(!run.errors.is_empty(), "expected a permission-denied error");
}

/// End-to-end with the **real** shipped bundle: load the example plugin's
/// `plugin.json` + esbuild-produced `index.js` (built from `src/index.ts`
/// against the actual `@outl/plugin-sdk`) and run its command. Proves the
/// whole authoring → bundle → engine → host → outl-actions chain works.
#[test]
fn real_example_bundle_archives_done_blocks() {
    const MANIFEST: &[u8] = include_bytes!("../../../examples/todo-archiver/plugin.json");
    const BUNDLE: &str = include_str!("../../../examples/todo-archiver/index.js");

    let (mut ws, hlc) = ws();
    let page = page::open_or_create(&mut ws, &hlc, "inbox", "Inbox", PageKind::Page).unwrap();
    block::create_under(&mut ws, &hlc, page, Some("DONE shipped")).unwrap();
    block::create_under(&mut ws, &hlc, page, Some("TODO pending")).unwrap();
    block::create_under(&mut ws, &hlc, page, Some("DONE merged")).unwrap();

    let mut host = PluginHost::new(
        [Capability::SlashCommand, Capability::OpHook]
            .into_iter()
            .collect(),
    );
    let manifest = PluginManifest::parse(MANIFEST).unwrap();
    host.load_plugin(
        manifest,
        BUNDLE,
        PermissionSet::new(vec![
            Permission::ReadPage,
            Permission::WritePage,
            Permission::SubmitOp,
            Permission::StorageLocal,
        ]),
        serde_json::json!({ "archivePage": "done-archive" }),
    )
    .unwrap();

    let run = host
        .run_command(
            &mut ws,
            &hlc,
            "app.outl.examples.todo-archiver",
            "todo-archive-done",
        )
        .unwrap();
    assert!(run.errors.is_empty(), "errors: {:?}", run.errors);
    assert_eq!(run.applied, 3, "ensure-page + two DONE moves");
    assert!(
        run.notifications.iter().any(|n| n.contains("Archived 2")),
        "notes: {:?}",
        run.notifications
    );

    let rm = build_read_model(&ws);
    let archived: Vec<_> = rm
        .blocks
        .iter()
        .filter(|b| b.page == "done-archive")
        .collect();
    assert_eq!(archived.len(), 2);
}

#[test]
fn op_hook_fires_on_user_edit_then_is_loop_safe() {
    let (mut ws, hlc) = ws();
    let page = page::open_or_create(&mut ws, &hlc, "inbox", "Inbox", PageKind::Page).unwrap();
    let b = block::create_under(&mut ws, &hlc, page, Some("plain")).unwrap();

    let mut host = host_with_plugin(
        &[Capability::OpHook],
        &[
            Permission::ReadOpLog,
            Permission::ReadPage,
            Permission::WritePage,
            Permission::SubmitOp,
        ],
    );
    host.mark_synced(&ws);

    // User edits the block to start with "flag" → hook logs it.
    block::edit_text(&mut ws, &hlc, b, "flag this").unwrap();
    let run = host.sync_hooks(&mut ws, &hlc).unwrap();
    assert!(
        run.logs.iter().any(|l| l.starts_with("flagged ")),
        "logs: {:?}",
        run.logs
    );

    // A second sweep with no new ops does nothing.
    let again = host.sync_hooks(&mut ws, &hlc).unwrap();
    assert!(again.logs.is_empty());
}

const UI_PLUGIN: &str = r#"
        globalThis.__outl_register({ activate(ctx) {
            ctx.ops.onOp((op) => {
                if (op.kind === 'Edit') ctx.ui.render('<b>' + op.node + '</b>');
            });
        }});
    "#;

fn ui_host(client_caps: &[Capability]) -> (Workspace, HlcGenerator, PluginHost, NodeId) {
    let (mut ws, hlc) = ws();
    let page = page::open_or_create(&mut ws, &hlc, "p", "P", PageKind::Page).unwrap();
    let b = block::create_under(&mut ws, &hlc, page, Some("x")).unwrap();
    let mut host = PluginHost::new(client_caps.iter().copied().collect());
    let manifest = PluginManifest::parse(
        br#"{"id":"run.x.ui","name":"UI","version":"1.0.0","api":"^1.0","main":"i.js",
                 "capabilities":["op-hook","ui-render"],"permissions":["read-op-log"]}"#,
    )
    .unwrap();
    host.load_plugin(
        manifest,
        UI_PLUGIN,
        PermissionSet::new(vec![Permission::ReadOpLog]),
        Value::Null,
    )
    .unwrap();
    host.mark_synced(&ws);
    (ws, hlc, host, b)
}

#[test]
fn ui_render_flows_when_capability_granted() {
    // GUI-like client implements ui-render → the author's markup reaches it.
    let (mut ws, hlc, mut host, b) = ui_host(&[Capability::OpHook, Capability::UiRender]);
    block::edit_text(&mut ws, &hlc, b, "changed").unwrap();
    let run = host.sync_hooks(&mut ws, &hlc).unwrap();
    assert_eq!(run.views.len(), 1);
    assert!(run.views[0].contains("<b>"));
}

#[test]
fn ui_render_is_dropped_when_capability_absent() {
    // TUI-like client without ui-render → views are gated out (not rendered).
    let (mut ws, hlc, mut host, b) = ui_host(&[Capability::OpHook]);
    block::edit_text(&mut ws, &hlc, b, "changed").unwrap();
    let run = host.sync_hooks(&mut ws, &hlc).unwrap();
    assert!(run.views.is_empty());
}

/// The real shipped confetti bundle: toggling a block to DONE makes its
/// `onOp` emit the author's confetti HTML through `ctx.ui.render`.
#[test]
fn real_confetti_bundle_emits_view_on_done() {
    const MANIFEST: &[u8] = include_bytes!("../../../examples/confetti/plugin.json");
    const BUNDLE: &str = include_str!("../../../examples/confetti/index.js");

    let (mut ws, hlc) = ws();
    let page = page::open_or_create(&mut ws, &hlc, "p", "P", PageKind::Page).unwrap();
    let b = block::create_under(&mut ws, &hlc, page, Some("ship it")).unwrap();

    let mut host = PluginHost::new(
        [Capability::OpHook, Capability::UiRender]
            .into_iter()
            .collect(),
    );
    let manifest = PluginManifest::parse(MANIFEST).unwrap();
    host.load_plugin(
        manifest,
        BUNDLE,
        PermissionSet::new(vec![Permission::ReadOpLog]),
        Value::Null,
    )
    .unwrap();
    host.mark_synced(&ws);

    // One sweep per commit, like a real client (the desktop calls
    // sync_hooks after each commit). Only the DONE transition fires the
    // author's confetti — the two states before it draw nothing.
    block::toggle_todo(&mut ws, &hlc, b).unwrap(); // → TODO
    let todo_run = host.sync_hooks(&mut ws, &hlc).unwrap();
    assert!(todo_run.views.is_empty(), "no confetti on TODO");

    block::toggle_todo(&mut ws, &hlc, b).unwrap(); // → DOING
    let doing_run = host.sync_hooks(&mut ws, &hlc).unwrap();
    assert!(doing_run.views.is_empty(), "no confetti on DOING");

    block::toggle_todo(&mut ws, &hlc, b).unwrap(); // → DONE
    let done_run = host.sync_hooks(&mut ws, &hlc).unwrap();
    assert_eq!(
        done_run.views.len(),
        1,
        "one confetti burst on the DONE transition"
    );
    assert!(
        done_run.views[0].contains("<canvas"),
        "author's confetti markup"
    );
}

#[test]
fn keybindings_and_toolbar_are_parsed_and_gated() {
    use crate::capability::Capability;
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
    use crate::capability::Capability;
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

#[test]
fn content_transformer_runs_and_is_capability_gated() {
    use crate::capability::Capability;
    const BUNDLE: &str = r#"
        globalThis.__outl_register({ activate(ctx) {
            ctx.content.register('upper', (text) => ({ kind: 'text', content: text.toUpperCase() }));
        }});
    "#;
    let manifest_json = r#"{
        "id": "run.x.up", "name": "Up", "version": "1.0.0", "api": "^1.0", "main": "i.js",
        "capabilities": ["content-transformer:text"],
        "contributes": { "transformers": [{ "lang": "upper", "kind": "text" }] }
    }"#;

    // Client implements text transformers → declared + runs.
    let mut host = PluginHost::new([Capability::ContentTransformerText].into_iter().collect());
    host.load_plugin(
        PluginManifest::parse(manifest_json.as_bytes()).unwrap(),
        BUNDLE,
        PermissionSet::new(vec![]),
        Value::Null,
    )
    .unwrap();
    let decls = host.transformers();
    assert_eq!(decls.len(), 1);
    assert_eq!(decls[0].lang, "upper");
    let out = host
        .transform_block("run.x.up", "upper", "hello")
        .unwrap()
        .unwrap();
    assert_eq!(out.kind, "text");
    assert_eq!(out.content, "HELLO");
    // Unknown language → None.
    assert!(host
        .transform_block("run.x.up", "mermaid", "x")
        .unwrap()
        .is_none());

    // Client WITHOUT the capability → the transformer is not surfaced.
    let mut bare = PluginHost::new([Capability::OpHook].into_iter().collect());
    bare.load_plugin(
        PluginManifest::parse(manifest_json.as_bytes()).unwrap(),
        BUNDLE,
        PermissionSet::new(vec![]),
        Value::Null,
    )
    .unwrap();
    assert!(bare.transformers().is_empty());
}

#[test]
fn net_fetch_refuses_unapproved_domain() {
    use crate::capability::Capability;
    use crate::permission::NetworkDomain;
    let bundle = r#"
        globalThis.__outl_register({ activate(ctx) {
            ctx.commands.register('try', () => {
                const r = ctx.net.fetch('http://evil.example.org/x');
                ctx.ui.notify(r.ok ? 'ok' : ('denied:' + r.error));
            });
        }});
    "#;
    let manifest = PluginManifest::parse(
        r#"{
            "id": "run.x.net", "name": "Net", "version": "1.0.0", "api": "^1.0", "main": "i.js",
            "capabilities": ["slash-command"],
            "permissions": ["network:*.openai.com"],
            "contributes": { "commands": [{ "id": "try", "title": "Try" }] }
        }"#
        .as_bytes(),
    )
    .unwrap();
    let mut host = PluginHost::new([Capability::SlashCommand].into_iter().collect());
    host.load_plugin(
        manifest,
        bundle,
        PermissionSet::new(vec![Permission::Network(
            NetworkDomain::parse("*.openai.com").unwrap(),
        )]),
        Value::Null,
    )
    .unwrap();

    let (mut ws, hlc) = ws();
    // The fetch host (evil.example.org) is not covered by network:*.openai.com,
    // so it's refused inside the engine without touching the network.
    let run = host.run_command(&mut ws, &hlc, "run.x.net", "try").unwrap();
    assert_eq!(run.notifications.len(), 1);
    assert!(
        run.notifications[0].contains("denied"),
        "got: {:?}",
        run.notifications
    );
}

#[test]
fn sync_transport_carries_ops_between_workspaces() {
    use crate::capability::Capability;
    // A loopback transport: push stashes the JSONL in a global, pull returns it.
    // Stands in for "ship to backend / fetch from backend" without a network.
    const SYNC_PLUGIN: &str = r#"
        globalThis.__buf = '';
        globalThis.__outl_register({ activate(ctx) {
            ctx.sync.register({
                push: (jsonl) => { globalThis.__buf = jsonl; },
                pull: () => globalThis.__buf,
            });
        }});
    "#;
    let manifest = PluginManifest::parse(
        br#"{"id":"run.x.sync","name":"Sync","version":"1.0.0","api":"^1.0","main":"i.js",
             "capabilities":["sync-transport"]}"#,
    )
    .unwrap();
    let mut host = PluginHost::new([Capability::SyncTransport].into_iter().collect());
    host.load_plugin(
        manifest,
        SYNC_PLUGIN,
        PermissionSet::new(vec![]),
        Value::Null,
    )
    .unwrap();

    // Device 1 authors a page + block, then pushes its local ops to the transport.
    let (mut ws1, hlc1) = ws();
    let page = page::open_or_create(&mut ws1, &hlc1, "shared", "Shared", PageKind::Page).unwrap();
    block::create_under(&mut ws1, &hlc1, page, Some("hello from device 1")).unwrap();
    let pushed = host.sync_push(&ws1).unwrap();
    assert!(pushed >= 2, "pushed page + block ops, got {pushed}");

    // Device 2 starts empty; pulling applies device 1's ops through the CRDT.
    let (mut ws2, hlc2) = ws();
    assert!(page::find_by_slug(&ws2, "shared").is_none());
    let applied = host.sync_pull(&mut ws2, &hlc2).unwrap();
    assert!(applied >= 2, "applied the transported ops, got {applied}");
    assert!(
        page::find_by_slug(&ws2, "shared").is_some(),
        "page converged onto device 2"
    );
}

/// Every shipped example plugin must parse + load: a valid manifest and a
/// bundle whose `activate` runs clean in the engine. Guards the whole
/// `examples/` gallery against bit-rot in one shot.
#[test]
fn every_shipped_example_plugin_loads() {
    use crate::capability::Capability;
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples");
    let all_caps: ClientCapabilities = [
        Capability::OpHook,
        Capability::SlashCommand,
        Capability::Keybinding,
        Capability::ConfigSchema,
        Capability::UiRender,
        Capability::ToolbarButton,
        Capability::ContentTransformerText,
        Capability::ContentTransformerRich,
        Capability::QueryProvider,
        Capability::SyncTransport,
    ]
    .into_iter()
    .collect();

    let mut loaded = Vec::new();
    for entry in std::fs::read_dir(dir).expect("examples dir") {
        let path = entry.unwrap().path();
        let manifest_path = path.join("plugin.json");
        let bundle_path = path.join("index.js");
        if !manifest_path.exists() || !bundle_path.exists() {
            continue;
        }
        let manifest = PluginManifest::parse(&std::fs::read(&manifest_path).unwrap())
            .unwrap_or_else(|e| panic!("{}: invalid manifest: {e}", path.display()));
        let id = manifest.id.clone();
        let perms = PermissionSet::new(manifest.permissions.clone());
        let bundle = std::fs::read_to_string(&bundle_path).unwrap();
        let mut host = PluginHost::new(all_caps.clone());
        host.load_plugin(manifest, &bundle, perms, Value::Null)
            .unwrap_or_else(|e| panic!("{}: load failed: {e}", path.display()));
        loaded.push(id);
    }
    // The two combo examples plus one per capability.
    assert!(
        loaded.len() >= 9,
        "loaded only {}: {:?}",
        loaded.len(),
        loaded
    );
}

#[test]
fn storage_persists_across_turns_and_is_gated() {
    use crate::capability::Capability;
    use crate::permission::Permission;
    let bundle = r#"
        globalThis.__outl_register({ activate(ctx) {
            ctx.commands.register('save', () => { ctx.storage.set('n', (ctx.storage.get('n') || 0) + 1); });
            ctx.commands.register('show', () => { ctx.ui.notify('n=' + ctx.storage.get('n')); });
        }});
    "#;
    let manifest_json = r#"{
        "id": "app.outl.examples.kv", "name": "KV", "version": "1.0.0", "api": "^1.0", "main": "i.js",
        "capabilities": ["slash-command"], "permissions": ["storage:local"],
        "contributes": { "commands": [{ "id": "save", "title": "Save" }, { "id": "show", "title": "Show" }] }
    }"#;
    let tmp = tempfile::tempdir().unwrap();
    let id = "app.outl.examples.kv";

    let mut host = PluginHost::new([Capability::SlashCommand].into_iter().collect());
    host.set_storage_dir(tmp.path().to_path_buf());
    host.load_plugin(
        PluginManifest::parse(manifest_json.as_bytes()).unwrap(),
        bundle,
        PermissionSet::new(vec![Permission::StorageLocal]),
        Value::Null,
    )
    .unwrap();

    let (mut ws, hlc) = ws();
    host.run_command(&mut ws, &hlc, id, "save").unwrap(); // null -> 1
    host.run_command(&mut ws, &hlc, id, "save").unwrap(); // 1 -> 2 (loaded from disk)
    let run = host.run_command(&mut ws, &hlc, id, "show").unwrap();
    assert_eq!(
        run.notifications,
        vec!["n=2"],
        "storage survived between turns"
    );
    assert!(
        tmp.path().join(id).join("storage.json").exists(),
        "persisted to disk"
    );

    // Without the storage:local permission, ctx.storage throws.
    let mut bare = PluginHost::new([Capability::SlashCommand].into_iter().collect());
    bare.set_storage_dir(tmp.path().to_path_buf());
    bare.load_plugin(
        PluginManifest::parse(manifest_json.as_bytes()).unwrap(),
        bundle,
        PermissionSet::new(vec![]), // no storage:local
        Value::Null,
    )
    .unwrap();
    let denied = bare.run_command(&mut ws, &hlc, id, "save");
    assert!(denied.is_err(), "storage without permission should error");
}

/// A plugin `InstantiateTemplate` intent on a journal page must resolve
/// `{{date}}` to the journal's OWN date (derived from its slug), not to
/// today — matching the CLI/TUI path. Regression for the footgun where
/// the host passed `page_date: None` and every plugin instantiation
/// rendered today's date regardless of which page the block lived on.
#[test]
fn instantiate_template_intent_uses_journal_page_date() {
    use outl_core::property::PropValue;

    let (mut ws, hlc) = ws();

    // A template whose body echoes `{{date}}`.
    let tpl =
        page::open_or_create(&mut ws, &hlc, "template-daily", "daily", PageKind::Page).unwrap();
    page::set_property(
        &mut ws,
        &hlc,
        tpl,
        tpl_actions::TEMPLATE_KEY,
        Some(PropValue::Text("daily".into())),
    )
    .unwrap();
    block::append_block(&mut ws, &hlc, Some(tpl), Some("day is {{date}}")).unwrap();

    // A journal page dated well in the past, with a host block.
    let journal =
        page::open_or_create(&mut ws, &hlc, "2020-01-02", "2020-01-02", PageKind::Journal).unwrap();
    let host_block = block::append_block(&mut ws, &hlc, Some(journal), Some("host")).unwrap();

    let intent = HostIntent::InstantiateTemplate {
        name: "daily".into(),
        under: host_block.to_string(),
    };
    apply_one(&mut ws, &hlc, &intent).unwrap();

    // The cloned block must carry the journal's date, never today's.
    let clone_text = outl_actions::tree::children_of(&ws, host_block)
        .into_iter()
        .filter_map(|(id, _)| ws.block_text(id))
        .find(|t| t.starts_with("day is"))
        .expect("template block was cloned under the host");
    assert!(
        clone_text.contains("2020-01-02"),
        "`{{{{date}}}}` should resolve to the journal's date, got: {clone_text}"
    );
}

// --- ctx.secrets ------------------------------------------------------------

const SECRET_PLUGIN: &str = r#"
        globalThis.__outl_register({
            activate(ctx) {
                ctx.commands.register('show-secret', () => {
                    const t = ctx.secrets.get('token');
                    ctx.ui.notify(t === null ? 'none' : ('token=' + t));
                });
            }
        });
    "#;

const SECRET_READER_ID: &str = "app.outl.examples.secret-reader";

fn host_with_secret_plugin(
    perms: &[Permission],
    store: std::rc::Rc<dyn crate::secrets::SecretStore>,
) -> PluginHost {
    let mut host = PluginHost::new([Capability::SlashCommand].into_iter().collect());
    host.set_secret_store(store);
    let manifest = PluginManifest::parse(
        br#"{
                "id": "app.outl.examples.secret-reader",
                "name": "Secret Reader",
                "version": "1.0.0",
                "api": "^1.0",
                "main": "index.js",
                "capabilities": ["slash-command"],
                "permissions": ["secrets"],
                "contributes": { "commands": [{ "id": "show-secret", "title": "Show secret" }] }
            }"#,
    )
    .unwrap();
    host.load_plugin(
        manifest,
        SECRET_PLUGIN,
        PermissionSet::new(perms.to_vec()),
        serde_json::json!({}),
    )
    .unwrap();
    host
}

#[test]
fn granted_plugin_reads_its_secret_from_the_store() {
    let (mut ws, hlc) = ws();
    let store = std::rc::Rc::new(crate::secrets::MemorySecretStore::new());
    store
        .set(
            &crate::secrets::plugin_service(SECRET_READER_ID),
            "token",
            "pat-123",
        )
        .unwrap();

    let mut host = host_with_secret_plugin(&[Permission::Secrets], store);
    let run = host
        .run_command(&mut ws, &hlc, SECRET_READER_ID, "show-secret")
        .unwrap();
    assert_eq!(run.notifications, vec!["token=pat-123"]);
    assert!(run.errors.is_empty(), "errors: {:?}", run.errors);
}

#[test]
fn granted_plugin_gets_null_when_secret_unset() {
    let (mut ws, hlc) = ws();
    // Store is empty: a granted read of a never-set key returns null, not an error.
    let store = std::rc::Rc::new(crate::secrets::MemorySecretStore::new());
    let mut host = host_with_secret_plugin(&[Permission::Secrets], store);
    let run = host
        .run_command(&mut ws, &hlc, SECRET_READER_ID, "show-secret")
        .unwrap();
    assert_eq!(run.notifications, vec!["none"]);
}

#[test]
fn plugin_without_secrets_permission_is_denied() {
    let (mut ws, hlc) = ws();
    // Secret exists in the store, but the plugin holds no `secrets` grant:
    // ctx.secrets.get must throw, surfacing as an engine error, never the value.
    let store = std::rc::Rc::new(crate::secrets::MemorySecretStore::new());
    store
        .set(
            &crate::secrets::plugin_service(SECRET_READER_ID),
            "token",
            "pat-should-not-leak",
        )
        .unwrap();

    let mut host = host_with_secret_plugin(&[], store);
    let result = host.run_command(&mut ws, &hlc, SECRET_READER_ID, "show-secret");
    assert!(
        result.is_err(),
        "reading a secret without the `secrets` permission must fail, got {result:?}"
    );
}

#[test]
fn one_plugin_cannot_read_anothers_secret() {
    let (mut ws, hlc) = ws();
    // The token is stored under a *different* plugin's service namespace.
    let store = std::rc::Rc::new(crate::secrets::MemorySecretStore::new());
    store
        .set(
            &crate::secrets::plugin_service("app.outl.examples.other-plugin"),
            "token",
            "not-yours",
        )
        .unwrap();

    let mut host = host_with_secret_plugin(&[Permission::Secrets], store);
    let run = host
        .run_command(&mut ws, &hlc, SECRET_READER_ID, "show-secret")
        .unwrap();
    // The reader is namespaced to its own service, so the other plugin's secret
    // is invisible: it sees null.
    assert_eq!(run.notifications, vec!["none"]);
}

// --- AppendTree: seed a fresh page in one turn ------------------------------

#[test]
fn append_tree_seeds_a_fresh_page_in_one_turn() {
    use std::str::FromStr;

    let (mut ws, hlc) = ws();
    // The page does not exist yet — this is exactly the case a plugin can't
    // handle with `create` (no parent id to hand it mid-turn).
    let intent = HostIntent::AppendTree {
        target: MoveTarget::ToPage {
            to_page: "ouraring-2025-11-29".into(),
        },
        tree: vec![crate::model::TreeNode {
            text: "#ouraring anchor".into(),
            children: vec![
                crate::model::TreeNode {
                    text: "Sleep — Score 85".into(),
                    children: vec![],
                },
                crate::model::TreeNode {
                    text: "Readiness — Score 78".into(),
                    children: vec![],
                },
            ],
        }],
    };
    apply_one(&mut ws, &hlc, &intent).unwrap();

    // The page now exists — its title keeps the pretty `/`-name, its slug is
    // the slugified filename-safe form.
    let rm = build_read_model(&ws);
    assert!(
        rm.pages.iter().any(|p| p.slug == "ouraring-2025-11-29"),
        "flat page created from the slug, got: {:?}",
        rm.pages.iter().map(|p| &p.slug).collect::<Vec<_>>()
    );
    // …with the anchor as a top-level block on it.
    let anchor = rm
        .blocks
        .iter()
        .find(|b| b.text == "#ouraring anchor")
        .expect("anchor block created on the fresh page");

    // …and the two section lines nested under it (verified via the tree, not
    // just page membership).
    let anchor_id = NodeId(ulid::Ulid::from_str(&anchor.id).unwrap());
    let kids = outl_actions::tree::children_of(&ws, anchor_id);
    assert_eq!(kids.len(), 2, "anchor has its two children");
    let texts: Vec<String> = kids
        .into_iter()
        .filter_map(|(id, _)| ws.block_text(id))
        .collect();
    assert!(texts.iter().any(|t| t.starts_with("Sleep")));
    assert!(texts.iter().any(|t| t.starts_with("Readiness")));
}
