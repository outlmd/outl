//! Regression tests for the projection-ahead-of-log recovery
//! (`outl_actions::desync`).
//!
//! `tests/fixtures/2026-07-03.{md,outl}` are the **real** `.md` +
//! sidecar pair from the incident device: the mobile app edited the
//! daily during an offline flight, wrote the projection (23:57), but
//! the ops append never happened. The sidecar's `last_synced_hash`
//! matches the `.md` (they were written together), so the hash-based
//! orphan scan considers the page "in sync" forever while the block
//! ids exist in no op log.

use std::fs;
use std::path::Path;
use std::str::FromStr;

use chrono::NaiveDate;
use outl_actions::{
    append_block, apply_page_md_with_sidecar, delete, open_journal, recover_desynced_projection,
    scan_for_desynced_projections, SyncEngine,
};
use outl_core::hlc::HlcGenerator;
use outl_core::id::{ActorId, NodeId};
use outl_core::storage::JsonlStorage;
use outl_core::workspace::Workspace;
use tempfile::TempDir;

const FIXTURE_MD: &str = include_str!("fixtures/2026-07-03.md");
const FIXTURE_OUTL: &str = include_str!("fixtures/2026-07-03.outl");

/// The six block ids the device's sidecar carries — none of them ever
/// reached an op log.
const FIXTURE_BLOCK_IDS: [&str; 6] = [
    "01KWM0ABBSPJPCJGHSNE4P12BY",
    "01KWM0AJNCXEJERRWT2N1ZPNA5",
    "01KWM0BNQEE18HXYJ4S0M5EM1T",
    "01KWM0W8GV6W1RK46ZSG2F1VKF",
    "01KWM0X1B0BJ4ZS5088ZA9BJVZ",
    "01KWM1EAR2ZJ23R521EP5ECYYX",
];

fn node_id(s: &str) -> NodeId {
    NodeId(ulid::Ulid::from_str(s).expect("valid ulid"))
}

fn open_ws(root: &Path, actor: ActorId) -> Workspace {
    fs::create_dir_all(root.join("ops")).unwrap();
    fs::create_dir_all(root.join("journals")).unwrap();
    fs::create_dir_all(root.join("pages")).unwrap();
    let storage = JsonlStorage::open(root.join("ops"), actor).unwrap();
    Workspace::open_with_storage(actor, Box::new(storage), Some(root.to_path_buf())).unwrap()
}

#[test]
fn recovers_flight_blocks_from_real_device_fixture() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = open_ws(root, actor);

    // The journal page itself has ops (it existed before the flight),
    // plus a block that arrived through normal sync — exactly the
    // incident's shape.
    let date = NaiveDate::from_ymd_opt(2026, 7, 3).unwrap();
    let page_id = open_journal(&mut ws, &hlc, date).unwrap();
    assert_eq!(
        page_id.to_string(),
        "3NJDQ4MMH20P0G6TA9SAV118EM",
        "deterministic journal id must match the device sidecar's page_id"
    );
    let briefing = append_block(&mut ws, &hlc, Some(page_id), Some("briefing de produto")).unwrap();

    // Drop the device's projection pair on disk verbatim.
    let md_path = root.join("journals/2026-07-03.md");
    fs::write(&md_path, FIXTURE_MD).unwrap();
    fs::write(root.join("journals/2026-07-03.outl"), FIXTURE_OUTL).unwrap();

    // The hash gate is blind to this state (that's the bug this
    // module fixes); the workspace-aware scan is not.
    //
    // The fixture's `pipeline_version` is kept at `CURRENT_PIPELINE_VERSION`
    // on purpose. It is a real device capture in every other field, but a
    // stale pipeline version would make `scan_for_orphans` flag the page
    // for the *migration* reason and mask what this test is actually
    // asserting: that the hash comparison alone cannot see a projection
    // running ahead of the log. Bump it here whenever the pipeline
    // version bumps.
    let engine = SyncEngine::new(root.to_path_buf(), actor);
    assert!(
        engine.scan_for_orphans().is_empty(),
        "hash-based scan must consider the page in sync"
    );
    let flagged = engine.scan_for_desynced_projections(&ws);
    assert_eq!(flagged, vec![md_path.clone()]);

    let applied = recover_desynced_projection(&mut ws, &hlc, root, &md_path).unwrap();
    assert!(applied > 0, "recovery must emit ops");

    // Every sidecar id now exists in the tree, id preserved.
    for id in FIXTURE_BLOCK_IDS {
        assert!(
            ws.tree().parent(node_id(id)).is_some(),
            "sidecar id {id} must be materialised"
        );
    }
    // Structure follows the `.md`: indents 0/1/1/2/1/0.
    let voo = node_id(FIXTURE_BLOCK_IDS[0]);
    let airmode = node_id(FIXTURE_BLOCK_IDS[1]);
    let sem_internet = node_id(FIXTURE_BLOCK_IDS[2]);
    let gocache = node_id(FIXTURE_BLOCK_IDS[3]);
    assert_eq!(ws.tree().parent(voo), Some(page_id));
    assert_eq!(ws.tree().parent(airmode), Some(voo));
    assert_eq!(ws.tree().parent(sem_internet), Some(voo));
    assert_eq!(ws.tree().parent(gocache), Some(sem_internet));
    // Text comes from the `.md` (parser trims trailing whitespace).
    assert_eq!(ws.block_text(voo).as_deref(), Some("voo de volta pra sp"));
    assert_eq!(
        ws.block_text(airmode).as_deref(),
        Some("vendo como sync se comporta offline airmode")
    );
    assert_eq!(
        ws.block_text(gocache).as_deref(),
        Some("#gocache perdi a reuniao de alinhamento de prodito")
    );

    // The merge is additive: the synced block is untouched.
    assert_eq!(ws.tree().parent(briefing), Some(page_id));

    // The page was re-projected: `.md` + sidecar now show the merged
    // state (log blocks AND recovered blocks).
    let md_after = fs::read_to_string(&md_path).unwrap();
    assert!(md_after.contains("briefing de produto"), "{md_after}");
    assert!(md_after.contains("voo de volta pra sp"), "{md_after}");
    let sc = outl_md::sidecar::read(&root.join("journals/2026-07-03.outl")).unwrap();
    assert_eq!(sc.last_synced_hash, outl_md::sidecar::file_hash(&md_after));
    let sc_ids: Vec<NodeId> = sc.blocks.iter().map(|b| b.id).collect();
    assert!(sc_ids.contains(&briefing));
    assert!(sc_ids.contains(&voo));

    // The recovered ops are **persisted** — a peer replaying this
    // jsonl materialises the flight blocks.
    let ops_file = root.join("ops").join(format!("ops-{actor}.jsonl"));
    let ops_text = fs::read_to_string(&ops_file).unwrap();
    for id in FIXTURE_BLOCK_IDS {
        assert!(ops_text.contains(id), "op log must contain {id}");
    }

    // Converged: nothing left to detect, second recovery is a no-op.
    assert!(engine.scan_for_desynced_projections(&ws).is_empty());
    assert_eq!(
        recover_desynced_projection(&mut ws, &hlc, root, &md_path).unwrap(),
        0
    );
}

#[test]
fn legit_remote_delete_is_not_resurrected_while_lost_blocks_are_added() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = open_ws(root, actor);

    let date = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let page_id = open_journal(&mut ws, &hlc, date).unwrap();
    let alpha = append_block(&mut ws, &hlc, Some(page_id), Some("alpha")).unwrap();
    let beta = append_block(&mut ws, &hlc, Some(page_id), Some("beta")).unwrap();
    apply_page_md_with_sidecar(&ws, root, page_id).unwrap();

    // Simulate the phone: "gamma" was added offline, the projection
    // (md + sidecar, consistent hashes) reached disk, the ops didn't.
    let md_path = root.join("journals/2026-01-01.md");
    let sc_path = root.join("journals/2026-01-01.outl");
    let md_stale = format!("{}- gamma\n", fs::read_to_string(&md_path).unwrap());
    let ghost = NodeId(ulid::Ulid::generate());
    let mut sc = outl_md::sidecar::read(&sc_path).unwrap();
    sc.blocks.push(outl_md::sidecar::SidecarBlock::from_text(
        ghost,
        sc.blocks.len() + 1,
        0,
        "gamma",
    ));
    sc.last_synced_hash = outl_md::sidecar::file_hash(&md_stale);
    fs::write(&md_path, &md_stale).unwrap();
    outl_md::sidecar::write(&sc_path, &sc).unwrap();

    // Meanwhile the log records a real delete of beta (`Move` to
    // TRASH_ROOT) — the stale `.md` still shows it.
    delete(&mut ws, &hlc, beta).unwrap();

    let flagged = scan_for_desynced_projections(&ws, root);
    assert_eq!(flagged, vec![md_path.clone()]);
    let applied = recover_desynced_projection(&mut ws, &hlc, root, &md_path).unwrap();
    assert!(applied > 0);

    // gamma (ops lost) is recreated with its sidecar id; beta (delete
    // IS an op — the log wins) stays trashed; alpha is untouched.
    assert_eq!(ws.tree().parent(ghost), Some(page_id));
    assert_eq!(ws.block_text(ghost).as_deref(), Some("gamma"));
    assert_eq!(ws.tree().parent(beta), Some(NodeId::trash()));
    assert_eq!(ws.tree().parent(alpha), Some(page_id));
    // md order preserved relative to existing siblings: gamma after alpha.
    let alpha_pos = ws.tree().position(alpha).cloned().unwrap();
    let gamma_pos = ws.tree().position(ghost).cloned().unwrap();
    assert!(alpha_pos < gamma_pos);

    // Re-projected `.md`: alpha + gamma, beta gone (log wins).
    let md_after = fs::read_to_string(&md_path).unwrap();
    assert!(md_after.contains("alpha"), "{md_after}");
    assert!(md_after.contains("gamma"), "{md_after}");
    assert!(!md_after.contains("beta"), "{md_after}");
}

#[test]
fn stale_hash_pages_stay_with_the_orphan_scan() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = open_ws(root, actor);

    let date = NaiveDate::from_ymd_opt(2026, 2, 2).unwrap();
    let page_id = open_journal(&mut ws, &hlc, date).unwrap();
    append_block(&mut ws, &hlc, Some(page_id), Some("hello")).unwrap();
    apply_page_md_with_sidecar(&ws, root, page_id).unwrap();

    // External edit: `.md` changed, sidecar hash now stale. That page
    // belongs to the normal reconcile path, not to desync recovery —
    // even though it also references no missing id yet.
    let md_path = root.join("journals/2026-02-02.md");
    let edited = format!("{}- typed in vim\n", fs::read_to_string(&md_path).unwrap());
    fs::write(&md_path, &edited).unwrap();

    let engine = SyncEngine::new(root.to_path_buf(), actor);
    assert_eq!(engine.scan_for_orphans(), vec![md_path.clone()]);
    assert!(engine.scan_for_desynced_projections(&ws).is_empty());
    assert_eq!(
        recover_desynced_projection(&mut ws, &hlc, root, &md_path).unwrap(),
        0,
        "stale-hash pages must be left to reconcile_md"
    );
}

#[test]
fn fully_synced_pages_are_not_flagged() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = open_ws(root, actor);

    let date = NaiveDate::from_ymd_opt(2026, 3, 3).unwrap();
    let page_id = open_journal(&mut ws, &hlc, date).unwrap();
    append_block(&mut ws, &hlc, Some(page_id), Some("all good")).unwrap();
    apply_page_md_with_sidecar(&ws, root, page_id).unwrap();

    assert!(scan_for_desynced_projections(&ws, root).is_empty());
}

/// The typical offline session is not "one lost `Create`" — it is a new
/// block **and** a reworded existing one, and only the first half has an
/// id the tree has never seen.
///
/// The recovery is strictly additive by design, so it re-creates the new
/// block and leaves the existing one alone: the log's text may be a peer
/// edit that landed while the projection was frozen, and writing the disk
/// text back as an `Op::Edit` would revert that peer permanently on an
/// append-only log.
///
/// But the re-projection that followed then rendered the tree over the
/// file, deleting the reworded text — which exists in no op, so nowhere
/// else. Same defect as RFC 0210, reached through a different door: a
/// write authorised by `last_synced_hash` agreeing with the bytes on
/// disk, which only ever proved outl wrote them.
///
/// Keep the recovered ops, leave the file.
#[test]
fn recovery_does_not_reproject_over_text_the_log_never_saw() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = open_ws(root, actor);

    let date = NaiveDate::from_ymd_opt(2026, 4, 4).unwrap();
    let page_id = open_journal(&mut ws, &hlc, date).unwrap();
    let alpha = append_block(&mut ws, &hlc, Some(page_id), Some("alpha")).unwrap();
    apply_page_md_with_sidecar(&ws, root, page_id).unwrap();

    // The offline session: alpha reworded, gamma created. The projection
    // pair reached disk together (hashes agree); the ops append did not.
    let md_path = root.join("journals/2026-04-04.md");
    let sc_path = root.join("journals/2026-04-04.outl");
    let md_offline = "- alpha reworded offline\n- gamma\n";
    let ghost = NodeId(ulid::Ulid::generate());
    let mut sc = outl_md::sidecar::read(&sc_path).unwrap();
    sc.blocks = vec![
        outl_md::sidecar::SidecarBlock::from_text(alpha, 1, 0, "alpha reworded offline"),
        outl_md::sidecar::SidecarBlock::from_text(ghost, 2, 0, "gamma"),
    ];
    sc.last_synced_hash = outl_md::sidecar::file_hash(md_offline);
    fs::write(&md_path, md_offline).unwrap();
    outl_md::sidecar::write(&sc_path, &sc).unwrap();

    assert_eq!(
        scan_for_desynced_projections(&ws, root),
        vec![md_path.clone()]
    );
    let applied = recover_desynced_projection(&mut ws, &hlc, root, &md_path).unwrap();

    // The additive half still runs: gamma's ops reach the log.
    assert!(applied > 0, "the lost Create must still be recovered");
    assert_eq!(ws.tree().parent(ghost), Some(page_id));
    assert_eq!(ws.block_text(ghost).as_deref(), Some("gamma"));
    // alpha keeps the log's text — the recovery never overrides the log.
    assert_eq!(ws.block_text(alpha).as_deref(), Some("alpha"));
    // And the file keeps the text that exists in no op.
    assert_eq!(
        fs::read_to_string(&md_path).unwrap(),
        md_offline,
        "re-projecting here deletes content the op log has never seen"
    );
}
