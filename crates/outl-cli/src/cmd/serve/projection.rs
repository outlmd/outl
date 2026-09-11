//! `outl serve`'s `tree → .md` half: the projection sweep and what it
//! says out loud.
//!
//! The `.md → tree` direction has had a permanent executor for as long
//! as this daemon has existed — the file watcher next door. This one had
//! none, so ops arriving by sync left a page's `.md` wrong until a human
//! happened to open it. `outl_actions::journal::reproject_stale_pages`
//! is the executor; this module decides when it runs and when its
//! findings are worth a log line.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use outl_actions::journal::{reproject_stale_pages, ReprojectionSweep};
use outl_core::workspace::Workspace;
use tracing::{info, warn};

/// Floor on how often the `tree → .md` sweep runs while the daemon is
/// up.
///
/// The sweep renders every page to compare it with disk, which on a
/// 2.5k-page graph is seconds of work. Peer ops arrive in bursts, so
/// without a floor a catch-up sync would render the whole graph once per
/// batch. A projection lagging by half a minute is invisible — the op
/// log already holds the truth and every client reads it, not the `.md`.
pub(super) const PROJECTION_MIN_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);

/// Cap on how many pages a projection summary names individually.
const MAX_NAMED_PAGES: usize = 20;

/// Run the `tree → .md` sweep and report it.
///
/// **Why the daemon does this at all.** The `.md → tree` direction has
/// had a permanent executor for as long as `outl serve` has existed —
/// the watcher below. The other direction had none: ops arriving by
/// sync, or a render-affecting fix landing in the renderer, left a
/// page's `.md` wrong until a human happened to open it or happened to
/// run a maintenance command they had no reason to run. Measured on a
/// real 2,574-page workspace: 704 pages stale, 702 of them re-projecting
/// with **zero** content lines removed.
///
/// **Why it cannot become a reconcile.** Every write here replaces the
/// `.md` *and* its sidecar from one tree snapshot, so the watcher event
/// it generates reconciles to zero ops — `needs_reconcile` compares the
/// sidecar hash against the file and they agree by construction. A
/// projection this pass writes can therefore never be read back as an
/// external edit, which is the only way a `tree → .md` write could turn
/// into a `Delete`.
///
/// **Why it does not need a backup.** It writes only pages whose
/// re-projection removes nothing from disk
/// (`outl_actions::journal::reproject_stale_pages`). Anything that would
/// remove a content line is withheld for `outl doctor --repair`, which
/// copies every file into `.outl/repair-backup/` first and applies
/// volume ceilings.
pub(super) fn project_stale_pages(ws: &Workspace, root: &Path, reporter: &mut ProjectionReporter) {
    let sweep = reproject_stale_pages(ws, root);
    for line in reporter.observe(&sweep) {
        warn!("{line}");
    }
    if !sweep.written.is_empty() {
        info!(
            "re-projected {} stale page(s) from the op log ({} page(s) surveyed)",
            sweep.written.len(),
            sweep.surveyed
        );
    }
}

/// Decides when a projection sweep's findings are worth saying out
/// loud, and says them.
///
/// **Why a daemon needs this and a command does not.** An interactive
/// `outl doctor` runs because somebody asked, so it prints everything it
/// found and the user is reading. `serve` sweeps every
/// [`PROJECTION_MIN_INTERVAL`] and a frozen page stays frozen until a
/// human runs `outl reconcile --ahead-of-log` — which may be days. Naming
/// it 2,880 times a day is not "reaching the user" (root `CLAUDE.md`
/// invariant 8); it is the same silence with more lines, because nobody
/// reads a log that repeats.
///
/// So the signal is the **change**. The set of pages needing attention
/// — refused, withheld, declined — is announced in full when it is
/// first seen and again whenever it differs, and not otherwise. When it
/// empties, that is said once: `docs/clients.md` makes the same point
/// about the GUI banner, that a warning outliving its condition is the
/// mirror of the silence it was added to end.
///
/// One mechanism for all three categories on purpose. They differ in
/// what the user should do about them, not in how often they should
/// hear about them.
#[derive(Default)]
pub(super) struct ProjectionReporter {
    /// The attention set as of the last sweep, or `None` before the
    /// first. `Some(empty)` and `None` are different states: the first
    /// means "we looked and it was clean", which is what makes the
    /// clearing line fire exactly once.
    reported: Option<BTreeSet<PathBuf>>,
}

impl ProjectionReporter {
    /// Record `sweep` and return the lines it changed.
    pub(super) fn observe(&mut self, sweep: &ReprojectionSweep) -> Vec<String> {
        let attention: BTreeSet<PathBuf> = sweep
            .refused
            .iter()
            .map(|f| f.path.clone())
            .chain(sweep.withheld.iter().map(|w| w.path.clone()))
            .chain(sweep.declined.iter().cloned())
            .chain(sweep.unreadable.iter().map(|u| u.path.clone()))
            .collect();
        if self.reported.as_ref() == Some(&attention) {
            return Vec::new();
        }
        let had_any = self.reported.as_ref().is_some_and(|prev| !prev.is_empty());
        self.reported = Some(attention.clone());
        if attention.is_empty() {
            return if had_any {
                vec![
                    "every page that needed attention is converging again — nothing is \
                     held back from the op log"
                        .to_string(),
                ]
            } else {
                Vec::new()
            };
        }
        projection_summary(sweep)
    }
}

/// Format one sweep's findings, in full.
///
/// Split from [`ProjectionReporter`] so the two questions stay separate
/// and separately testable: *what* is worth saying, and *when*.
fn projection_summary(sweep: &ReprojectionSweep) -> Vec<String> {
    let mut out = Vec::new();
    for failure in sweep.refused.iter().take(MAX_NAMED_PAGES) {
        out.push(format!(
            "{}: stopped syncing — {}. Run `outl reconcile --ahead-of-log` to bring those \
             lines into the op log",
            failure.path.display(),
            failure.error
        ));
    }
    if sweep.refused.len() > MAX_NAMED_PAGES {
        out.push(format!(
            "… and {} more page(s) that stopped syncing",
            sweep.refused.len() - MAX_NAMED_PAGES
        ));
    }
    for page in sweep.withheld.iter().take(MAX_NAMED_PAGES) {
        out.push(format!(
            "{}: left stale — re-projecting it would remove {} content line(s) from disk, \
             which is `outl doctor --repair`'s call to make (it backs the file up first)",
            page.path.display(),
            page.lines_removed
        ));
    }
    if sweep.withheld.len() > MAX_NAMED_PAGES {
        out.push(format!(
            "… and {} more page(s) whose re-projection would remove content",
            sweep.withheld.len() - MAX_NAMED_PAGES
        ));
    }
    for path in sweep.declined.iter().take(MAX_NAMED_PAGES) {
        out.push(format!(
            "{}: projection declined after the survey selected it — the file changed \
             underneath, or its sidecar cannot vouch for it",
            path.display()
        ));
    }
    if sweep.declined.len() > MAX_NAMED_PAGES {
        out.push(format!(
            "… and {} more declined page(s)",
            sweep.declined.len() - MAX_NAMED_PAGES
        ));
    }
    // A page whose bytes could not be looked at — an I/O error, or an
    // iCloud file still in the cloud. It is not re-projected and it is
    // not a failure, so without this it is the one outcome the daemon
    // has no word for, and `ReprojectionSweep::declined`'s own doc says
    // a skip nobody can see is what invariant 8 exists to prevent.
    for page in sweep.unreadable.iter().take(MAX_NAMED_PAGES) {
        out.push(format!(
            "{}: not re-projected — {}",
            page.path.display(),
            page.reason
        ));
    }
    if sweep.unreadable.len() > MAX_NAMED_PAGES {
        out.push(format!(
            "… and {} more page(s) the sweep could not read",
            sweep.unreadable.len() - MAX_NAMED_PAGES
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use outl_actions::journal::{ProjectionFailure, ReprojectionSweep, WithheldPage};
    use std::path::PathBuf;

    fn frozen(path: &str, lines: usize) -> ProjectionFailure {
        ProjectionFailure {
            path: PathBuf::from(path),
            error: outl_actions::ActionError::PageMarkdownAheadOfLog {
                path: path.into(),
                lines,
                sample: "\"only ever on disk\"".into(),
            },
        }
    }

    /// Invariant 8: a page that stopped syncing must be **named**, not
    /// folded into a count. A daemon that swallows the refusal ships a
    /// page that silently stopped converging — the failure the guard
    /// exists to prevent, moved one layer up.
    #[test]
    fn the_first_sweep_names_every_page_that_stopped_syncing() {
        let mut reporter = ProjectionReporter::default();
        let sweep = ReprojectionSweep {
            surveyed: 3,
            refused: vec![frozen("/ws/pages/frozen.md", 4)],
            ..Default::default()
        };

        let joined = reporter.observe(&sweep).join("\n");

        assert!(
            joined.contains("/ws/pages/frozen.md"),
            "the page must be named: {joined}"
        );
        assert!(
            joined.contains("outl reconcile --ahead-of-log"),
            "the recovery command must be named: {joined}"
        );
    }

    /// A daemon sweeps every 30s and a frozen page stays frozen until a
    /// human runs the recovery. Re-naming it 2,880 times a day is not
    /// "reaching the user" — it is the same silence with more lines,
    /// because nobody reads a log that repeats. The signal is the
    /// *change*.
    #[test]
    fn an_unchanged_attention_set_is_not_repeated() {
        let mut reporter = ProjectionReporter::default();
        let sweep = || ReprojectionSweep {
            surveyed: 3,
            refused: vec![frozen("/ws/pages/frozen.md", 4)],
            ..Default::default()
        };

        assert!(!reporter.observe(&sweep()).is_empty(), "first sweep speaks");
        assert!(
            reporter.observe(&sweep()).is_empty(),
            "the same set must not be announced twice"
        );
        assert!(reporter.observe(&sweep()).is_empty());
    }

    /// The mirror: going quiet must not make the reporter deaf. A page
    /// that starts failing after a clean sweep is new information.
    #[test]
    fn a_page_that_starts_failing_after_a_quiet_sweep_is_named() {
        let mut reporter = ProjectionReporter::default();
        reporter.observe(&ReprojectionSweep {
            surveyed: 3,
            ..Default::default()
        });

        let joined = reporter
            .observe(&ReprojectionSweep {
                surveyed: 3,
                refused: vec![frozen("/ws/pages/newly-frozen.md", 1)],
                ..Default::default()
            })
            .join("\n");

        assert!(joined.contains("/ws/pages/newly-frozen.md"), "{joined}");
    }

    /// `docs/clients.md`: "A banner that outlives the condition is the
    /// mirror of the silence this section exists to end." The same rule
    /// applies to a log line — the user who ran the recovery is entitled
    /// to see it worked, once.
    #[test]
    fn a_cleared_attention_set_is_announced_once() {
        let mut reporter = ProjectionReporter::default();
        reporter.observe(&ReprojectionSweep {
            surveyed: 3,
            refused: vec![frozen("/ws/pages/frozen.md", 4)],
            ..Default::default()
        });

        let cleared = reporter.observe(&ReprojectionSweep {
            surveyed: 3,
            ..Default::default()
        });
        assert!(
            cleared.join("\n").contains("converging again"),
            "the recovery must be acknowledged: {cleared:?}"
        );
        assert!(
            reporter
                .observe(&ReprojectionSweep {
                    surveyed: 3,
                    ..Default::default()
                })
                .is_empty(),
            "and only once"
        );
    }

    /// A withheld page is not a failure and not a success: it is work
    /// deliberately left to the command that backs up first.
    #[test]
    fn the_summary_points_a_withheld_page_at_the_command_that_backs_up() {
        let mut reporter = ProjectionReporter::default();
        let sweep = ReprojectionSweep {
            surveyed: 2,
            withheld: vec![WithheldPage {
                path: PathBuf::from("/ws/pages/shrunk.md"),
                lines_removed: 3,
            }],
            ..Default::default()
        };

        let joined = reporter.observe(&sweep).join("\n");

        assert!(joined.contains("/ws/pages/shrunk.md"), "{joined}");
        assert!(
            joined.contains("3"),
            "the line count must be stated: {joined}"
        );
        assert!(
            joined.contains("outl doctor"),
            "the backed-up path must be named: {joined}"
        );
    }

    /// An all-quiet first sweep says nothing at all. A daemon that logs
    /// a line per tick trains the user to stop reading its output, and
    /// the lines that matter are the ones above.
    #[test]
    fn the_summary_is_silent_when_nothing_happened() {
        let mut reporter = ProjectionReporter::default();
        let sweep = ReprojectionSweep {
            surveyed: 900,
            ..Default::default()
        };
        assert!(reporter.observe(&sweep).is_empty());
    }
}
