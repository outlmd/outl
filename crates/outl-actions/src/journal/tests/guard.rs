//! `apply_page_md_with_sidecar_guarded` — the locked write path.

use super::*;

/// A local edit must not delete unlogged content either.
///
/// The re-projection guard covers the *read* paths — opening a page.
/// The background projection writer runs after a real mutation and wrote
/// unconditionally, so the deletion the open path refuses happened
/// anyway on the next keystroke commit. Same invariant, a door nobody
/// had checked; found in review of the fix for the first door.
///
/// The user's edit is never at risk here: it went through
/// `Workspace::apply` and lives in the op log. Only the `.md` lags,
/// which is the recoverable direction.
#[test]
fn a_local_edit_does_not_project_over_content_the_log_never_saw() {
    let dir = TempDir::new().expect("tempdir");
    let root = dir.path();
    let actor = ActorId::new();
    let mut ws = Workspace::open_in_memory(actor).expect("ws");
    let hlc = HlcGenerator::new(actor);

    let page = open_or_create(&mut ws, &hlc, "notes", "Notes", PageKind::Page).expect("page");
    append_block(&mut ws, &hlc, Some(page), Some("logged line")).expect("block");
    apply_page_md_with_sidecar(&ws, root, page).expect("first projection");

    // Something lands on disk that the log never saw — the state issue
    // #210 is about.
    let meta = page_meta(&ws, page).expect("meta");
    let path = page_md_path(root, &meta);
    let with_extra = format!(
        "{}- a line that exists in no op\n",
        std::fs::read_to_string(&path).expect("read")
    );
    std::fs::write(&path, &with_extra).expect("write");

    // The user edits the page in the app; the projection writer fires.
    append_block(&mut ws, &hlc, Some(page), Some("newly typed")).expect("edit");
    let result = apply_page_md_with_sidecar_guarded(&ws, root, page);

    assert!(
        matches!(
            result,
            Err(crate::error::ActionError::PageMarkdownAheadOfLog { .. })
        ),
        "the guarded projection must refuse, got {result:?}"
    );
    assert_eq!(
        std::fs::read_to_string(&path).expect("read back"),
        with_extra,
        "the unlogged line must still be on disk"
    );
    let kids = crate::tree::children_of(&ws, page);
    assert_eq!(
        kids.len(),
        2,
        "the user's edit is in the tree regardless of the projection"
    );
}

#[test]
fn guarded_projection_checks_disk_only_after_acquiring_the_page_lock() {
    use fs2::FileExt;

    let dir = TempDir::new().expect("tempdir");
    let root = dir.path();
    let actor = ActorId::new();
    let mut ws = Workspace::open_in_memory(actor).expect("ws");
    let hlc = HlcGenerator::new(actor);
    let page = open_or_create(&mut ws, &hlc, "notes", "Notes", PageKind::Page).expect("page");
    append_block(&mut ws, &hlc, Some(page), Some("logged")).expect("block");
    apply_page_md_with_sidecar(&ws, root, page).expect("seed");
    append_block(&mut ws, &hlc, Some(page), Some("mutation")).expect("edit");

    let path = page_md_path(root, &page_meta(&ws, page).expect("meta"));
    let lock_path = path.with_file_name(format!(
        ".{}.lock",
        path.file_name()
            .and_then(|name| name.to_str())
            .expect("name")
    ));
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(lock_path)
        .expect("lock file");
    lock.lock_exclusive().expect("lock");

    std::thread::scope(|scope| {
        let (tx, rx) = std::sync::mpsc::channel();
        // `Workspace` is `Send` but not `Sync` (its content store caches
        // through `RefCell`), so the writer thread owns it outright.
        scope.spawn(move || {
            tx.send(apply_page_md_with_sidecar_guarded(&ws, root, page))
                .expect("send");
        });
        assert!(
            rx.recv_timeout(std::time::Duration::from_millis(20))
                .is_err(),
            "the guarded projection must wait before reading disk"
        );

        let ahead = format!(
            "{}- arrived while the writer waited\n",
            std::fs::read_to_string(&path).expect("read")
        );
        std::fs::write(&path, &ahead).expect("external write");
        FileExt::unlock(&lock).expect("unlock");

        assert!(matches!(
            rx.recv().expect("result"),
            Err(crate::ActionError::PageMarkdownAheadOfLog { .. })
        ));
        assert_eq!(std::fs::read_to_string(&path).expect("read back"), ahead);
    });
}

/// And the guard stays out of the way on a healthy page — otherwise
/// every mutation in the app would stop projecting.
#[test]
fn a_local_edit_on_a_healthy_page_still_projects() {
    let dir = TempDir::new().expect("tempdir");
    let root = dir.path();
    let actor = ActorId::new();
    let mut ws = Workspace::open_in_memory(actor).expect("ws");
    let hlc = HlcGenerator::new(actor);

    let page = open_or_create(&mut ws, &hlc, "notes", "Notes", PageKind::Page).expect("page");
    append_block(&mut ws, &hlc, Some(page), Some("first")).expect("block");
    apply_page_md_with_sidecar(&ws, root, page).expect("first projection");

    append_block(&mut ws, &hlc, Some(page), Some("second")).expect("edit");
    apply_page_md_with_sidecar_guarded(&ws, root, page).expect("must project");

    let meta = page_meta(&ws, page).expect("meta");
    let on_disk = std::fs::read_to_string(page_md_path(root, &meta)).expect("read");
    assert!(
        on_disk.contains("second"),
        "the edit must reach disk:\n{on_disk}"
    );
}

/// A page with no `.md` yet has nothing on disk to lose.
#[test]
fn the_guard_projects_a_page_that_has_no_md_yet() {
    let dir = TempDir::new().expect("tempdir");
    let root = dir.path();
    let actor = ActorId::new();
    let mut ws = Workspace::open_in_memory(actor).expect("ws");
    let hlc = HlcGenerator::new(actor);

    let page = open_or_create(&mut ws, &hlc, "fresh", "Fresh", PageKind::Page).expect("page");
    append_block(&mut ws, &hlc, Some(page), Some("hello")).expect("block");

    apply_page_md_with_sidecar_guarded(&ws, root, page).expect("a fresh page must project");
}

/// An unreadable sidecar is a refusal, not a fall-through.
///
/// Missing, corrupt, and written-by-a-newer-binary are three states
/// where "does the log know this line" has one honest answer: *I cannot
/// tell*. The first version of the post-mutation guard used
/// `if let Ok(sidecar)` and wrote anyway on all three — so the page the
/// read path protects was overwritten on the next keystroke commit,
/// which is the door this guard was added to close, standing open on a
/// different hinge. Found in review, not by the suite.
#[test]
fn the_guard_refuses_when_the_sidecar_cannot_be_read_at_all() {
    for (name, wreck) in [("missing", 0u8), ("corrupt json", 1), ("newer version", 2)] {
        let dir = TempDir::new().expect("tempdir");
        let root = dir.path();
        let actor = ActorId::new();
        let mut ws = Workspace::open_in_memory(actor).expect("ws");
        let hlc = HlcGenerator::new(actor);

        let page = open_or_create(&mut ws, &hlc, "notes", "Notes", PageKind::Page).expect("page");
        append_block(&mut ws, &hlc, Some(page), Some("logged")).expect("block");
        apply_page_md_with_sidecar(&ws, root, page).expect("seed");

        let meta = page_meta(&ws, page).expect("meta");
        let md_path = page_md_path(root, &meta);
        let sidecar_path = outl_md::sidecar::sidecar_path_for(&md_path);
        let before = std::fs::read_to_string(&md_path).expect("read md");

        match wreck {
            0 => std::fs::remove_file(&sidecar_path).expect("remove"),
            1 => std::fs::write(&sidecar_path, "{ not json").expect("write"),
            _ => std::fs::write(&sidecar_path, r#"{"version":99,"page_id":"01HXY","last_synced_hash":"x","last_synced_at":"2026-05-24T10:00:00-03:00","blocks":[]}"#).expect("write"),
        }

        append_block(&mut ws, &hlc, Some(page), Some("typed after")).expect("edit");
        let result = apply_page_md_with_sidecar_guarded(&ws, root, page);

        assert!(
            matches!(
                result,
                Err(crate::error::ActionError::PageSidecarUnreadable(_))
            ),
            "{name}: an unreadable sidecar must refuse the write, and say *that* rather than \
             claiming the file holds unlogged lines — got {result:?}"
        );
        assert_eq!(
            std::fs::read_to_string(&md_path).expect("read back"),
            before,
            "{name}: the `.md` must be untouched"
        );
    }
}

/// A page whose hash was **withheld** must still raise the banner.
///
/// `reconcile_md` writes `last_synced_hash = ""` when it read content it
/// could not turn into ops. Every downstream gate tests hash-equality,
/// so before this arm existed that page was the one page the user was
/// never told about: `_if_stale` returned `Ok(None)`, no
/// `PageMarkdownAheadOfLog`, no banner. The producer fix erased the
/// signal the same release built to report it.
#[test]
fn a_withheld_hash_still_reaches_the_user() {
    let dir = TempDir::new().expect("tempdir");
    let root = dir.path();
    let actor = ActorId::new();
    let mut ws = Workspace::open_in_memory(actor).expect("ws");
    let hlc = HlcGenerator::new(actor);

    let page = open_or_create(&mut ws, &hlc, "notes", "Notes", PageKind::Page).expect("page");
    append_block(&mut ws, &hlc, Some(page), Some("logged line")).expect("block");
    apply_page_md_with_sidecar(&ws, root, page).expect("seed");

    let meta = page_meta(&ws, page).expect("meta");
    let md_path = page_md_path(root, &meta);
    let sidecar_path = outl_md::sidecar::sidecar_path_for(&md_path);

    // Disk gains a line the log never saw, and the producer withholds
    // the hash — exactly the state `reconcile_md` leaves behind.
    let with_extra = format!(
        "{}- never recorded\n",
        std::fs::read_to_string(&md_path).expect("read")
    );
    std::fs::write(&md_path, &with_extra).expect("write");
    let mut sc = outl_md::sidecar::read(&sidecar_path).expect("sidecar");
    sc.last_synced_hash = String::new();
    outl_md::sidecar::write(&sidecar_path, &sc).expect("rewrite");

    let result = apply_page_md_with_sidecar_if_stale(&ws, root, page);
    assert!(
        matches!(
            result,
            Err(crate::error::ActionError::PageMarkdownAheadOfLog { .. })
        ),
        "a withheld hash must surface, not read as an ordinary pending edit — got {result:?}"
    );
}
