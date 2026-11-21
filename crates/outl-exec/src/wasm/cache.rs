//! Lazy generation cache for `.wasm` modules.
//!
//! Two kinds of artefacts land here:
//!
//! 1. **Per-source compilations** — e.g. a Rust snippet → WASM goes
//!    into `runtimes/rust/<source-hash>.wasm`. Idempotent: same source
//!    text never recompiles.
//! 2. **Drop-in interpreters** — once Steel-WASM / QuickJS-wasi ship,
//!    a user can place `runtimes/<lang>.wasm` here and the registry
//!    discovers it (M2 follow-up).
//!
//! Path resolution:
//! - Linux/BSD → `$XDG_CACHE_HOME/outl/runtimes/` or `~/.cache/outl/runtimes/`
//! - macOS     → `~/Library/Caches/outl/runtimes/`
//! - Windows   → `%LOCALAPPDATA%\outl\runtimes\`
//!
//! Delegated to the `dirs` crate so we follow OS conventions instead
//! of hard-coding paths.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// Root cache directory: `<os cache>/outl/runtimes/`.
///
/// Creates the directory if missing. Returns `None` if the OS doesn't
/// expose a user cache dir (extremely rare — sandboxed CI without a
/// HOME, for example) so the caller can fall back to compiling on the
/// fly without caching.
pub fn cache_dir() -> Option<PathBuf> {
    let mut p = dirs::cache_dir()?;
    p.push("outl");
    p.push("runtimes");
    std::fs::create_dir_all(&p).ok()?;
    Some(p)
}

/// Path where a compiled artefact for `(language, source)` should
/// land. The filename is `<sha256-of-source>.wasm`; collisions are
/// vanishingly unlikely and the hash also doubles as a cache key
/// for "is this source the same as last time?".
///
/// Returns `None` only when `cache_dir()` is unavailable. The caller
/// can fall back to a `tempfile` in that case.
pub fn cache_path_for_source(language: &str, source: &str) -> Option<PathBuf> {
    let mut dir = cache_dir()?;
    dir.push(language);
    std::fs::create_dir_all(&dir).ok()?;
    let mut hasher = Sha256::new();
    hasher.update(source.as_bytes());
    let digest = hasher.finalize();
    let mut hash = String::with_capacity(digest.len() * 2);
    for b in digest {
        use std::fmt::Write;
        let _ = write!(hash, "{b:02x}");
    }
    dir.push(format!("{hash}.wasm"));
    Some(dir)
}

/// Is there a regular, non-empty file at `path`? Used by lazy-gen
/// runtimes ([`crate::runtimes::rust`] today) to decide "skip the
/// compile, just read the bytes".
///
/// `is_file()` is the load-bearing half, not boilerplate. A directory
/// satisfies `exists()`, and on macOS and Linux it also reports a
/// non-zero `metadata().len()` (64 on APFS, 4096 on ext4; Windows
/// reports 0, so only the Unix hosts are exposed). A checker that tests
/// length instead of kind therefore reports a directory as a cache hit,
/// and the caller's `read` fails the user's execution where a recompile
/// was the right answer. A missing path needs no `exists()` call of its
/// own, because `metadata` already errors on one.
pub fn is_fresh(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.len() > 0)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_dir_exists_or_returns_none() {
        // Smoke test — in CI/dev `dirs::cache_dir()` always works.
        // The point is that we never panic.
        let _ = cache_dir();
    }

    #[test]
    fn same_source_hashes_to_same_path() {
        let a = cache_path_for_source("rust", "fn main() {}").unwrap();
        let b = cache_path_for_source("rust", "fn main() {}").unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn different_sources_hash_differently() {
        let a = cache_path_for_source("rust", "fn main() {}").unwrap();
        let b = cache_path_for_source("rust", "fn main() { 1; }").unwrap();
        assert_ne!(a, b);
    }
}
