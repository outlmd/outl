//! The single owner of "who tells the user about a refusal, and how"
//! — one exhaustive `match` over ([`Refusal`], [`Surface`]), mirroring
//! the mechanism `outl_shortcuts::support` uses for `(Action, Client)`
//! (RFC 0255).
//!
//! ## Why a second catalog, not the same one
//!
//! `outl_shortcuts::Client` has three members on purpose — TUI,
//! desktop, mobile — because it answers "does this thing that draws
//! an outline handle the chord". The CLI and the MCP server are
//! deliberately excluded from it: neither renders an outline, so
//! neither can ever "perform an action" in that sense.
//!
//! A refusal is a different question. [`crate::ActionError::PageMarkdownAheadOfLog`]
//! (invariant 8) is something **every** surface that can write a page
//! owes an explanation for when it declines — a CLI script and an
//! MCP-driven agent both need to know "this page stopped syncing" as
//! much as a person looking at a screen does. Reusing `Client` here
//! would give every one of `Action::ALL`'s rows two columns that can
//! only ever say "not applicable" (RFC 0255, "Why not the
//! alternatives").
//!
//! ## Why this lives in `outl-actions`, not `outl-shortcuts`
//!
//! [`crate::ActionError`] is defined in this crate. `outl-shortcuts`
//! is a leaf catalog crate with exactly one dependency (`serde`) — it
//! has no reason to know what an `ActionError` is, and giving it one
//! would point the dependency arrow the wrong way: this crate sits
//! *under* every client, while `outl-shortcuts` sits *beside* it,
//! consumed only by the clients that draw chords. So the matrix sits
//! next to the error type it describes instead, with its own small
//! `Support` enum rather than an import of `outl_shortcuts::Support`
//! — reusing that would be the same wrong-direction dependency one
//! type down, for a shape (five surfaces, not three clients) that
//! doesn't fit `ClientSupport` anyway.
//!
//! ## Populated so far
//!
//! [`Refusal::ALL`] has one member, deliberately. Adding the rest of
//! `ActionError`'s variants is mechanical once the mechanism exists;
//! doing it in the same change that built the mechanism would bury it
//! under forty declarations (RFC 0255, Scope).

/// A refusal a write path hands back instead of silently doing
/// nothing — or silently doing the wrong thing.
///
/// Named independently of [`crate::ActionError`] rather than wrapping
/// it: a `Refusal` is a fact about *user communication* ("does this
/// surface explain itself when this happens"), and only some
/// `ActionError` variants rise to that bar. Most are ordinary
/// validation failures every surface already reports the same way
/// (`BLOCK_NOT_FOUND`, `INVALID_ARG`, …) and don't need a row here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// [`crate::ActionError::PageMarkdownAheadOfLog`] — invariant 8's
    /// refusal to overwrite a `.md` holding content that exists in no
    /// op.
    PageMarkdownAheadOfLog,
}

impl Refusal {
    /// Every refusal tracked in this catalog, for exhaustive
    /// iteration in tests and in the doc generator.
    pub const ALL: [Refusal; 1] = [Refusal::PageMarkdownAheadOfLog];
}

/// A surface that can attempt a write [`Refusal`] might decline.
///
/// Five members — not `outl_shortcuts::Client`'s three — because a
/// refusal is owed to whoever asked for the write, whether or not
/// that asker draws an outline. See the module doc for why this
/// isn't `Client` with two more variants bolted on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Surface {
    /// `outl-tui`.
    Tui,
    /// `outl-desktop`.
    Desktop,
    /// `outl-mobile`.
    Mobile,
    /// The `outl` CLI subcommands.
    Cli,
    /// `outl mcp serve`.
    Mcp,
}

impl Surface {
    /// Every surface, for exhaustive iteration in tests and in the
    /// doc generator.
    pub const ALL: [Surface; 5] = [
        Surface::Tui,
        Surface::Desktop,
        Surface::Mobile,
        Surface::Cli,
        Surface::Mcp,
    ];
}

/// What a surface does when a [`Refusal`] happens on a write it made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Support {
    /// The surface tells whoever asked for the write about the
    /// refusal. The string describes how — it feeds the generated doc
    /// table, and is not itself end-user copy (contrast
    /// `outl_shortcuts::Support::nudge`, which *is* shown to a user
    /// directly).
    Full(&'static str),
    /// Not surfaced today. The string says why, so the gap is
    /// declared rather than silently absent — invariant 12.
    Missing(&'static str),
}

impl Support {
    /// The description string, regardless of which state it's in.
    ///
    /// Test-only: the only consumer today is the doc-table generator
    /// and its own pinning test.
    #[cfg(test)]
    fn text(self) -> &'static str {
        match self {
            Support::Full(t) | Support::Missing(t) => t,
        }
    }
}

/// One [`Refusal`]'s support across every surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SurfaceSupport {
    /// `outl-tui`.
    pub tui: Support,
    /// `outl-desktop`.
    pub desktop: Support,
    /// `outl-mobile`.
    pub mobile: Support,
    /// The `outl` CLI.
    pub cli: Support,
    /// `outl mcp serve`.
    pub mcp: Support,
}

impl SurfaceSupport {
    /// Support for one surface.
    pub fn get(&self, surface: Surface) -> Support {
        match surface {
            Surface::Tui => self.tui,
            Surface::Desktop => self.desktop,
            Surface::Mobile => self.mobile,
            Surface::Cli => self.cli,
            Surface::Mcp => self.mcp,
        }
    }
}

/// Shorthand so the table below reads as five columns.
macro_rules! row {
    ($tui:expr, $desktop:expr, $mobile:expr, $cli:expr, $mcp:expr) => {
        SurfaceSupport {
            tui: $tui,
            desktop: $desktop,
            mobile: $mobile,
            cli: $cli,
            mcp: $mcp,
        }
    };
}

use Support::Full;

/// Per-surface support for every refusal in the catalog.
///
/// **Exhaustive on purpose**, the same mechanism as
/// `outl_shortcuts::support`: adding a [`Refusal`] variant does not
/// compile until every surface has declared what it does about it.
pub fn refusal_support(refusal: Refusal) -> SurfaceSupport {
    match refusal {
        Refusal::PageMarkdownAheadOfLog => row!(
            Full(
                "A status-line message wherever a TUI-initiated write re-projects the page \
                 (template apply, `call:` code-block exec, mention-creation autocomplete) and \
                 a toast when a peer-sync reload's re-projection declines \
                 (`SyncEngine::reproject_page`, `reload_workspace_from_disk`). The TUI still \
                 does not call `apply_page_md_with_sidecar_if_stale` on its own page-open path, \
                 so a page that drifted ahead of the log with no local write attempt in between \
                 stays silent until the next write touches it."
            ),
            Full(
                "`<PageAheadOfLogBanner client=\"desktop\" />` above the outline, from \
                 `PageView.md_ahead_of_log`. Names the command to run in the workspace folder."
            ),
            Full(
                "Same banner, `client=\"mobile\"`. **There is no `outl` binary on iOS**, so \
                 the copy says to open the workspace on a computer instead of pointing at a \
                 terminal that doesn't exist."
            ),
            Full(
                "Every write subcommand that touches an existing page (`page update`, \
                 `block append`, `template apply`/`run`, ...) returns the same structured \
                 `PAGE_MARKDOWN_AHEAD_OF_LOG` JSON error the MCP does, with `--json`. \
                 `outl doctor` names the page, the line count and one sample outside any \
                 write attempt; `outl reconcile --ahead-of-log` is the recovery."
            ),
            Full(
                "A structured tool refusal — `PAGE_MARKDOWN_AHEAD_OF_LOG` — naming the page, \
                 line count, sample, and the recovery command, instead of a generic failure."
            )
        ),
    }
}

/// Render the refusal matrix that `docs/clients.md`'s
/// "Surfacing a page that stopped syncing" section carries, so the
/// doc is a projection of the code rather than a second, hand-kept
/// copy of it. Mirrors `outl_shortcuts::support::parity_table_markdown`.
///
/// Test-only, same reasoning as its sibling: regenerating is a
/// `cargo test` away, see `the_clients_doc_matches_the_refusal_matrix`.
#[cfg(test)]
pub(crate) fn refusal_matrix_markdown() -> String {
    let mut out = String::new();
    for refusal in Refusal::ALL {
        let s = refusal_support(refusal);
        out.push_str(&format!("**`{refusal:?}`**\n\n"));
        out.push_str("| Client | Surface |\n|---|---|\n");
        out.push_str(&format!("| CLI | {} |\n", s.cli.text()));
        out.push_str(&format!("| Desktop | {} |\n", s.desktop.text()));
        out.push_str(&format!("| Mobile | {} |\n", s.mobile.text()));
        out.push_str(&format!("| TUI | {} |\n", s.tui.text()));
        out.push_str(&format!("| MCP | {} |\n", s.mcp.text()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_surface_declares_every_refusal() {
        // The `match` in `refusal_support` is exhaustive, so this
        // cannot fail for a missing arm — it fails for a missing
        // *variant* in `Refusal::ALL`, which would silently shrink
        // every other test in this module and the doc table with it.
        for refusal in Refusal::ALL {
            let s = refusal_support(refusal);
            for surface in Surface::ALL {
                let _ = s.get(surface);
            }
        }
        assert_eq!(Refusal::ALL.len(), 1, "Refusal::ALL changed size");
    }

    #[test]
    fn every_declared_state_explains_itself() {
        for refusal in Refusal::ALL {
            let s = refusal_support(refusal);
            for surface in Surface::ALL {
                let text = s.get(surface).text();
                assert!(
                    !text.trim().is_empty(),
                    "{refusal:?} on {surface:?} has an empty description",
                );
            }
        }
    }

    #[test]
    fn the_clients_doc_matches_the_refusal_matrix() {
        // `docs/clients.md`'s "Surfacing a page that stopped syncing"
        // table used to be hand-maintained prose — nothing regenerated
        // it and no test pinned it, which is exactly why the MCP row
        // went missing for as long as it did (RFC 0255). This test is
        // the projection, mirroring
        // `outl_shortcuts::support::the_parity_doc_matches_the_code`.
        let expected = refusal_matrix_markdown();

        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/clients.md");
        let doc =
            std::fs::read_to_string(path).unwrap_or_else(|e| panic!("cannot read {path}: {e}"));

        let begin = "<!-- BEGIN GENERATED: refusal-matrix -->";
        let end = "<!-- END GENERATED: refusal-matrix -->";
        let begin_at = doc
            .find(begin)
            .unwrap_or_else(|| panic!("{path} has no {begin} marker"));
        let content_start = begin_at + begin.len();
        let end_at = doc[content_start..]
            .find(end)
            .unwrap_or_else(|| panic!("{path} has no {end} marker"))
            + content_start;

        // Regenerate rather than hand-edit:
        //     OUTL_UPDATE_CLIENTS_DOC=1 cargo test -p outl-actions refusal::
        if std::env::var_os("OUTL_UPDATE_CLIENTS_DOC").is_some() {
            let mut updated = String::new();
            updated.push_str(&doc[..begin_at]);
            updated.push_str(begin);
            updated.push_str("\n\n");
            updated.push_str(expected.trim());
            updated.push('\n');
            updated.push_str(end);
            updated.push_str(&doc[end_at + end.len()..]);
            std::fs::write(path, updated).expect("cannot write docs/clients.md");
            return;
        }

        let in_doc = doc[content_start..end_at].trim();
        assert_eq!(
            in_doc,
            expected.trim(),
            "docs/clients.md's {begin} … {end} region is out of date \n\
             — regenerate with `OUTL_UPDATE_CLIENTS_DOC=1 cargo test -p outl-actions refusal::`",
        );
    }
}
