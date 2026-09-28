//! `.outl` sidecar file — JSON dotfile next to each `.md`.
//!
//! Holds the IDs and content hashes the clean `.md` cannot. See
//! `docs/markdown-format.md` §sidecar for the format spec.
//!
//! This file owns **loading and storing** one sidecar. The three
//! pieces it needs live next to it, each answering a different
//! question, and every name below is re-exported here so
//! `outl_md::sidecar::…` stays the single path callers use:
//!
//! - `payload` — what the file *is*: the serde shapes plus the two
//!   version numbers that say what a reader may assume about a file it
//!   did not write.
//! - `paths` — where the file *lives*, including the migration off
//!   the legacy dotted name.
//! - `digest` — the values it stores that are *derived* rather than
//!   observed: content hash, file hash, ref handle.

mod digest;
mod paths;
mod payload;

pub use digest::{
    content_hash, derive_ref_handle, file_hash, REF_HANDLE_PREFIX, REF_HANDLE_TAIL_LEN,
};
pub use paths::{resolve_sidecar_path, sidecar_path_for};
pub use payload::{
    Sidecar, SidecarBlock, SidecarError, CURRENT_PIPELINE_VERSION, MIN_READABLE_SIDECAR_VERSION,
    SIDECAR_VERSION,
};

use std::path::Path;

/// Read and validate a sidecar from disk.
///
/// Accepts any version in `[MIN_READABLE_SIDECAR_VERSION, SIDECAR_VERSION]`.
/// Older payloads are upgraded in-memory: every block missing a
/// `ref_handle` gets one [derived from its id](derive_ref_handle), and
/// the in-memory `version` is bumped to [`SIDECAR_VERSION`]. The next
/// [`write()`] then persists the upgraded shape.
///
/// A missing `text` is **not** backfilled — there is nothing to backfill
/// it from, since the whole point of the field is to hold the text as it
/// was *before* the `.md` on disk changed. Those blocks come back with
/// an empty `text` and level-2 matching skips them; the next [`write()`]
/// records their current text, so the page is covered from that point
/// on. This is the steady state in a mixed-version workspace, where a
/// peer that predates the field rewrites the sidecar without it.
///
/// **Unknown JSON keys are ignored, not rejected.** That is half of the
/// forward-compatibility contract described on [`SIDECAR_VERSION`]: a
/// field added by a newer binary at the same version costs this reader
/// nothing. The other half is that a *higher* version is refused with
/// [`SidecarError::UnsupportedVersion`], because by that rule a bumped
/// number means a field this reader already knows has changed meaning —
/// the one case where guessing is worse than stopping.
pub fn read(path: &Path) -> Result<Sidecar, SidecarError> {
    let s = std::fs::read_to_string(path)?;
    let mut sc: Sidecar = serde_json::from_str(&s)?;
    if sc.version < MIN_READABLE_SIDECAR_VERSION || sc.version > SIDECAR_VERSION {
        return Err(SidecarError::UnsupportedVersion(sc.version));
    }
    for b in &mut sc.blocks {
        if b.ref_handle.is_empty() {
            b.ref_handle = derive_ref_handle(b.id);
        }
    }
    sc.version = SIDECAR_VERSION;
    Ok(sc)
}

/// Write a sidecar to disk as pretty-printed JSON.
///
/// Uses [`crate::atomic::write_atomic`] so a crash mid-write can never
/// leave a half-written sidecar that would fail to parse on next open.
pub fn write(path: &Path, sidecar: &Sidecar) -> Result<(), SidecarError> {
    let s = serde_json::to_string_pretty(sidecar)?;
    crate::atomic::write_atomic(path, s.as_bytes())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use outl_core::id::NodeId;
    use tempfile::TempDir;

    #[test]
    fn roundtrip_through_disk() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".foo.outl");
        let sc = Sidecar::new_for_page(NodeId::new(), &file_hash("- hello\n"));
        write(&path, &sc).unwrap();
        let loaded = read(&path).unwrap();
        assert_eq!(loaded.version, SIDECAR_VERSION);
        assert_eq!(loaded.page_id, sc.page_id);
        assert_eq!(loaded.last_synced_hash, sc.last_synced_hash);
    }

    #[test]
    fn unsupported_version_fails_loudly() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".bad.outl");
        std::fs::write(
            &path,
            r#"{"version":99,"page_id":"01HXY","last_synced_hash":"x","last_synced_at":"2026-05-24T10:00:00-03:00","blocks":[]}"#,
        )
        .unwrap();
        match read(&path) {
            Err(SidecarError::InvalidJson(_)) | Err(SidecarError::UnsupportedVersion(99)) => {}
            other => panic!("expected version/json error, got {other:?}"),
        }
    }

    #[test]
    fn future_version_is_refused_even_when_every_known_field_is_valid() {
        // The forward half of the compatibility contract on
        // `SIDECAR_VERSION`: by the bump rule, a *higher* number can
        // only mean a field this reader already knows changed meaning.
        // Guessing there is worse than stopping, so `read` refuses —
        // and `reconcile_md` propagates that refusal instead of
        // rebuilding the page from scratch with fresh ULIDs.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("future.outl");
        let page = NodeId::new();
        let block = NodeId::new();
        let json = format!(
            r#"{{
              "version": 99,
              "page_id": "{page}",
              "last_synced_hash": "sha256:abc",
              "last_synced_at": "2026-05-24T10:00:00-03:00",
              "pipeline_version": 2,
              "blocks": [
                {{
                  "id": "{block}",
                  "line": 1,
                  "indent": 0,
                  "content_hash": "sha256:def",
                  "ref_handle": "blk-abcdef",
                  "text": "still perfectly parseable"
                }}
              ]
            }}"#
        );
        std::fs::write(&path, json).unwrap();
        match read(&path) {
            Err(SidecarError::UnsupportedVersion(99)) => {}
            other => panic!("a future version must be refused, got {other:?}"),
        }
    }

    #[test]
    fn unknown_fields_at_the_current_version_are_ignored_not_rejected() {
        // The other half of the contract: an additive field does NOT
        // bump the version, so this reader has to survive keys it has
        // never heard of. If this ever regresses to `deny_unknown_
        // fields`, the next additive field becomes a fleet-wide break
        // for every already-shipped binary.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("newer.outl");
        let page = NodeId::new();
        let block = NodeId::new();
        let json = format!(
            r#"{{
              "version": {SIDECAR_VERSION},
              "page_id": "{page}",
              "last_synced_hash": "sha256:abc",
              "last_synced_at": "2026-05-24T10:00:00-03:00",
              "pipeline_version": 2,
              "invented_by_a_newer_binary": {{"nested": [1, 2, 3]}},
              "blocks": [
                {{
                  "id": "{block}",
                  "line": 1,
                  "indent": 0,
                  "content_hash": "sha256:def",
                  "ref_handle": "blk-abcdef",
                  "text": "hello",
                  "some_future_per_block_field": 42
                }}
              ]
            }}"#
        );
        std::fs::write(&path, json).unwrap();
        let sc = read(&path).expect("unknown keys must not fail the read");
        assert_eq!(sc.blocks.len(), 1);
        assert_eq!(sc.blocks[0].id, block);
        assert_eq!(sc.blocks[0].ref_handle, "blk-abcdef");
        assert_eq!(sc.blocks[0].text, "hello");
    }

    #[test]
    fn the_text_field_did_not_bump_the_version() {
        // Regression guard for the incident this rule came from: `text`
        // was shipped as v3, every already-released binary rejected the
        // file, and on the paths that consume a sidecar a rejected one
        // looked exactly like a missing one — fresh ULID per block,
        // every `((blk-…))` handle rotated, duplicates on both sides of
        // the sync. The field is additive; the number must not move.
        assert_eq!(
            SIDECAR_VERSION, 2,
            "adding a `#[serde(default)]` field must not bump \
             SIDECAR_VERSION — see the bump rule on that constant"
        );

        let dir = TempDir::new().unwrap();
        let path = dir.path().join("written.outl");
        let mut sc = Sidecar::new_for_page(NodeId::new(), &file_hash("- hello\n"));
        sc.blocks
            .push(SidecarBlock::from_text(NodeId::new(), 1, 0, "hello"));
        write(&path, &sc).unwrap();

        let raw: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            raw["version"], 2,
            "what lands on disk is what an older binary version-checks"
        );
        assert_eq!(raw["blocks"][0]["text"], "hello");
    }

    #[test]
    fn v1_sidecar_loads_and_backfills_ref_handle() {
        // Hand-written v1 payload (no `ref_handle` field on the block).
        // We deserialize through `read` and assert it:
        //   1. parses without error,
        //   2. surfaces version == SIDECAR_VERSION on the in-memory
        //      value (upgrade-on-read),
        //   3. populates a non-empty `ref_handle` derived from `id`.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".legacy.outl");
        let id = NodeId::new();
        let v1_json = format!(
            r#"{{
              "version": 1,
              "page_id": "{page}",
              "last_synced_hash": "sha256:abc",
              "last_synced_at": "2026-05-24T10:00:00-03:00",
              "blocks": [
                {{
                  "id": "{block}",
                  "line": 1,
                  "indent": 0,
                  "content_hash": "sha256:def"
                }}
              ]
            }}"#,
            page = NodeId::new(),
            block = id,
        );
        std::fs::write(&path, v1_json).unwrap();
        let sc = read(&path).unwrap();
        assert_eq!(sc.version, SIDECAR_VERSION);
        assert_eq!(sc.blocks.len(), 1);
        assert_eq!(sc.blocks[0].ref_handle, derive_ref_handle(id));
    }

    #[test]
    fn write_then_read_v2_preserves_ref_handle() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".foo.outl");
        let page_id = NodeId::new();
        let block_id = NodeId::new();
        let mut sc = Sidecar::new_for_page(page_id, &file_hash("- hello\n"));
        sc.blocks
            .push(SidecarBlock::from_text(block_id, 1, 0, "hello"));
        write(&path, &sc).unwrap();

        let loaded = read(&path).unwrap();
        assert_eq!(loaded.version, SIDECAR_VERSION);
        assert_eq!(loaded.blocks.len(), 1);
        assert_eq!(loaded.blocks[0].ref_handle, derive_ref_handle(block_id));

        // And the on-disk JSON actually contains the field — guards
        // against a future serde attribute accidentally skipping it.
        let on_disk = std::fs::read_to_string(&path).unwrap();
        assert!(
            on_disk.contains("ref_handle"),
            "v2 sidecar must persist ref_handle on disk; got: {on_disk}"
        );
    }
}
