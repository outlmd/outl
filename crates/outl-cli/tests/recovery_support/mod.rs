//! Shared harness for the `outl recover` / `outl reconcile` end-to-end
//! tests (`recover_cmd.rs`, `reconcile_cmd.rs`).
//!
//! Both binaries compile this module in full, so an item only one of
//! them uses reads as dead code in the other — hence the blanket allow.
//! A `tests/` helper has no consumers to protect.
#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;
use tempfile::TempDir;

// ---------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------

/// A workspace in its own `TempDir`, with its own device store.
///
/// The device store matters: `outl-core`'s `DeviceStore` is
/// machine-global, and a test battery that resolves the real one writes
/// the developer's `~/.config/outl` full of orphaned `TempDir` bindings
/// — root `CLAUDE.md` invariant 9, third question ("how does a test get
/// its own copy?"). The repo's `.cargo/config.toml` points
/// `OUTL_DEVICE_DIR` at a shared `.dev-device-store`; each test here
/// narrows it further to its own directory so parallel runs cannot see
/// each other's actors.
pub struct Ws {
    pub dir: TempDir,
}

impl Ws {
    /// `outl init` a fresh workspace.
    pub fn new() -> Self {
        let ws = Ws {
            dir: TempDir::new().expect("tempdir"),
        };
        let root = ws.root();
        ws.ok(&["init", root.to_str().expect("utf-8 path")]);
        ws
    }

    pub fn root(&self) -> PathBuf {
        self.dir.path().join("ws")
    }

    pub fn root_str(&self) -> String {
        self.root().to_string_lossy().into_owned()
    }

    pub fn md(&self, rel: &str) -> PathBuf {
        self.root().join(rel)
    }

    /// Run `outl` with this workspace's device store. Never asserts.
    pub fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_outl"))
            .args(args)
            .env("OUTL_DEVICE_DIR", self.dir.path().join("device"))
            .output()
            .expect("failed to spawn the outl binary")
    }

    /// Run and require success, returning stdout.
    pub fn ok(&self, args: &[&str]) -> String {
        let out = self.run(args);
        assert!(
            out.status.success(),
            "`outl {}` must succeed:\nstdout: {}\nstderr: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    /// Run a `--json` subcommand and return its envelope `data`.
    pub fn json(&self, args: &[&str]) -> Value {
        let stdout = self.ok(args);
        let envelope: Value = serde_json::from_str(&stdout).unwrap_or_else(|e| {
            panic!(
                "non-JSON stdout for `outl {}`: {e}\n{stdout}",
                args.join(" ")
            )
        });
        envelope["data"].clone()
    }

    /// Create a page and append one block, returning the block id.
    pub fn seed_block(&self, slug: &str, text: &str) -> String {
        let root = self.root_str();
        self.json(&["page", "create", slug, "--json", "--workspace", &root]);
        let data = self.json(&[
            "block",
            "append",
            "--page",
            slug,
            "--text",
            text,
            "--json",
            "--workspace",
            &root,
        ]);
        data["id"].as_str().expect("block id").to_string()
    }

    /// Create a page holding one keeper block with `children` children,
    /// in a single `append-tree` call.
    pub fn seed_wide_page(&self, slug: &str, children: usize) {
        let root = self.root_str();
        self.json(&["page", "create", slug, "--json", "--workspace", &root]);
        let tree = serde_json::json!({
            "text": "keeper",
            "children": (0..children)
                .map(|i| serde_json::json!({ "text": format!("child {i}") }))
                .collect::<Vec<_>>(),
        })
        .to_string();
        self.json(&[
            "block",
            "append-tree",
            "--page",
            slug,
            "--tree",
            &tree,
            "--json",
            "--workspace",
            &root,
        ]);
    }

    /// Every line currently in `ops/`, file by file, in a stable order.
    ///
    /// The op log is append-only (root `CLAUDE.md` invariant 1), so a
    /// snapshot taken before a write must remain an exact **prefix** of
    /// the snapshot after it. That is the only mechanical way to say
    /// "this restore added history instead of rewriting it".
    pub fn op_log(&self) -> Vec<String> {
        let mut files: Vec<PathBuf> = fs::read_dir(self.root().join("ops"))
            .expect("ops dir")
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
            .collect();
        files.sort();
        files
            .iter()
            .flat_map(|p| {
                fs::read_to_string(p)
                    .expect("read op log")
                    .lines()
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .collect()
    }
}

/// Put a page into the exact state RFC 0210 describes: its `.md` holds
/// content that exists in no op, **and** its sidecar hash agrees with
/// those bytes, so every hash-gated pass reads the page as in-sync.
///
/// Built by hand because the producer that used to create it is fixed.
/// Only `last_synced_hash` moves — the block entries are rewritten
/// byte-identical, which is what makes this the state and not a
/// different one.
pub fn make_ahead_of_log(md_path: &Path, extra: &str) {
    let text = fs::read_to_string(md_path).expect("read .md") + extra;
    write_md_hash_faithful(md_path, &text);
}

/// Replace a page's `.md` wholesale and re-stamp the sidecar hash so the
/// page still reads as in-sync.
///
/// Only `last_synced_hash` moves; the block entries are rewritten
/// byte-identical, which is what keeps this the hash-faithful state and
/// not a different one.
pub fn write_md_hash_faithful(md_path: &Path, text: &str) {
    fs::write(md_path, text).expect("write .md");
    let sidecar_path = outl_md::sidecar::sidecar_path_for(md_path);
    let mut sidecar = outl_md::sidecar::read(&sidecar_path).expect("read sidecar");
    sidecar.last_synced_hash = outl_md::sidecar::file_hash(text);
    outl_md::sidecar::write(&sidecar_path, &sidecar).expect("write sidecar");
}

/// Blank every block's `text` in a sidecar, keeping the hash faithful.
///
/// This is a pre-0.11 sidecar's shape: it lists blocks but records no
/// text for any of them, so it cannot answer "does the op log know this
/// line" — `outl_md::sidecar_can_answer` returns false.
pub fn blank_sidecar_text(md_path: &Path) {
    let sidecar_path = outl_md::sidecar::sidecar_path_for(md_path);
    let mut sidecar = outl_md::sidecar::read(&sidecar_path).expect("read sidecar");
    assert!(
        !sidecar.blocks.is_empty(),
        "the fixture needs a sidecar that lists blocks"
    );
    for block in &mut sidecar.blocks {
        block.text = String::new();
    }
    outl_md::sidecar::write(&sidecar_path, &sidecar).expect("write sidecar");
}
