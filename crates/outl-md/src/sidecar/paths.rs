//! Where a page's sidecar lives on disk, and the one-way migration off
//! the legacy dotted name.
//!
//! Separate from reading and writing it on purpose: the answer to
//! "which path" depends on what is already on disk (a legacy
//! `.<stem>.outl` still to be renamed), while reading and writing take
//! the path as given.

use std::path::{Path, PathBuf};

/// Compute the sidecar path for a given `.md` path.
///
/// `pages/foo.md` → `pages/foo.outl`. The `.md` is dropped on purpose —
/// the sidecar always pairs with a markdown file, so encoding the
/// extension twice (`.foo.md.outl`) is noise.
///
/// **The sidecar is not hidden.** Earlier releases stored it as
/// `.foo.outl` to keep it out of casual `ls` output, but that confused
/// iCloud Drive (it would still sync, but Files.app on iOS hides
/// dotted entries entirely, leaving users unable to confirm a
/// peer-side write had landed). Sitting next to its `.md` makes the
/// relationship visible to the user and any other tool walking the
/// directory.
pub fn sidecar_path_for(md_path: &Path) -> PathBuf {
    let parent = md_path.parent().unwrap_or_else(|| Path::new("."));
    let stem = md_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "untitled".to_string());
    parent.join(format!("{stem}.outl"))
}

/// Legacy sidecar path (dotted) used by builds before v0. Kept so the
/// reader can transparently pick up old sidecars and rename them to
/// the modern un-hidden form on first read.
fn legacy_sidecar_path_for(md_path: &Path) -> PathBuf {
    let parent = md_path.parent().unwrap_or_else(|| Path::new("."));
    let stem = md_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "untitled".to_string());
    parent.join(format!(".{stem}.outl"))
}

/// Find the path the caller should use right now to read or write the
/// sidecar for `md_path`.
///
/// In the common case this is the canonical (non-dotted) `<stem>.outl`
/// next to the `.md`. Two transitional cases also return the legacy
/// dotted form so the caller still sees a sidecar where there is one:
///
/// - The modern path doesn't exist yet but a legacy `.<stem>.outl`
///   does and the migration rename to the modern name succeeds — we
///   return the modern path.
/// - Same setup, but the rename fails (read-only filesystem, race with
///   another writer) — we return the legacy dotted path so the caller
///   can still read it. The next successful call moves it.
///
/// Returning the legacy path on rename failure is intentional: callers
/// `read()` and `write()` against whatever we return. If we always
/// returned the modern path while the file was still at the legacy one,
/// `read()` would fail with `NotFound` and the sidecar would appear to
/// be missing.
pub fn resolve_sidecar_path(md_path: &Path) -> PathBuf {
    let modern = sidecar_path_for(md_path);
    if modern.exists() {
        return modern;
    }
    let legacy = legacy_sidecar_path_for(md_path);
    if legacy.exists() {
        if std::fs::rename(&legacy, &modern).is_ok() {
            return modern;
        }
        return legacy;
    }
    modern
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn sidecar_path_is_visible_next_to_md() {
        let p = sidecar_path_for(Path::new("/notes/pages/foo.md"));
        assert_eq!(p, PathBuf::from("/notes/pages/foo.outl"));
    }

    #[test]
    fn sidecar_path_drops_md_extension() {
        // Regression: we used to emit `foo.md.outl`. The `.md` is
        // redundant (sidecars always pair with `.md`) and confusing.
        let p = sidecar_path_for(Path::new("/notes/journals/2026-05-22.md"));
        assert_eq!(
            p,
            PathBuf::from("/notes/journals/2026-05-22.outl"),
            "sidecar must drop the .md extension"
        );
    }

    #[test]
    fn resolve_sidecar_migrates_dotted_legacy() {
        let tmp = TempDir::new().unwrap();
        let md = tmp.path().join("foo.md");
        std::fs::write(&md, "- block\n").unwrap();
        let legacy = tmp.path().join(".foo.outl");
        std::fs::write(&legacy, "{\"version\":2}").unwrap();

        let resolved = resolve_sidecar_path(&md);
        assert_eq!(resolved, tmp.path().join("foo.outl"));
        assert!(
            resolved.exists(),
            "modern sidecar must exist after migration"
        );
        assert!(!legacy.exists(), "legacy dotted sidecar must be gone");
    }
}
