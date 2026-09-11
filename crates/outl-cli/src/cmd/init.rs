//! `outl init <path>` — scaffold a workspace.

use crate::workspace_layout::{init, read_config, today, Paths};
use anyhow::{Context, Result};
use outl_actions::{
    append_block, apply_page_md_with_sidecar, open_or_create_by_name, open_today, set_property,
    PageKind, JOURNAL_TEMPLATE_NAME, TEMPLATE_KEY,
};
use outl_core::hlc::HlcGenerator;
use outl_core::id::ActorId;
use outl_core::property::PropValue;
use outl_core::storage::{JsonlStorage, PageScope};
use outl_core::workspace::Workspace;
use std::fs;
use std::path::Path;

/// Run the `init` subcommand.
///
/// `scope = "per-page"` switches new workspaces to the Phase B layout
/// (`ops/<actor>/<slug>.jsonl`) — boot is proportional to the active
/// page, not the whole workspace. Default is `"global"` for back-compat
/// with every existing workspace.
///
/// `bare` skips [`seed_workspace`], so the workspace comes up with the
/// directory layout and a config and **no ops at all**. It exists for the
/// one case where seeding is actively wrong: a workspace created only to
/// become a replica of an existing graph, which then joins it with
/// `outl peer pair --ticket`. Pairing adopts the host's
/// [`outl_core::WorkspaceId`] but keeps the joiner's ops, so a seeded
/// replica pushes its own `templates/journal` page and its own journal
/// for today into the host's graph — two page nodes with one slug, both
/// projecting to `pages/templates/journal.md`. See `docs/self-hosting.md`.
pub fn run(path: &Path, scope: &str, bare: bool) -> Result<()> {
    let paths = Paths::at(path.to_path_buf());

    // Create all directories and seed config.
    init(&paths)?;

    let cfg = read_config(&paths)?;
    // Claims the freshly-seeded `actor_id` for this device, so the
    // machine that ran `outl init` is the one that keeps writing to
    // `ops-<that actor>.jsonl` after the workspace is replicated.
    let actor = outl_ws::actor::resolve_device_actor(
        &paths,
        &cfg,
        &outl_core::device::DeviceStore::open_default(),
    )?;
    let initial_scope = match scope {
        "per-page" => PageScope::PerPage("home".into()),
        _ => PageScope::Global,
    };

    if !bare {
        seed_workspace(&paths, actor, initial_scope)?;
    }

    println!("Initialized outl workspace at {}", paths.root.display());
    println!("  ops:      {}", paths.ops.display());
    println!("  config:   {}", paths.config.display());
    if bare {
        println!("  journal:  (none — --bare wrote no ops)");
    } else {
        println!("  journal:  {}", paths.journal_md(today()).display());
    }
    println!("  scope:    {}", scope);
    Ok(())
}

/// Seed the `templates/journal` template page and today's journal
/// through the op log, then project both to disk.
///
/// The journal template is a **page** (`template:: journal`), not a
/// `templates/journal.md` file (issue #146). A legacy file, if present,
/// migrates into the page body best-effort. Opening today then stamps
/// the template automatically via [`open_today`].
fn seed_workspace(paths: &Paths, actor: ActorId, scope: PageScope) -> Result<()> {
    let storage = JsonlStorage::open_with_scope_cap(paths.ops.clone(), actor, scope, 0)
        .with_context(|| format!("opening JSONL log at {}", paths.ops.display()))?;
    let mut ws = Workspace::open_with_storage(actor, Box::new(storage), Some(paths.root.clone()))
        .with_context(|| "materializing workspace")?;
    let hlc = HlcGenerator::new(actor);
    // `init` is idempotent, so this workspace is not always empty — a
    // re-init over an existing graph seeds through the same generator.
    // Raise it above whatever is already logged before the first op (see
    // `Workspace::seed_clock`); a no-op on the fresh case this mostly
    // runs in. Propagated, like every other failure in `init`: a
    // workspace that cannot be read is not one to scaffold into.
    ws.seed_clock(&hlc)
        .with_context(|| "seeding the clock from the op log")?;

    // Create the journal template page once (idempotent on re-init).
    if !has_journal_template(&ws) {
        let tpl = open_or_create_by_name(&mut ws, &hlc, "templates/journal", PageKind::Page)
            .with_context(|| "creating templates/journal page")?;
        set_property(
            &mut ws,
            &hlc,
            tpl,
            TEMPLATE_KEY,
            Some(PropValue::Text(JOURNAL_TEMPLATE_NAME.into())),
        )
        .with_context(|| "marking templates/journal as a template")?;

        // Migrate a legacy `templates/journal.md` body if it exists;
        // otherwise seed a single empty bullet.
        let legacy = fs::read_to_string(&paths.journal_template).unwrap_or_default();
        for line in journal_template_body(&legacy) {
            append_block(&mut ws, &hlc, Some(tpl), Some(&line))
                .with_context(|| "seeding journal template body")?;
        }
        let _ = apply_page_md_with_sidecar(&ws, &paths.root, tpl);
    }

    // Open today's journal — auto-instantiates the template into a
    // fresh daily — and project it.
    let today_id = open_today(&mut ws, &hlc).with_context(|| "opening today's journal")?;
    let _ = apply_page_md_with_sidecar(&ws, &paths.root, today_id);
    Ok(())
}

/// Whether the workspace already defines a `journal` template.
fn has_journal_template(ws: &Workspace) -> bool {
    outl_actions::list_templates(ws)
        .iter()
        .any(|t| t.name == JOURNAL_TEMPLATE_NAME)
}

/// Turn a legacy `templates/journal.md` into the block bodies for the
/// template page. Strips leading `- ` bullets; blank input yields a
/// single empty bullet so the template is never zero-block.
fn journal_template_body(legacy: &str) -> Vec<String> {
    let lines: Vec<String> = legacy
        .lines()
        .map(|l| l.trim().strip_prefix("- ").unwrap_or(l.trim()).to_string())
        .filter(|l| !l.is_empty())
        .collect();
    if lines.is_empty() {
        vec![String::new()]
    } else {
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn init_creates_full_workspace() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("notes");
        run(&root, "global", false).unwrap();

        let paths = Paths::at(&root);
        assert!(paths.dot_outl.is_dir(), ".outl/ should exist");
        assert!(paths.ops.is_dir(), "ops/ should exist");
        assert!(paths.config.is_file(), "config.toml should exist");
        assert!(paths.pages.is_dir(), "pages/ should exist");
        assert!(paths.journals.is_dir(), "journals/ should exist");
        // Today's journal seeded.
        let today_path = paths.journal_md(today());
        assert!(today_path.is_file(), "today's journal should exist");

        // The journal template is a page (`template:: journal`), not a
        // `templates/journal.md` file (issue #146).
        assert!(
            !paths.journal_template.is_file(),
            "templates/journal.md should NOT be seeded as a file"
        );
        let cfg = read_config(&paths).unwrap();
        let actor = cfg.actor().unwrap();
        let storage =
            JsonlStorage::open_with_scope_cap(paths.ops.clone(), actor, PageScope::Global, 0)
                .unwrap();
        let ws = Workspace::open_with_storage(actor, Box::new(storage), Some(paths.root.clone()))
            .unwrap();
        assert!(
            outl_actions::list_templates(&ws)
                .iter()
                .any(|t| t.name == "journal"),
            "journal template page should exist"
        );
    }

    #[test]
    fn init_is_idempotent() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("notes");
        run(&root, "global", false).unwrap();
        // Second run must not error or wipe state.
        run(&root, "global", false).unwrap();
        let paths = Paths::at(&root);
        assert!(paths.ops.is_dir());
    }

    /// `--bare` exists so a replica can join an existing graph without
    /// pushing a second `templates/journal` page into it. If seeding
    /// ever leaks back into this path, the duplicate lands in the
    /// *user's* graph, where two page nodes share one slug and one
    /// `.md` path.
    #[test]
    fn bare_init_writes_no_ops() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("notes");
        run(&root, "global", true).unwrap();

        let paths = Paths::at(&root);
        assert!(paths.ops.is_dir(), "layout is still created");
        assert!(paths.config.is_file(), "config is still written");
        assert!(
            !paths.journal_md(today()).exists(),
            "--bare must not project today's journal"
        );

        let cfg = read_config(&paths).unwrap();
        let actor = cfg.actor().unwrap();
        let storage =
            JsonlStorage::open_with_scope_cap(paths.ops.clone(), actor, PageScope::Global, 0)
                .unwrap();
        let ws = Workspace::open_with_storage(actor, Box::new(storage), Some(paths.root.clone()))
            .unwrap();
        assert!(
            outl_actions::list_templates(&ws).is_empty(),
            "--bare must not seed the journal template page"
        );
    }

    #[test]
    fn init_with_per_page_scope_creates_actor_subdir() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("notes");
        run(&root, "per-page", false).unwrap();

        let paths = Paths::at(&root);
        // `ops/` exists; the per-actor subdir gets created on first
        // `JsonlStorage::open_with_scope_cap` call (lazy).
        assert!(paths.ops.is_dir());
    }
}
