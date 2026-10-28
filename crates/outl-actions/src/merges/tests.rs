use super::*;
use crate::{append_block, edit_text, open_or_create_page as open_or_create, PageKind};
use outl_core::hlc::HlcGenerator;

fn replay_into(ws: &Workspace, into: &mut Workspace) {
    let have: HashSet<Hlc> = into.log().iter().map(|op| op.ts).collect();
    let ops: Vec<_> = ws
        .log()
        .iter()
        .filter(|op| !have.contains(&op.ts))
        .cloned()
        .collect();
    for op in ops {
        into.apply(op).unwrap();
    }
}

/// Two devices share a block; each edits it before seeing the other.
/// The peer's edit is stamped earlier, then arrives here.
fn race() -> (Workspace, Workspace, ActorId, NodeId) {
    let me = ActorId::new();
    let peer = ActorId::new();
    let peer_hlc = HlcGenerator::new(peer);
    let my_hlc = HlcGenerator::new(me);

    let mut theirs = Workspace::open_in_memory(peer).unwrap();
    let page = open_or_create(&mut theirs, &peer_hlc, "alpha", "Alpha", PageKind::Page).unwrap();
    let block = append_block(&mut theirs, &peer_hlc, Some(page), Some("milk")).unwrap();

    let mut mine = Workspace::open_in_memory(me).unwrap();
    replay_into(&theirs, &mut mine);

    edit_text(&mut theirs, &peer_hlc, block, "milk and eggs").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(3));
    edit_text(&mut mine, &my_hlc, block, "oat milk").unwrap();

    let old = {
        let mut copy = Workspace::open_in_memory(me).unwrap();
        replay_into(&mine, &mut copy);
        copy
    };
    replay_into(&theirs, &mut mine);
    (old, mine, me, block)
}

#[test]
fn a_racing_peer_edit_is_listed_with_both_texts() {
    let (old, fresh, me, block) = race();
    let found = detect(&old, &fresh, me);
    assert_eq!(found.len(), 1);
    let e = &found[0];
    assert_eq!(e.block_id, block.to_string());
    assert_eq!(e.ours, "oat milk");
    assert_ne!(e.merged, e.ours);
    assert_eq!(e.page_title, "Alpha");
}

#[test]
fn a_peer_edit_with_no_local_edit_is_not_a_merge() {
    let me = ActorId::new();
    let peer = ActorId::new();
    let peer_hlc = HlcGenerator::new(peer);
    let mut theirs = Workspace::open_in_memory(peer).unwrap();
    let page = open_or_create(&mut theirs, &peer_hlc, "alpha", "Alpha", PageKind::Page).unwrap();
    let block = append_block(&mut theirs, &peer_hlc, Some(page), Some("a")).unwrap();
    let mut old = Workspace::open_in_memory(me).unwrap();
    replay_into(&theirs, &mut old);
    edit_text(&mut theirs, &peer_hlc, block, "a b").unwrap();
    let mut fresh = Workspace::open_in_memory(me).unwrap();
    replay_into(&theirs, &mut fresh);
    assert!(detect(&old, &fresh, me).is_empty());
}

fn entry(n: u64) -> MergedEditDto {
    MergedEditDto {
        id: n.to_string(),
        page_id: String::new(),
        page_title: String::new(),
        block_id: String::new(),
        ours: String::new(),
        merged: String::new(),
        peer_actor: String::new(),
        at_ms: n,
    }
}

fn fresh_root(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("outl-merges-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    root
}

/// Drop the in-memory copy, as a new process would start without it.
fn restart(root: &Path) {
    book().lock().unwrap().remove(root);
}

const ME: ActorId = ActorId(ulid::Ulid::from_parts(1, 7));

#[test]
fn the_log_is_newest_first_and_capped() {
    let root = fresh_root("cap");
    record(&root, ME, (0..40).map(entry).collect());
    record(&root, ME, (100..120).map(entry).collect());
    let log = list(&root, ME);
    assert_eq!(log.len(), MERGE_LOG_CAP);
    assert_eq!(log[0].at_ms, 119);
    assert!(take(&root, ME, "119").is_some());
    assert!(take(&root, ME, "119").is_none(), "undo removes the entry");
}

#[test]
fn the_log_survives_a_restart_and_so_does_an_undo() {
    let root = fresh_root("restart");
    record(&root, ME, (0..3).map(entry).collect());
    restart(&root);
    let ids: Vec<_> = list(&root, ME).into_iter().map(|e| e.id).collect();
    assert_eq!(ids, ["2", "1", "0"]);
    assert!(take(&root, ME, "1").is_some());
    restart(&root);
    let ids: Vec<_> = list(&root, ME).into_iter().map(|e| e.id).collect();
    assert_eq!(ids, ["2", "0"], "an undone entry stays gone");
}

#[test]
fn a_repeated_detection_is_listed_once() {
    let root = fresh_root("dup");
    record(&root, ME, vec![entry(5)]);
    restart(&root);
    record(&root, ME, vec![entry(5)]);
    assert_eq!(list(&root, ME).len(), 1);
}

#[test]
fn each_device_keeps_its_own_file_and_a_corrupt_one_starts_empty() {
    let root = fresh_root("corrupt");
    record(&root, ME, vec![entry(1)]);
    assert!(state_path(&root, ME).exists());
    assert_ne!(
        state_path(&root, ME),
        state_path(&root, ActorId(ulid::Ulid::from_parts(1, 8)))
    );
    std::fs::write(state_path(&root, ME), b"{not json").unwrap();
    restart(&root);
    assert!(list(&root, ME).is_empty());
}

/// The log is read as "newest first" by its doc and by the panel that
/// renders `at_ms`. `record` used to sort only the arriving batch and
/// prepend it, which orders by *detection* instead: a peer edit that
/// synced late landed above entries newer than itself.
#[test]
fn a_late_arriving_older_edit_sorts_below_the_newer_ones() {
    let root = fresh_root("late-arrival");
    record(&root, ME, vec![entry(50), entry(40)]);
    // Syncs now, but happened before both of the above.
    record(&root, ME, vec![entry(10)]);

    let times: Vec<u64> = list(&root, ME).iter().map(|e| e.at_ms).collect();
    assert_eq!(times, [50, 40, 10], "newest first, by when it happened");
}
