//! Values the sidecar stores but never reads from the file: the content
//! hash, the file hash, and the `((blk-XXXXXX))` ref handle.
//!
//! All three are pure functions of their input, and all three are
//! computed independently on every device. Two devices that disagree on
//! any of them disagree about which block a reference points at, so the
//! determinism here is a sync property, not a style preference — see the
//! crate's invariants 5 and 7.

use outl_core::id::NodeId;
use sha2::{Digest, Sha256};

/// Prefix every block ref handle carries in the `.md` file.
///
/// `((blk-r6s4a1))` is what users see. The prefix lets a reader (human
/// or parser) tell a block ref apart from page refs / tags at a glance.
pub const REF_HANDLE_PREFIX: &str = "blk-";

/// Number of base32 (Crockford, lowercased) characters taken from the
/// **tail** of the block's ULID to form its ref handle.
///
/// ULIDs are 26 chars total, split as 10 chars of timestamp + 16 chars
/// of random tail. Pulling 6 chars from the tail gives ~30 bits of
/// entropy (~1B values). Birthday-collision probability at 100k blocks
/// is ~5e-6 — effectively zero. Lazy expansion to 7+ chars happens at
/// index-build time if a collision is ever observed (see
/// `WorkspaceIndex`); the sidecar itself always stores whatever handle
/// resolved a given block at write time.
pub const REF_HANDLE_TAIL_LEN: usize = 6;

/// Derive the canonical ref handle for a given block id.
///
/// Format: `blk-` followed by the last [`REF_HANDLE_TAIL_LEN`] characters
/// of the ULID's Crockford base32 representation, lowercased. ULID
/// `Display` is always exactly 26 ASCII characters today; iterating
/// by `chars()` keeps the function safe if a future id encoding ever
/// becomes multi-byte UTF-8.
///
/// Determinism matters: the same block id must always yield the same
/// handle so that two devices building the sidecar independently agree
/// on what `((blk-XXXXXX))` means.
pub fn derive_ref_handle(id: NodeId) -> String {
    let s = id.to_string();
    let total = s.chars().count();
    let skip = total.saturating_sub(REF_HANDLE_TAIL_LEN);
    let tail: String = s.chars().skip(skip).collect();
    format!("{REF_HANDLE_PREFIX}{}", tail.to_lowercase())
}

/// Compute the canonical content hash of a block's text.
///
/// The text is whitespace-normalized (internal whitespace collapsed to a
/// single space, leading/trailing trimmed) before hashing. The result is
/// `sha256:<lowercase-hex>`. Same function used on read and write.
pub fn content_hash(text: &str) -> String {
    let normalized = normalize(text);
    let mut h = Sha256::new();
    h.update(normalized.as_bytes());
    format!("sha256:{}", hex::encode(h.finalize()))
}

/// Compute the hash of the full `.md` file content.
pub fn file_hash(md: &str) -> String {
    let mut h = Sha256::new();
    h.update(md.as_bytes());
    format!("sha256:{}", hex::encode(h.finalize()))
}

fn normalize(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_ref_handle_uses_last_six_chars_lowercased() {
        // The derivation is "take the lowercased tail of the ULID's
        // Display impl". We assert that property holds for an arbitrary
        // id without depending on the `ulid` crate here (outl-md does
        // not have it as a direct dependency).
        let id = NodeId::new();
        let display = id.to_string();
        let expected_tail = display[display.len() - REF_HANDLE_TAIL_LEN..].to_lowercase();
        assert_eq!(
            derive_ref_handle(id),
            format!("{REF_HANDLE_PREFIX}{expected_tail}")
        );
    }

    #[test]
    fn derive_ref_handle_is_deterministic() {
        let id = NodeId::new();
        assert_eq!(derive_ref_handle(id), derive_ref_handle(id));
    }

    #[test]
    fn derive_ref_handle_format_is_blk_prefix_plus_six() {
        let id = NodeId::new();
        let h = derive_ref_handle(id);
        assert!(h.starts_with(REF_HANDLE_PREFIX));
        let tail = &h[REF_HANDLE_PREFIX.len()..];
        assert_eq!(tail.len(), REF_HANDLE_TAIL_LEN);
        assert!(tail.chars().all(|c| c.is_ascii_alphanumeric()));
        assert_eq!(tail, tail.to_lowercase());
    }

    #[test]
    fn content_hash_normalizes_whitespace() {
        let a = content_hash("hello world");
        let b = content_hash("  hello   world  ");
        let c = content_hash("hello\tworld\n");
        assert_eq!(a, b);
        assert_eq!(a, c);
    }

    #[test]
    fn content_hash_differs_on_real_content_changes() {
        assert_ne!(content_hash("hello world"), content_hash("hello worlds"));
    }
}
