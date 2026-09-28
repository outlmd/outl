//! The on-disk payload and the two version numbers that govern it.
//!
//! [`Sidecar`] and [`SidecarBlock`] are the serde shapes; the constants
//! next to them say what a reader may assume about a file it did not
//! write. The long-form reasoning on [`SIDECAR_VERSION`] and
//! [`CURRENT_PIPELINE_VERSION`] is load-bearing — both numbers have
//! already cost a fleet-wide incident when moved for the wrong reason.

use super::digest::{content_hash, derive_ref_handle};
use chrono::{DateTime, FixedOffset, Local};
use outl_core::id::NodeId;
use serde::{Deserialize, Serialize};

/// Current sidecar format version.
///
/// Version history:
/// - **1** — initial format (page_id, last_synced_hash, blocks with id /
///   line / indent / content_hash).
/// - **2** — `ref_handle` on every block to power `((blk-XXXXXX))`
///   inline references. Backward-compatible read: v1 sidecars load fine
///   and their handles are derived on the fly from the block id.
///   Later **additive** fields ride along at this same version:
///   `pipeline_version` on the payload, [`SidecarBlock::text`] on every
///   block. See the bump rule below for why they did not move the
///   number.
///
/// # When to bump — and when not to
///
/// The version answers exactly one question for a reader that did not
/// write the file: *can I still trust the fields I know?* Compatibility
/// runs in **both** directions, and the two directions are not
/// symmetric.
///
/// - **Backward** — this binary reading an older payload. Always
///   supported, down to [`MIN_READABLE_SIDECAR_VERSION`]. A read path is
///   never dropped when a newer one lands.
/// - **Forward** — an *already shipped* binary reading a payload this
///   one wrote. Every released binary rejects
///   `version > its own SIDECAR_VERSION`, and that rejection cannot be
///   patched retroactively. On the paths that consume a sidecar, an
///   unreadable one used to look exactly like a missing one: no old
///   blocks, so every block matched at level 3, got a fresh ULID, and
///   the old id stayed in the tree — one boot of a stale device on a
///   shared iCloud folder duplicated the whole workspace and broke
///   every `((blk-…))` handle. The devices in one workspace never
///   update at the same instant (TestFlight lag, a laptop closed for a
///   week), so this is a real user, not a hypothetical one.
///
/// Hence the rule:
///
/// - **An additive field does NOT bump the version.** Give it
///   `#[serde(default)]` and the format stays readable in both
///   directions: an older reader ignores the unknown JSON key, a newer
///   reader treats "missing" as "feature off for this entry".
///   `pipeline_version` and [`SidecarBlock::text`] are both this shape.
///   **Feature detection is per-field presence, never per version
///   number** — an empty `text` disables level-2 matching for that
///   block whatever the number on the file says, which is also the only
///   correct answer once an old binary rewrites the sidecar and drops
///   the field it never knew about.
/// - **Bump only when an older reader would _misread_ the file** — an
///   existing field changes meaning, changes encoding, or goes away.
///   There the old binary's [`SidecarError::UnsupportedVersion`] is the
///   *desired* outcome: a loud refusal beats silent corruption, and
///   `reconcile_md` propagates that error instead of rebuilding the
///   page from scratch.
/// - A bump is a coordinated release, not a patch. It needs a migration
///   note in `docs/markdown-format.md` and an `outl doctor` path,
///   because every device that has not updated stops reconciling those
///   pages until it does.
///
/// Note: the sidecar is intentionally **only structural metadata**
/// (ids, position, content hashes, ref handles, last-synced text). Any
/// state that needs to *converge between devices* — collapsed/folded
/// blocks, future per-block flags — goes through the `Op` log in
/// `outl-core`, not here. The sidecar is a projection cache for
/// matching `.md` ↔ tree; it is not a sync surface. See the root
/// `CLAUDE.md` invariants.
pub const SIDECAR_VERSION: u32 = 2;

/// Lowest sidecar version this crate is willing to read.
///
/// Older versions return [`SidecarError::UnsupportedVersion`]. Keeping
/// this explicit (rather than a magic number in `read`) so the contract
/// is greppable when a future version ever needs to drop v1
/// support.
pub const MIN_READABLE_SIDECAR_VERSION: u32 = 1;

/// One block entry in the sidecar.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SidecarBlock {
    /// Block id.
    pub id: NodeId,
    /// 1-indexed line number in the `.md` at last sync.
    pub line: usize,
    /// Indent level (0 for top-level outline items).
    pub indent: u32,
    /// SHA-256 of the block's textual content, formatted `sha256:<hex>`.
    pub content_hash: String,
    /// Short, stable, human-typeable handle for `((blk-XXXXXX))` inline
    /// references and `!((blk-XXXXXX))` embeds.
    ///
    /// Default-derived from [`derive_ref_handle`] using the block id.
    /// The handle is stable as long as the block keeps the same id —
    /// editing the block's text does **not** change it. Persisted so
    /// that a future change to the derivation scheme cannot invalidate
    /// existing references already living in `.md` files.
    ///
    /// `#[serde(default)]` is what makes v1 sidecars load cleanly:
    /// missing handles are backfilled by [`crate::sidecar::read`] from
    /// the id.
    #[serde(default)]
    pub ref_handle: String,
    /// The block's text **as of the last sync** — the input level-2
    /// matching diffs a freshly-parsed `.md` against.
    ///
    /// `content_hash` can only answer "identical or not"; recovering an
    /// id after the user rewords a block needs the old string itself.
    /// Without it, any save that both edits one block and adds/removes
    /// another falls straight to level 3: fresh ULID, old id orphaned,
    /// every `((blk-…))` pointing at it broken.
    ///
    /// Stored **verbatim and in full** (not truncated). A prefix would
    /// make two blocks that share a long opening look identical, and a
    /// level-2 false positive hands one block's id — and its ref handle
    /// — to a different block. That is the exact corruption matching
    /// exists to prevent, so the duplicated bytes are the cheaper side
    /// of the trade.
    ///
    /// **Additive on purpose, at the same [`SIDECAR_VERSION`].**
    /// `#[serde(default)]` is what lets a payload written before this
    /// field existed load cleanly — and, just as importantly, what lets
    /// a binary that predates the field keep reading (and rewriting)
    /// the sidecar without the version number turning it away. A block
    /// whose `text` is missing or empty simply doesn't participate in
    /// level 2; matching degrades to hash + position, exactly what
    /// shipped before, never worse. The next write by a binary that
    /// knows the field records the text again.
    #[serde(default)]
    pub text: String,
}

impl SidecarBlock {
    /// Build an entry for `text` at `line` / `indent`, deriving
    /// `content_hash` and the default `ref_handle` from `id`.
    ///
    /// The one constructor every caller building a sidecar from a tree
    /// or an AST should use: it keeps hash, handle, and stored text
    /// derived from the same string, so a block can never end up with
    /// a `content_hash` describing one revision and a `text` from
    /// another. Callers preserving a *previous* handle (an expanded
    /// one, post-collision) build the literal and overwrite
    /// `ref_handle`.
    pub fn from_text(id: NodeId, line: usize, indent: u32, text: impl Into<String>) -> Self {
        let text = text.into();
        Self {
            id,
            line,
            indent,
            content_hash: content_hash(&text),
            ref_handle: derive_ref_handle(id),
            text,
        }
    }
}

/// Full sidecar payload.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sidecar {
    /// Format version. Always present.
    pub version: u32,
    /// Page id (also the root block id).
    pub page_id: NodeId,
    /// SHA-256 of the full `.md` at last sync (`sha256:<hex>`).
    pub last_synced_hash: String,
    /// When the sidecar was last written. ISO 8601 with timezone.
    pub last_synced_at: DateTime<FixedOffset>,
    /// Block entries in tree (depth-first preorder) order.
    pub blocks: Vec<SidecarBlock>,
    /// Reconcile-pipeline version that produced the tree state this
    /// sidecar describes.
    ///
    /// Bumped whenever the pipeline learns to emit a category of op
    /// it didn't before — `diff_to_ops_with_page_props` propagating
    /// page-level `Op::SetProp`s, `ensure_page_root_in_tree` emitting
    /// `Op::Create` for the page root, etc. The orphan scanner
    /// (`needs_reconcile`) re-runs `reconcile_md` when this value is
    /// lower than [`CURRENT_PIPELINE_VERSION`], so a binary that gains
    /// a new pipeline step automatically rematerialises every legacy
    /// page on the next boot — no user intervention, idempotent on
    /// the CRDT.
    ///
    /// Sidecars predating this field (including earlier intermediates
    /// that used a boolean flag of a different name) deserialise as
    /// `0` via `#[serde(default)]`, which forces a re-reconcile
    /// against the current pipeline.
    #[serde(default)]
    pub pipeline_version: u32,
}

/// The pipeline version this binary writes to fresh sidecars and
/// expects to find on disk before treating a page as fully
/// reconciled.
///
/// Bump every time the reconcile pipeline acquires a new pass that
/// could have produced a different op log for the same `.md`:
///
/// - `1` — `diff_to_ops_with_page_props` propagates page-level
///   properties (`title::`, `type::`, `pinned::`, …) as
///   `Op::SetProp` on the page root.
/// - `2` — `ensure_page_root_in_tree` emits `Op::Create` for the page
///   root when the node isn't in `self.nodes` yet. Without it,
///   externally-authored `.md` files left the page as an unrooted
///   ghost (`Op::Move` is a no-op on never-created nodes), so
///   `children_of(root)` skipped them silently.
/// - `3` — the parser stopped discarding prose that follows a block
///   property. A `key:: value` line used to close continuation for the
///   rest of the block, so every following text line was dropped with
///   no AST entry and no warning — the same `.md` now parses to more
///   text, which is precisely "a different op log for the same file".
///   Without this bump those pages stay hash-faithful forever and the
///   content never enters the log, because the short-circuit in
///   `reconcile_md` only consults the hash. Measured on a real
///   workspace: 233 pages, 1,426 lines. See issue #210 / RFC 0210.
/// - `4` — the parser stopped losing content three further ways, all of
///   them reachable from one ordinary shape (a block whose text carries
///   a blank line): an over-indented line was recovered only at depth 0
///   and skipped mutely below it; a blank line inside a block's text was
///   read as a separator; and a continuation line's own indentation
///   pushed it past the level that could claim it. Same file, more text,
///   so the same reasoning as `3` applies — without the bump those pages
///   stay hash-faithful and their content never enters the log.
///   Measured against the same workspace: pages holding unlogged content
///   went from 41 to 8, lines from 387 to 49.
///
///   This bump was **missed** in the first version of that change and
///   caught in review. Worth naming, because the failure is invisible
///   exactly where it matters: on the author's own machine the recovery
///   commands get run by hand, so nothing looks wrong, while every other
///   user keeps content outside the log with no symptom at all.
/// - `5` — the parser stopped reading a leading YAML frontmatter fence as
///   outline blocks. The same `.md` now produces one `Op::SetProp` on the
///   page root instead of one `Op::Create` per delimiter and key line, so
///   it is the "different op log for the same file" criterion exactly.
///
///   The bump is what repairs the pages already hit. A workspace that ran
///   an earlier binary over an Obsidian vault has the fence in its log as
///   bullets *and* a sidecar whose hash matches the file, so the
///   short-circuit above would skip the page forever — the fence would
///   keep rendering as `- ---` and the next projection would write it
///   back. Re-reconciling re-reads the fence as page metadata and orphans
///   the bullet blocks. See [issue #281].
///
/// [issue #281]: https://github.com/outlmd/outl/issues/281
pub const CURRENT_PIPELINE_VERSION: u32 = 5;

impl Sidecar {
    /// Build an empty sidecar for a new page.
    pub fn new_for_page(page_id: NodeId, md_hash: &str) -> Self {
        Self {
            version: SIDECAR_VERSION,
            page_id,
            last_synced_hash: md_hash.to_string(),
            last_synced_at: now_local(),
            blocks: Vec::new(),
            // Fresh sidecars stamp the current pipeline so the orphan
            // scanner skips them next time.
            pipeline_version: CURRENT_PIPELINE_VERSION,
        }
    }
}

/// Errors loading or storing a sidecar.
#[derive(Debug, thiserror::Error)]
pub enum SidecarError {
    /// JSON parse failure.
    #[error("invalid sidecar JSON: {0}")]
    InvalidJson(#[from] serde_json::Error),
    /// I/O failure reading/writing the sidecar file.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// Unsupported sidecar version.
    #[error("unsupported sidecar version: {0}")]
    UnsupportedVersion(u32),
}

fn now_local() -> DateTime<FixedOffset> {
    Local::now().fixed_offset()
}
