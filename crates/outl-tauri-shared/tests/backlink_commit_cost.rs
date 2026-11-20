//! What one cross-page commit is allowed to cost, and how many times it
//! may walk the page it is writing to.
//!
//! The third of the three tests root `CLAUDE.md` invariant 15 asks of a
//! performance fix: *is it cheap once* — **read the call sites, because a
//! type cannot bound how many times a caller asks.** The other two are
//! `outl-actions/tests/outline_property_pass.rs`, which covers the read
//! that was made cheap; this covers how often the commit asks for it.
//!
//! It matters here because the asking is spread across three files in two
//! crates, and no signature says so. `edit_backlink_block` runs the
//! stale-`.md` pre-check, `outl_actions::commit_page` renders the page
//! twice for the undo snapshot, `TauriCommitHooks::project` renders and
//! writes it a third time because `BacklinkRefresh::PatchPage` reads the
//! `.md` back, and `finish_in_page_with_refresh` then projects it once
//! more for the reply. Each is defensible where it is written; the total
//! is only visible from here.
//!
//! Measured on the reference workspace (175,096 nodes, 193k properties,
//! 5,257 pages), editing a 688-block page through a backlink row:
//! **2,650 ms** before, **110 ms** after. The whole of that difference
//! was the reply's projection — `docs/performance.md` carries the table.
//!
//! # Why eight of the nine gestures keep `DropAll`
//!
//! A reviewer read the split as an oversight: `edit_backlink_block` takes
//! [`BacklinkRefresh::PatchPage`] and `backlink_row_op` /
//! `create_backlink_sibling` take `DropAll`, so eight of the nine
//! non-text gestures drop the whole index and the next `page_backlinks`
//! reads and parses every `.md` in the workspace. It was declined
//! untested. Both positions were measured, and `POLICY` below is what
//! keeps the answer.
//!
//! The fixture, so the numbers are re-derivable (the harness is
//! deliberately not in the tree — see `docs/performance.md` → *How to
//! measure*): 3,382 pages built with `Workspace::open_in_memory`,
//! 161,669 nodes, 79,144 properties, a 688-block page and a 20-block
//! journal day each carrying one `[[busiest]]` citing block, every page
//! projected, the index warm. Each row is a median of 7.
//!
//! | per gesture | commit, on the IPC thread | next `page_backlinks`, off it |
//! |---|---|---|
//! | `DropAll`, 688-block page | **4.1 ms** | 280 ms |
//! | `PatchPage`, 688-block page | **29.3 ms** | 5.3 ms |
//! | `DropAll`, 20-block journal day | **2.3 ms** | 277 ms |
//! | `PatchPage`, 20-block journal day | **23.5 ms** | 5.4 ms |
//!
//! **`PatchPage`'s cost is one synchronous `.md` + sidecar fsync pair,
//! and it is flat in page size** — 24.4 ms of the 29.3 on the 688-block
//! page, 21.2 of the 23.5 on the day; the backlink patch itself is
//! 0.7 ms. The same figure falls out from the other direction: an
//! ordinary `edit_block` costs 1.1 ms on a journal day with a
//! `ProjectionWriter` wired and **24.2 ms** with `projection_writer()`
//! returning `None`. So the trade is +21 to +25 ms on the thread that
//! accepts keystrokes, against −275 ms of background CPU that nothing
//! awaits and that `lateSectionVisible` keeps rows across.
//!
//! The user feels the IPC thread, so `DropAll` stays. That is also the
//! project-wide default this would be a local exception to — async
//! writes exist precisely so no client blocks on a write — and
//! `edit_backlink_block` is the one gesture where the exception was
//! bought with a measurement of its own.
//!
//! Two things that did **not** turn out to matter, recorded so the next
//! reader does not re-measure them:
//!
//! - **`PatchPage` is not *wrong* for the structural gestures.** The
//!   index is keyed by **source page** (`BacklinkIndex::by_page`, drop +
//!   re-walk in `reindex_page_from_disk`), and every one of the nine is
//!   confined to `page_id`: `indent` lands on a previous sibling,
//!   `outdent` refuses at a page root, both moves swap siblings, and
//!   `delete` is a move to `NodeId::trash()`, which is not a page and
//!   has no `.md`. A re-walk recomputes `source_block_path` and the
//!   crumbs the moves shift. Equivalence against a full rebuild held
//!   after `toggle_todo`, `move_down` and `indent`. The choice is a cost
//!   question, not a correctness one.
//! - **A stale sidecar does not make the commit expensive.** With a
//!   third of the measured page's own blocks trashed and the sidecar
//!   left naming them, `edit_block` was 1.00 ms against 1.08 ms on a
//!   healthy page. `sidecar_can_vouch_for` is cheap.
//!
//! Where `DropAll` does cost something is a **held** key: `Alt+↑` /
//! `Alt+↓` / `Tab` repeat, the frontend fires one un-deduped
//! `pageBacklinks` per repeat, and ten of those cost the IPC thread
//! 30.0 ms per keystroke in workspace-lock contention — against
//! 29.9 ms for `PatchPage`'s own fsync. A wash for latency, ten whole
//! rebuilds of CPU apart. That is an argument for deduping the refetch,
//! not for moving the policy: switching would pay 25 ms on every single
//! gesture to fix the burst case.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use outl_actions::{
    append_block, apply_page_md_with_sidecar, apply_page_md_with_sidecar_if_stale,
    build_backlink_index_from_disk, list_pages, open_or_create_page, set_property, BacklinkIndex,
    PageKind, SyncTransport,
};
use outl_core::hlc::HlcGenerator;
use outl_core::id::{ActorId, NodeId};
use outl_core::property::PropValue;
use outl_core::workspace::Workspace;
use outl_exec::RuntimeRegistry;
use outl_tauri_shared::commands::backlink_edit::edit_backlink_block;
use outl_tauri_shared::host::AppHost;
use outl_tauri_shared::ProjectionWriter;
use parking_lot::Mutex;
use tempfile::TempDir;

// ---------------------------------------------------------------------
// The census.
// ---------------------------------------------------------------------

/// One call on the cross-page commit path that walks the page: the file
/// it lives in, the call as written, how many times it appears, and why
/// that number is the number.
struct Walk {
    file: &'static str,
    call: &'static str,
    times: usize,
    why: &'static str,
}

/// Every page walk a cross-page commit can make, declared.
///
/// `times` is the count in the **source**, not per commit: the two
/// projection calls and the two view builders are the arms of a branch,
/// so a commit runs one of each pair. The point of counting the source is
/// that adding an arm is the thing that has to be noticed.
const CENSUS: &[Walk] = &[
    Walk {
        file: "crates/outl-tauri-shared/src/commands/backlink_edit.rs",
        call: "reproject_stale_md(",
        times: 2,
        why: "the ahead-of-log refusal, which has to come before the \
              mutation — a post-commit refusal names an edit that is \
              already durable and will never reach the file. Two call \
              sites because the question is per gesture, not per \
              command: `edit_backlink_block` (the draft is seeded from \
              the `.md`, so the user typed over bytes the log does not \
              hold) and `backlink_row_op` **only** for `ToggleTodo` / \
              `MarkDone`, which read the logged text, move a marker and \
              write it back. The other five row ops name a node, read no \
              text, and do not ask — so this walk is paid by two of the \
              eight gestures routed through that command, never by all \
              of them. Asking the cheaper `sidecar_can_vouch_for` here \
              instead would be a second assembly of a verdict that also \
              has to pair it with \"is anything on disk\" and with the \
              frontmatter channel, which is the pairing issue #332 froze \
              233 pages for getting wrong",
    },
    Walk {
        file: "crates/outl-actions/src/commit.rs",
        call: "render_page_md(workspace, page)",
        times: 2,
        why: "the undo snapshot and the comparison that decides whether to \
              keep it, so `Cmd+Z` is not a visible nothing. Both are \
              skipped outright on a host with no undo stacks \
              (`CommitHooks::records_undo`)",
    },
    Walk {
        file: "crates/outl-tauri-shared/src/commit.rs",
        call: "apply_page_md_with_sidecar_guarded(",
        times: 2,
        why: "the `PatchPage` arm (inline, because the backlink patch reads \
              the `.md` it writes) and the no-writer fallback. One runs per \
              commit",
    },
    Walk {
        file: "crates/outl-tauri-shared/src/commit.rs",
        call: "build_page_view_from_tree(ws, page_id)",
        times: 1,
        why: "the reply, on a writer-wired host — which is the only shape \
              either GUI client ships. This is the walk that cost 2,588 ms \
              of the 2,650, and the fix was the read it makes, not this \
              call",
    },
    Walk {
        file: "crates/outl-tauri-shared/src/commit.rs",
        call: "build_page_view(ws, &root, page_id)",
        times: 1,
        why: "the reply, on a host with no projection writer. Reads the \
              `.md` instead, which is correct there precisely because the \
              projection was taken inline",
    },
];

/// Which [`BacklinkRefresh`] each cross-page command takes, and how many
/// times that variant may appear in `commands/backlink_edit.rs`.
///
/// The counts are the whole point: `PatchPage` once and `DropAll` twice
/// is the measured split in this file's header, and the way it gets
/// undone is somebody "unifying" the three commands onto one policy —
/// which is cheap to type, compiles, passes every behavioural test in
/// `backlink_row_ops.rs` and `backlink_edit_strict.rs`, and costs the
/// user 21-25 ms of synchronous fsync on eight more gestures.
const POLICY: &[(&str, usize, &str)] = &[
    (
        "PatchPage",
        1,
        "`edit_backlink_block` alone. It is the daily `#fup` text edit, it \
         is the one gesture whose inline projection was paid for with a \
         measurement, and it is not hold-to-repeat",
    ),
    (
        "DropAll",
        2,
        "`backlink_row_op` (the seven chords) and `create_backlink_sibling` \
         (`o` / `O`). Measured at 4.1 ms on the IPC thread against \
         `PatchPage`'s 29.3 ms, the difference being one synchronous `.md` \
         + sidecar fsync pair. The index rebuild it defers is 280 ms on a \
         background thread that nothing awaits",
    ),
];

/// Count occurrences of `ident` as a whole identifier in `src`'s **code**,
/// with comments stripped.
///
/// Three things a weaker net gets wrong, and the first two were found by
/// running this one against the file it guards:
///
/// - **Comments have to go first.** `backlink_edit.rs` names both
///   variants in its doc comments — that is the point of those comments —
///   so counting the raw file reports `PatchPage` three times and the
///   assertion would fail on a clean tree.
/// - **Identifier-exact, not a substring**, or a `DropAllTheThings` that
///   means nothing here would count.
/// - **Not a line or qualified-path match.** `rustfmt` is free to break
///   `BacklinkRefresh::PatchPage` across lines, so a net matching the
///   qualified spelling can silently find zero — the way this branch's
///   first regression ratchet did.
///
/// Stripping at the first `//` would truncate a code line whose string
/// literal contained one. That direction is the safe one: the count comes
/// out too low and the assertion fails loudly, rather than passing on a
/// policy nobody re-measured.
fn identifier_count(src: &str, ident: &str) -> usize {
    without_comments(src)
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|tok| *tok == ident)
        .count()
}

/// `src` with line comments (`//`, `///`, `//!`) and block comments cut out.
///
/// Both censuses in this file count against it. A doc comment that quotes
/// `render_page_md(workspace, page)` is prose, and counting the raw file
/// let it stand in for a call that had been deleted. Cutting at the first
/// `//` of a line that holds one inside a string literal truncates code,
/// which only ever lowers a count and so fails loudly.
fn without_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut chars = src.chars().peekable();
    while let Some(c) = chars.next() {
        match (c, chars.peek()) {
            ('/', Some('/')) => {
                for n in chars.by_ref() {
                    if n == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            ('/', Some('*')) => {
                chars.next();
                let mut prev = ' ';
                for n in chars.by_ref() {
                    if prev == '*' && n == '/' {
                        break;
                    }
                    prev = n;
                }
            }
            _ => out.push(c),
        }
    }
    out
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the workspace root is two levels above this crate")
}

#[test]
fn a_cross_page_commit_walks_the_page_the_declared_number_of_times() {
    let root = workspace_root();
    for walk in CENSUS {
        let path = root.join(walk.file);
        let src = std::fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!(
                "{} could not be read ({e}) — a census that scans nothing \
                 passes by default, which is worse than no census",
                walk.file
            )
        });
        let found = without_comments(&src).matches(walk.call).count();
        assert_eq!(
            found, walk.times,
            "{} now contains `{}` {found} time(s), not {}.\n\n\
             This path walks the page it is committing to, and the total is \
             only visible from this test — three files in two crates each \
             make a call that is defensible on its own. Before changing the \
             number, re-measure on a real graph (the procedure and the \
             before/after table are in `docs/performance.md`) and update \
             both.\n\n\
             The declared reason for this one: {}",
            walk.file, walk.call, walk.times, walk.why
        );
    }
}

/// The refresh policy each cross-page command takes is the measured one.
///
/// A behavioural test cannot hold this: both policies leave the same
/// `.md`, the same op log and the same backlink rows, so every assertion
/// in `backlink_row_ops.rs` and `backlink_edit_strict.rs` passes under
/// either. The only difference is *which thread pays*, and the only place
/// that is visible is the source.
#[test]
fn each_cross_page_command_takes_the_refresh_policy_it_was_measured_on() {
    let path = workspace_root().join("crates/outl-tauri-shared/src/commands/backlink_edit.rs");
    let src = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "commands/backlink_edit.rs could not be read ({e}) — a census \
             that scans nothing passes by default"
        )
    });
    for (variant, times, why) in POLICY {
        let found = identifier_count(&src, variant);
        assert_eq!(
            found, *times,
            "`commands/backlink_edit.rs` now names `{variant}` {found} \
             time(s), not {times}.\n\n\
             This file's header carries the measurement that chose the \
             split: `PatchPage` forces a synchronous `.md` + sidecar fsync \
             pair on the IPC thread (+21 to +25 ms per gesture, flat in \
             page size) to save a 280 ms index rebuild on a background \
             thread nothing awaits. Moving a gesture across is a \
             user-visible trade with no behavioural test that can fail, so \
             re-measure on a real graph first (`docs/performance.md` → How \
             to measure) and update the header table and this count \
             together.\n\n\
             The declared reason for this one: {why}"
        );
    }
}

// ---------------------------------------------------------------------
// The cheap branch, asserted rather than assumed.
// ---------------------------------------------------------------------

struct TestHost {
    workspace: Arc<Mutex<Option<Workspace>>>,
    hlc: HlcGenerator,
    root: PathBuf,
    registry: Arc<RuntimeRegistry>,
    backlinks: Arc<Mutex<Option<BacklinkIndex>>>,
    writer: ProjectionWriter,
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
    fn backlink_index(&self) -> Option<Arc<Mutex<Option<BacklinkIndex>>>> {
        Some(self.backlinks.clone())
    }
    fn projection_writer(&self) -> Option<&ProjectionWriter> {
        Some(&self.writer)
    }
}

/// `notes` cites `[[target]]`, both pages projected and in sync, the
/// backlink index warm — the state a user is in when they press Esc on a
/// backlink row.
///
/// The citing block carries a property so the reply exercises the grouped
/// property pass rather than the empty case.
fn host_with_a_warm_backlink() -> (TempDir, TestHost, NodeId, NodeId) {
    let tmp = TempDir::new().expect("tmp");
    let root = tmp.path().to_path_buf();
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = Workspace::open_in_memory(actor).expect("in-memory workspace");

    let target = open_or_create_page(&mut ws, &hlc, "target", "Target", PageKind::Page)
        .expect("the read page");
    append_block(&mut ws, &hlc, Some(target), Some("the page being read")).expect("a block");
    let source =
        open_or_create_page(&mut ws, &hlc, "notes", "Notes", PageKind::Page).expect("the source");
    let citing = append_block(&mut ws, &hlc, Some(source), Some("ask [[target]] about it"))
        .expect("the citing block");
    set_property(
        &mut ws,
        &hlc,
        citing,
        "owner",
        Some(PropValue::Text("avelino".into())),
    )
    .expect("a property on the citing block");
    apply_page_md_with_sidecar(&ws, &root, target).expect("target projected");
    apply_page_md_with_sidecar(&ws, &root, source).expect("source projected");

    let metas = list_pages(&ws);
    let index = build_backlink_index_from_disk(&metas, &root);

    let workspace = Arc::new(Mutex::new(Some(ws)));
    let writer = ProjectionWriter::spawn(workspace.clone(), root.clone(), |_| {});
    let host = TestHost {
        workspace,
        hlc,
        root,
        registry: Arc::new(RuntimeRegistry::new()),
        backlinks: Arc::new(Mutex::new(Some(index))),
        writer,
    };
    (tmp, host, source, citing)
}

/// The pre-check's cheap branch is the common one, and it has to stay
/// free of a write.
///
/// `reproject_stale_md` is `apply_page_md_with_sidecar_if_stale`, which
/// *can* write — and on the page a user is editing it almost never
/// should, because that page was projected by the previous commit. A
/// version of this check that re-projected unconditionally would put a
/// second 916 KB render + two fsyncs in front of every keystroke on the
/// hottest cross-page path, which is exactly the shape invariant 15's
/// fourth question asks about: *does the cheap path stay cheap on the
/// branch that does something?*
///
/// So this asserts the branch directly rather than timing it: `Ok(None)`
/// is the gate saying it left disk alone, and `None` is
/// `reproject_stale_md` agreeing.
#[test]
fn the_pre_check_writes_nothing_on_a_page_that_is_already_in_sync() {
    let (_tmp, host, source, citing) = host_with_a_warm_backlink();

    {
        let slot = host.workspace_arc();
        let guard = slot.lock();
        let ws = guard.as_ref().expect("the workspace is open");
        assert_eq!(
            apply_page_md_with_sidecar_if_stale(ws, &host.root, source).expect("the gate answers"),
            None,
            "the pre-check re-projected a page that was already in sync"
        );
        assert_eq!(
            outl_actions::reproject_stale_md(ws, &host.root, source),
            None,
            "a healthy page must come back with no refusal and no write"
        );
    }

    // And it is still true *after* a commit, which is the case that
    // matters: `PatchPage` projects inline, so the next keystroke's
    // pre-check finds the page in sync. If it did not, every keystroke
    // after the first would pay the write twice.
    edit_backlink_block(
        &host,
        source.to_string(),
        citing.to_string(),
        "ask [[target]] about it today".into(),
    )
    .expect("the cross-page edit lands");

    let slot = host.workspace_arc();
    let guard = slot.lock();
    let ws = guard.as_ref().expect("the workspace is open");
    assert_eq!(
        apply_page_md_with_sidecar_if_stale(ws, &host.root, source).expect("the gate answers"),
        None,
        "the commit left its own page stale, so the next keystroke \
         re-projects it before mutating — the write, paid twice"
    );
}

/// The reply is built once, from the tree, and says the same thing the
/// `.md` the commit just wrote says.
///
/// `PatchPage` is the one arm that projects inline on a writer-wired
/// host, so this is the one place the two readings are both available and
/// must agree. Pinning it is what stops the reply being "fixed" by
/// reading disk a second time: the answers are identical, so the extra
/// read would buy nothing and cost a parse.
#[test]
fn the_cross_page_reply_says_what_the_md_it_just_wrote_says() {
    let (_tmp, host, source, citing) = host_with_a_warm_backlink();

    let view = edit_backlink_block(
        &host,
        source.to_string(),
        citing.to_string(),
        "ask [[target]] about the migration".into(),
    )
    .expect("the cross-page edit lands");

    let slot = host.workspace_arc();
    let guard = slot.lock();
    let ws = guard.as_ref().expect("the workspace is open");
    let from_disk = outl_tauri_shared::helpers::build_page_view(ws, &host.root, source)
        .expect("the page reads back off disk");

    let texts = |v: &outl_tauri_shared::state::PageView| -> Vec<String> {
        v.outline.iter().map(|n| n.text.clone()).collect()
    };
    assert_eq!(texts(&view), texts(&from_disk));
    assert_eq!(
        view.outline
            .iter()
            .map(|n| n.properties.clone())
            .collect::<Vec<_>>(),
        from_disk
            .outline
            .iter()
            .map(|n| n.properties.clone())
            .collect::<Vec<_>>(),
        "the tree-built reply and the `.md` disagree about block properties \
         — the grouped property pass and the parser are reading different \
         things"
    );
    assert!(
        texts(&view)
            .iter()
            .any(|t| t.contains("about the migration")),
        "the edit is not in the reply: {:?}",
        texts(&view)
    );
}
