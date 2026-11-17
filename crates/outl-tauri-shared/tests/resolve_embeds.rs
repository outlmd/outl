//! `resolve_embeds` is the one round-trip the GUI clients use to turn a
//! `((blk-…))` / `!((blk-…))` handle into renderable content (issue #147).
//! This guards that a resolved handle carries both the source block's
//! `text` (what an inline `((…))` ref renders) and its `children`
//! subtree with inline tokens (what a `!((…))` embed expands) — the two
//! surfaces the desktop drives off one call.
//!
//! `resolve_page_embeds` is its page-level twin, for `![[page]]` (issue
//! #190), and it is tested here rather than beside it because the two
//! answer the same question at two granularities: a client resolves both
//! on the same navigation, and a divergence between them (a different
//! depth cap, a different "not found" shape) is the drift worth failing
//! on. Both are driven through the same `TestHost`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use outl_actions::{
    append_block, apply_page_md_with_sidecar, open_or_create_page, PageKind, SyncTransport,
};
use outl_core::hlc::HlcGenerator;
use outl_core::id::{ActorId, NodeId};
use outl_core::storage::JsonlStorage;
use outl_core::workspace::Workspace;
use outl_exec::RuntimeRegistry;
use outl_md::sidecar::derive_ref_handle;
use outl_tauri_shared::commands::exec::{resolve_embeds, resolve_page_embeds};
use outl_tauri_shared::host::AppHost;
use parking_lot::Mutex;
// `resolve_embeds` is `async` because it builds a `WorkspaceIndex` from
// disk and must not do that on the IPC thread (`outl_tauri_shared::offload`).
// A plain `#[test]` has no executor, so drive it on Tauri's own runtime —
// the same one the command would run on in the app.
use tauri::async_runtime::block_on;
use tempfile::TempDir;

/// Minimal [`AppHost`] for exercising a read-only command: `resolve_embeds`
/// only reads `storage_root()`, so the rest is boilerplate the trait
/// requires. Mirrors the shape both real clients implement.
struct TestHost {
    workspace: Arc<Mutex<Option<Workspace>>>,
    hlc: HlcGenerator,
    root: PathBuf,
    registry: Arc<RuntimeRegistry>,
}

impl AppHost for TestHost {
    fn workspace(&self) -> &Mutex<Option<Workspace>> {
        &self.workspace
    }
    fn workspace_arc(&self) -> Arc<Mutex<Option<Workspace>>> {
        self.workspace.clone()
    }
    fn hlc(&self) -> &HlcGenerator {
        &self.hlc
    }
    fn storage_root(&self) -> Result<PathBuf, String> {
        Ok(self.root.clone())
    }
    fn sync_transport(&self) -> Option<Arc<dyn SyncTransport>> {
        None
    }
    fn exec_registry(&self) -> Arc<RuntimeRegistry> {
        self.registry.clone()
    }
}

#[test]
fn resolves_handle_to_text_and_subtree() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().to_path_buf();
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);

    let storage = JsonlStorage::open(root.join("ops"), actor).unwrap();
    let mut ws =
        Workspace::open_with_storage(actor, Box::new(storage), Some(root.clone())).unwrap();

    // The exact shape from the issue repro: a source page with a parent
    // block carrying inline markup and a nested child.
    let src = open_or_create_page(&mut ws, &hlc, "src", "Src", PageKind::Page).unwrap();
    let parent = append_block(&mut ws, &hlc, Some(src), Some("parent **block**")).unwrap();
    append_block(&mut ws, &hlc, Some(parent), Some("child block")).unwrap();
    // A task block, so the reply's `status` (the TODO/DONE glyph the
    // client prefixes) gets exercised, not just the plain path.
    let task = append_block(&mut ws, &hlc, Some(src), Some("TODO buy milk")).unwrap();

    // The handle is deterministic from the block's NodeId — the same one
    // the sidecar records and the frontend types into `!((…))`.
    let handle = derive_ref_handle(parent);
    let task_handle = derive_ref_handle(task);

    // `WorkspaceIndex::build` reads the `.md` projection, so it must exist.
    apply_page_md_with_sidecar(&ws, &root, src).unwrap();

    let host = TestHost {
        workspace: Arc::new(Mutex::new(Some(ws))),
        hlc,
        root,
        registry: Arc::new(RuntimeRegistry::with_builtins()),
    };

    let resolved: HashMap<_, _> = block_on(resolve_embeds(
        &host,
        vec![handle.clone(), task_handle.clone()],
    ))
    .unwrap();

    let entry = resolved
        .get(&handle)
        .expect("handle should resolve to its source block");

    // Inline `((…))` ref surface: the source text, prefix-free.
    assert_eq!(entry.text, "parent **block**");
    assert!(entry.status.is_none());

    // A TODO block splits into a `status` + prefix-free `text`, so the
    // client can draw the checkbox instead of a literal "TODO ".
    let task_entry = resolved
        .get(&task_handle)
        .expect("task handle should resolve");
    assert_eq!(task_entry.status.as_deref(), Some("todo"));
    assert_eq!(task_entry.text, "buy milk");

    // Embed `!((…))` surface: the child subtree, projected WITH tokens so
    // the client renders it instead of an empty row. This is the field the
    // pre-#147 desktop lacked, so the embed showed only `↳ parent block`.
    assert_eq!(entry.children.len(), 1, "the child block must come through");
    let child = &entry.children[0];
    assert_eq!(child.text, "child block");
    assert!(
        !child.tokens.is_empty(),
        "subtree nodes must carry inline tokens"
    );

    // An unknown handle is simply absent (orphan → client shows raw chip).
    let empty = block_on(resolve_embeds(
        &host,
        vec![derive_ref_handle(NodeId::new()).clone()],
    ))
    .unwrap();
    assert!(empty.is_empty());
}

/// The reply subtree is capped at the client render depth (4). A source
/// block nested deeper than that must not ship the extra levels — the
/// client would never render them, they'd be pure IPC weight.
#[test]
fn caps_subtree_at_client_render_depth() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().to_path_buf();
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);

    let storage = JsonlStorage::open(root.join("ops"), actor).unwrap();
    let mut ws =
        Workspace::open_with_storage(actor, Box::new(storage), Some(root.clone())).unwrap();

    // A 6-deep chain under the embedded block (levels 1..6 of children).
    let src = open_or_create_page(&mut ws, &hlc, "deep", "Deep", PageKind::Page).unwrap();
    let embedded = append_block(&mut ws, &hlc, Some(src), Some("embedded")).unwrap();
    let mut parent = embedded;
    for level in 1..=6 {
        parent =
            append_block(&mut ws, &hlc, Some(parent), Some(&format!("level {level}"))).unwrap();
    }
    let handle = derive_ref_handle(embedded);
    apply_page_md_with_sidecar(&ws, &root, src).unwrap();

    let host = TestHost {
        workspace: Arc::new(Mutex::new(Some(ws))),
        hlc,
        root,
        registry: Arc::new(RuntimeRegistry::with_builtins()),
    };

    let resolved = block_on(resolve_embeds(&host, vec![handle.clone()])).unwrap();
    let entry = resolved.get(&handle).expect("handle resolves");

    // Walk the deepest chain: exactly 4 levels of children survive, the
    // level-4 node keeps its text but loses its (level-5+) descendants.
    let mut node = &entry.children;
    let mut depth = 0;
    while let Some(first) = node.first() {
        depth += 1;
        node = &first.children;
    }
    assert_eq!(depth, 4, "subtree must be capped at 4 levels of children");
}

/// `![[page]]`: the name as written comes back with the page's title and
/// its top-level blocks, and a name nothing answers to is absent.
///
/// Both halves matter, and the second is the one with a silent failure:
/// an unresolvable name must be **missing** from the map rather than
/// present-and-empty, so the client can fall back to the raw token
/// instead of drawing an embed of a page that does not exist.
#[test]
fn resolves_a_page_name_to_its_title_and_blocks() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().to_path_buf();
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);

    let storage = JsonlStorage::open(root.join("ops"), actor).unwrap();
    let mut ws =
        Workspace::open_with_storage(actor, Box::new(storage), Some(root.clone())).unwrap();

    // Title and slug deliberately differ in case, which is what makes
    // the slugify rung of the ladder load-bearing: `![[Infra]]` has no
    // literal slug, and `slugify("Infra")` is what finds the page.
    let page = open_or_create_page(&mut ws, &hlc, "infra", "Infra", PageKind::Page).unwrap();
    let parent = append_block(&mut ws, &hlc, Some(page), Some("deploy **notes**")).unwrap();
    append_block(&mut ws, &hlc, Some(parent), Some("child note")).unwrap();
    append_block(&mut ws, &hlc, Some(page), Some("second top-level")).unwrap();
    apply_page_md_with_sidecar(&ws, &root, page).unwrap();

    let host = TestHost {
        workspace: Arc::new(Mutex::new(Some(ws))),
        hlc,
        root,
        registry: Arc::new(RuntimeRegistry::with_builtins()),
    };

    let resolved: HashMap<_, _> = block_on(resolve_page_embeds(
        &host,
        vec!["Infra".into(), "infra".into(), "no such page".into()],
    ))
    .unwrap();

    // Keyed by the name **as written** — both spellings reach the same
    // page, and the renderer holds `token.value`, not the slug.
    for name in ["Infra", "infra"] {
        let entry = resolved
            .get(name)
            .unwrap_or_else(|| panic!("`{name}` should resolve to the page"));
        assert_eq!(entry.slug, "infra");
        assert_eq!(entry.title, "Infra");
        assert_eq!(entry.blocks.len(), 2, "both top-level blocks come through");
        assert_eq!(entry.blocks[0].text, "deploy **notes**");
        assert!(
            !entry.blocks[0].tokens.is_empty(),
            "blocks must carry inline tokens, like an embed subtree does"
        );
        assert_eq!(entry.blocks[0].children.len(), 1);
        assert_eq!(entry.blocks[0].children[0].text, "child note");
    }

    assert!(
        !resolved.contains_key("no such page"),
        "an unresolvable name is omitted, exactly like an orphan block handle"
    );
}

/// The page outline is capped at the same depth an embed subtree is, so
/// neither surface ships levels the client will not render.
#[test]
fn caps_a_page_embed_at_client_render_depth() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().to_path_buf();
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);

    let storage = JsonlStorage::open(root.join("ops"), actor).unwrap();
    let mut ws =
        Workspace::open_with_storage(actor, Box::new(storage), Some(root.clone())).unwrap();

    let page = open_or_create_page(&mut ws, &hlc, "deep", "Deep", PageKind::Page).unwrap();
    let mut parent = append_block(&mut ws, &hlc, Some(page), Some("level 1")).unwrap();
    for level in 2..=7 {
        parent =
            append_block(&mut ws, &hlc, Some(parent), Some(&format!("level {level}"))).unwrap();
    }
    apply_page_md_with_sidecar(&ws, &root, page).unwrap();

    let host = TestHost {
        workspace: Arc::new(Mutex::new(Some(ws))),
        hlc,
        root,
        registry: Arc::new(RuntimeRegistry::with_builtins()),
    };

    let resolved = block_on(resolve_page_embeds(&host, vec!["Deep".into()])).unwrap();
    let entry = resolved.get("Deep").expect("the page resolves");

    let mut node = &entry.blocks;
    let mut depth = 0;
    while let Some(first) = node.first() {
        depth += 1;
        node = &first.children;
    }
    assert_eq!(
        depth, 4,
        "a page embed is capped at 4 levels, like a subtree"
    );
}

/// The shape a real workspace hit first: a **namespaced** page.
///
/// `![[buser/team/people/beneficios]]` renders the unresolved literal
/// unless the ladder finds the page, and the two halves of a namespace
/// pull in opposite directions — the `/` survives only in `title::`
/// (`outl_actions::namespace`), while the file is
/// `pages/buser-team-people-beneficios.md` because a slug is a single
/// path component. So the name as written matches **neither** the slug
/// nor a literal lookup, and both of the remaining rungs have to work.
#[test]
fn resolves_a_namespaced_page_by_slug_fold_and_by_title() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().to_path_buf();
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);

    let storage = JsonlStorage::open(root.join("ops"), actor).unwrap();
    let mut ws =
        Workspace::open_with_storage(actor, Box::new(storage), Some(root.clone())).unwrap();

    let page = open_or_create_page(
        &mut ws,
        &hlc,
        "buser-team-people-beneficios",
        "buser/team/people/beneficios",
        PageKind::Page,
    )
    .unwrap();
    append_block(&mut ws, &hlc, Some(page), Some("vale refeição")).unwrap();
    apply_page_md_with_sidecar(&ws, &root, page).unwrap();

    let host = TestHost {
        workspace: Arc::new(Mutex::new(Some(ws))),
        hlc,
        root,
        registry: Arc::new(RuntimeRegistry::with_builtins()),
    };

    let resolved: HashMap<_, _> = block_on(resolve_page_embeds(
        &host,
        vec!["buser/team/people/beneficios".into()],
    ))
    .unwrap();

    let entry = resolved
        .get("buser/team/people/beneficios")
        .expect("a namespaced name must resolve");
    assert_eq!(entry.slug, "buser-team-people-beneficios");
    assert_eq!(entry.title, "buser/team/people/beneficios");
    assert_eq!(entry.blocks.len(), 1);
    assert_eq!(entry.blocks[0].text, "vale refeição");
}
