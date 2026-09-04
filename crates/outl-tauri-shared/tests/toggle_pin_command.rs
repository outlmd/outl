//! RFC 0254 phase 4 — `toggle_pin`, the IPC wrapper over
//! `outl_actions::page::toggle_pin`.
//!
//! `outl-actions`'s own test suite (`page::tests::toggle_pin_*`) pins
//! the read/write logic in isolation; these pin the **command wiring**
//! a real client depends on — the reply carries the flipped state
//! through `PageView.page.pinned`, and a journal refusal surfaces as a
//! string error instead of panicking through `finish_in_page`.

use std::path::PathBuf;
use std::sync::Arc;

use outl_actions::{open_journal, open_or_create_page, PageKind, SyncTransport};
use outl_core::hlc::HlcGenerator;
use outl_core::id::ActorId;
use outl_core::workspace::Workspace;
use outl_exec::RuntimeRegistry;
use outl_tauri_shared::commands::page::toggle_pin;
use outl_tauri_shared::host::AppHost;
use parking_lot::Mutex;
use tempfile::TempDir;

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

fn host() -> (TempDir, TestHost) {
    let tmp = TempDir::new().unwrap();
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let ws = Workspace::open_in_memory(actor).unwrap();
    let host = TestHost {
        workspace: Arc::new(Mutex::new(Some(ws))),
        hlc,
        root: tmp.path().to_path_buf(),
        registry: Arc::new(RuntimeRegistry::new()),
    };
    (tmp, host)
}

#[test]
fn toggle_pin_flips_the_flag_the_reply_carries() {
    let (_tmp, host) = host();
    let page = {
        let mut guard = host.workspace.lock();
        let ws = guard.as_mut().unwrap();
        open_or_create_page(ws, &host.hlc, "inbox", "Inbox", PageKind::Page).unwrap()
    };

    let pinned = toggle_pin(&host, page.to_string()).expect("toggle_pin");
    assert!(
        pinned.page.pinned,
        "the reply must reflect the freshly-flipped state"
    );

    let unpinned = toggle_pin(&host, page.to_string()).expect("toggle_pin");
    assert!(!unpinned.page.pinned);
}

#[test]
fn toggle_pin_on_a_journal_surfaces_a_string_error() {
    let (_tmp, host) = host();
    let journal = {
        let mut guard = host.workspace.lock();
        let ws = guard.as_mut().unwrap();
        open_journal(
            ws,
            &host.hlc,
            chrono::NaiveDate::from_ymd_opt(2026, 7, 9).unwrap(),
        )
        .unwrap()
    };

    let err = toggle_pin(&host, journal.to_string()).unwrap_err();
    assert!(
        err.contains("journal"),
        "the refusal must name why, got {err:?}"
    );
}
