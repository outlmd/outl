//! Reading a page's `.outl` sidecar, and whether it describes the page
//! it was read *for*.
//!
//! Both halves live here because they are one decision: the sidecar an
//! outline is built through is the sidecar whose `page_id` has to be
//! checked. The read used to be inline in
//! `super::read_page_outline_inner`, which zips the parsed `.md` against
//! `sidecar.blocks` positionally and never compared the two ids.
//!
//! ## The verdict reports; it never refuses the read
//!
//! `identify` answers a question nothing asked before. On the
//! maintainer's workspace 1,775 sidecars name a page root the tree no
//! longer holds under `NodeId::root()`: a Roam re-import on 2026-11-12
//! moved the previous generation of the graph to the trash and nothing
//! re-projected the `.md` / `.outl` pairs. Every live page root there is
//! `NodeId::from_slug(slug)`, which is what makes the check one string
//! comparison rather than a tree walk. Measurements, and the staged
//! repair this is pulled ahead of:
//! `docs/superpowers/specs/2026-11-20-stale-projection-repair-design.md`.
//!
//! **An `Err` here would be the deleting half of root `CLAUDE.md`
//! invariant 8's mirrored pair, arrived at by fixing the reported half.**
//! Three reasons, none of them a preference:
//!
//! - This sits on the **page-open path of every client**. Refusing
//!   trades a stale page for no page at all, which is the trade
//!   `outl-tauri-shared/CLAUDE.md` names as wrong on the hottest path
//!   in the app — the same reason a withheld projection lets the page
//!   open showing exactly what is on disk.
//! - [`crate::build_backlink_index_from_disk`] `warn!`s and *skips the
//!   page* on an `Err` from this read. So a refusal would take 1,775
//!   more pages' `[[ref]]`s out of every backlinks panel — the panel
//!   the mismatch was reported from.
//! - A sidecar naming another root is exactly the state a legacy or
//!   migrated workspace is in, and refusing on a predicate narrower than
//!   the question warranted is what froze every empty page a peer wrote
//!   into ([#332](https://github.com/outlmd/outl/issues/332)).
//!
//! There is also nothing here to refuse *for*: the condition is already
//! draining. Opening a page re-projects it, so the only thing missing
//! was that nobody could see it happening.
//!
//! So the verdict rides on [`super::PageOutline`] beside `warnings` and
//! the open path's control flow is what it was. `tests/sidecar_identity.rs`
//! is the net, and it asserts the no-refusal half on the five shapes a
//! page with nothing on disk takes.

use std::path::Path;

use outl_md::sidecar::Sidecar;
use serde::Serialize;
use tracing::debug;

/// Whether reading the sidecar may also migrate a legacy dotted one to
/// its modern name.
#[derive(Clone, Copy)]
pub(super) enum SidecarAccess {
    /// `outl_md::resolve_sidecar_path`, which renames a legacy sidecar
    /// in place. Every ordinary caller: they run inside an open
    /// workspace, so the guards are held.
    Migrating,
    /// `outl_md::readable_sidecar_path`, which never writes.
    ReadOnly,
}

/// Whether the sidecar read for a page names **that** page's root.
///
/// Reported on [`super::PageOutline`] and acted on by nobody in this
/// crate — see the module doc for why an `Err` would be the deleting
/// direction of the divergence it describes.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub enum SidecarIdentity {
    /// No usable sidecar was read beside the `.md`.
    ///
    /// Three ordinary shapes, none of them this defect: a live page
    /// whose `.md` this device has never written, a brand-new journal
    /// day before its first projection, and a `.md` a peer shipped
    /// without its sidecar. An *unreadable* sidecar lands here too,
    /// because the read is `.ok()`-swallowed — this type reports which
    /// page a sidecar describes, never whether one could be parsed.
    ///
    /// The outline's block ids are then transient, which is the
    /// pre-existing contract [`super::read_page_outline`] documents.
    #[default]
    Absent,
    /// The sidecar names this page's root — the healthy case.
    MatchesPage,
    /// The sidecar describes a **different** page root than the one it
    /// was read for, so the ids it carries belong to another page, or to
    /// a generation of this one the tree no longer holds under
    /// `NodeId::root()`.
    NamesAnotherPage {
        /// The root the sidecar claims, stringified like
        /// [`crate::PageMeta::id`].
        page_id: String,
    },
}

/// Read the page's sidecar, honouring whether this caller may migrate a
/// legacy dotted one.
///
/// An unreadable sidecar comes back as `None`, exactly as an absent one
/// does. That is the read path's pre-existing behaviour and the reason
/// [`SidecarIdentity::Absent`] is phrased as "no usable sidecar" rather
/// than "no file".
pub(super) fn read(md_path: &Path, access: SidecarAccess) -> Option<Sidecar> {
    match access {
        SidecarAccess::Migrating => {
            outl_md::sidecar::read(&outl_md::resolve_sidecar_path(md_path)).ok()
        }
        SidecarAccess::ReadOnly => {
            outl_md::readable_sidecar_path(md_path).and_then(|p| outl_md::sidecar::read(&p).ok())
        }
    }
}

/// The verdict, plus the line this one read leaves behind.
///
/// `page_id` is [`crate::PageMeta::id`] — the **live** page root, already
/// resolved by the caller and already a `String`. So this costs one
/// comparison: no `Workspace`, and no I/O, because `read` parsed the
/// sidecar a few lines up.
///
/// **`debug!`, not `warn!`, because this frame cannot know how many
/// times it is being called — and because the aggregate already has an
/// owner.**
///
/// It shipped as a `warn!` claiming to fire "once per affected page
/// **open**, never per frame". That was false in both halves.
/// `read_page_outline` is one page open *and* the body of
/// [`crate::build_backlink_index_from_disk`], which reads **every** page
/// in the workspace; and that rescan is on an ordinary edit path, since
/// `BacklinkRefresh::DropAll` is the GUI commit default (so the next
/// `page_backlinks` rebuilds) and the TUI re-spawns the build after a
/// cross-page save. On the workspace this check was written for — 1,775
/// of 3,390 sidecars naming a stale root — that is 1,775 records per
/// rescan, reached by editing a block. A signal that long is the same
/// failure as no signal, which is the defect the check was added to
/// close.
///
/// Three reasons there is no counted `warn!` here or in the walk, and
/// the first is the one that decides it:
///
/// - **`outl doctor` is the aggregate**, and reports strictly more than
///   a count: every one of these pages is a stale projection by its
///   survey's own test (`.md` hash-faithful to its sidecar, tree now
///   renders differently), listed with the lines at risk and a repair
///   plan. A second aggregate could only ever say a number, and two
///   owners of one fact is what this repo refuses in review.
/// - **The condition is self-healing**, so a `warn!` on an edit path is
///   a nag: opening a page re-projects it, and the only thing missing
///   was that nobody could see it happening. Nagging about a draining
///   condition trains the reader past the line that does not drain.
/// - **What is left for this frame is narrow** — *which read was I
///   doing when it happened* — and the path is the one thing only this
///   frame has. `debug!` is that: off by default, collected on request
///   with `RUST_LOG=outl_actions=debug`, the same choice for the same
///   reason as `commit_page`'s per-phase timing one module over.
///
/// `tests/sidecar_identity.rs` pins it as a **record count by level**
/// rather than as a sentence: a bulk walk over N mismatched pages emits
/// no `warn!` at all and N `debug!`s, so putting the `warn!` back fails
/// there instead of only being noticed on a 3,390-page graph.
pub(super) fn identify(
    md_path: &Path,
    sidecar: Option<&Sidecar>,
    page_id: &str,
) -> SidecarIdentity {
    let Some(sc) = sidecar else {
        return SidecarIdentity::Absent;
    };
    let named = sc.page_id.to_string();
    if named == page_id {
        return SidecarIdentity::MatchesPage;
    }
    debug!(
        "sidecar for `{}` names page root {named}, not {page_id}: the block ids \
         in this outline describe a page the tree no longer holds there, so the \
         projection on disk is a generation behind the op log",
        md_path.display()
    );
    SidecarIdentity::NamesAnotherPage { page_id: named }
}

#[cfg(test)]
mod tests {
    use super::*;
    use outl_core::id::NodeId;
    use outl_md::sidecar::file_hash;
    use std::path::PathBuf;

    fn md_path() -> PathBuf {
        PathBuf::from("/tmp/pages/notes.md")
    }

    fn sidecar_for(page: NodeId) -> Sidecar {
        Sidecar::new_for_page(page, &file_hash("- hello\n"))
    }

    #[test]
    fn no_sidecar_is_absent_not_a_mismatch() {
        assert_eq!(
            identify(&md_path(), None, &NodeId::new().to_string()),
            SidecarIdentity::Absent,
            "a page with no sidecar must not read as describing another page"
        );
    }

    #[test]
    fn the_pages_own_sidecar_matches() {
        let page = NodeId::new();
        assert_eq!(
            identify(&md_path(), Some(&sidecar_for(page)), &page.to_string()),
            SidecarIdentity::MatchesPage
        );
    }

    #[test]
    fn a_sidecar_for_another_root_reports_the_root_it_claims() {
        let other = NodeId::new();
        let asked_for = NodeId::new();
        assert_eq!(
            identify(
                &md_path(),
                Some(&sidecar_for(other)),
                &asked_for.to_string()
            ),
            SidecarIdentity::NamesAnotherPage {
                page_id: other.to_string(),
            },
            "the verdict has to carry the root the sidecar claims — a bare \
             boolean cannot tell a caller which page the ids belong to"
        );
    }

    /// An empty sidecar is not a mismatch. `Sidecar::new_for_page`
    /// carries the right `page_id` with `blocks: []`, which is what a
    /// page projected before it had any content looks like.
    #[test]
    fn an_empty_sidecar_for_this_page_still_matches() {
        let page = NodeId::new();
        let mut sc = sidecar_for(page);
        sc.blocks.clear();
        assert_eq!(
            identify(&md_path(), Some(&sc), &page.to_string()),
            SidecarIdentity::MatchesPage
        );
    }
}
