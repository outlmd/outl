//! Seeding the local HLC generator from the op log.
//!
//! [`HlcGenerator`](crate::hlc::HlcGenerator) is monotonic against its own
//! in-memory state, and that state is built from nothing on every boot —
//! `physical_ms: 0`, then whatever the wall clock says. Nothing connects
//! it to the ops already on disk.
//!
//! So a backwards wall-clock movement between two runs (NTP correction,
//! a VM resuming with a stale clock, a restored backup, a dual-boot) lets
//! the generator issue timestamps that sort *below* ops the log already
//! holds. The CRDT absorbs that correctly — `apply_op` reorders and every
//! replica still converges — so this module fixes a **cost**, not a
//! correctness property, and it should not be described as a sync fix.
//!
//! The cost is real: each such op forces the paper's undo/redo window
//! over every newer log entry, measured at ~74 ms for a single late op on
//! a 217,811-op log, paid synchronously on a foreground edit. The
//! maintainer's workspace carries 11 rollback events, the widest 2.7 days.
//!
//! This lives beside `Workspace` rather than in `hlc.rs` because the
//! authoritative answer to "what is the highest timestamp we have already
//! written" is a *storage* question, and `hlc.rs` is a leaf module that
//! must not learn about storage.

use super::{Workspace, WorkspaceError};
use crate::hlc::{wall_clock_ms_checked, Hlc, HlcGenerator, MAX_CLOCK_SKEW_MS};
use tracing::warn;

impl Workspace {
    /// The highest [`Hlc`] this workspace has ever recorded, across every
    /// actor and every storage shard.
    ///
    /// Derived from the per-actor maxima the snapshot cutoff already uses,
    /// so it is driven off the on-disk index rather than a log scan, and
    /// it is correct on **both** boot paths. That second part is the
    /// subtle one: after a snapshot boot the resident log holds only the
    /// post-cutoff delta, so `self.log().last()` would report a maximum
    /// far below the truth and seed the clock too low — which is the very
    /// failure this is meant to prevent, reintroduced by using the
    /// convenient source instead of the correct one.
    ///
    /// `None` for a workspace that holds no ops at all.
    pub fn max_known_hlc(&self) -> Result<Option<Hlc>, WorkspaceError> {
        Ok(self.last_ts_per_actor_combined()?.into_values().max())
    }

    /// Raise `hlc` so every timestamp it issues sorts after everything
    /// this workspace has already written.
    ///
    /// Call this once, immediately after opening a workspace, at every
    /// site that pairs a [`HlcGenerator`] with a [`Workspace`]. It is a
    /// no-op on an empty workspace and cheap on a large one.
    ///
    /// Deliberately a method on `Workspace` rather than something each
    /// caller assembles from `max_known_hlc` + `HlcGenerator::seed`:
    /// there are several open sites across `outl-ws`, the TUI, the CLI
    /// and both Tauri clients, and "every caller remembers to do the same
    /// two steps" is the shape this repo has already paid for more than
    /// once.
    pub fn seed_clock(&self, hlc: &HlcGenerator) -> Result<(), WorkspaceError> {
        if let Some(max) = self.max_known_hlc()? {
            hlc.seed(clamp_to_skew_window(max));
        }
        Ok(())
    }
}

/// Lower `max` to `now + `[`MAX_CLOCK_SKEW_MS`]` when it sits beyond that
/// ceiling, so one bad line in the op log cannot raise this device's clock
/// into the far future.
///
/// **Why the ceiling has to exist here.** [`Workspace::max_known_hlc`] folds
/// the maximum of *every* actor's log into this device's generator, and the
/// seeded value is then stamped onto this device's own ops and appended to
/// `ops-<own>.jsonl` — where the next boot reads it back as the new maximum.
/// A single corrupt or hostile `physical_ms` is therefore absorbed on the
/// first boot that sees it and pinned there permanently, irreversibly for
/// that workspace. `outl-sync-iroh` drops an incoming op more than
/// [`MAX_CLOCK_SKEW_MS`] ahead for the same reason, but the file transports
/// (iCloud / Syncthing / shared FS — a documented, shipping mode) never pass
/// through that gate, so before the clamp the only thing keeping such a line
/// inert was that nothing read the log's maximum.
///
/// **Why clamping is safe.** It only ever *lowers* how far the clock is
/// raised, and [`HlcGenerator::seed`] is monotone against the generator's own
/// state (a seed below it is ignored) while `next()` is monotone against that
/// same state. So clamping can never rewind a clock, never issue a duplicate,
/// and never make a local op sort below one this generator already produced.
/// The worst it can do is leave the clock *lower* than a far-future op in the
/// log — which puts us back in the pre-seeding world for that one workspace,
/// a reorder cost the CRDT absorbs, not a divergence. Seeding exists to avoid
/// that cost, and it keeps doing so for every value inside the window.
///
/// A clock we cannot read cannot anchor "the future", so the seed passes
/// through unclamped rather than being judged against a ceiling of `0` —
/// the same choice `outl-sync-iroh` makes for a pre-epoch clock. Seeding
/// raw is what this branch already did; refusing to seed is not obviously
/// better and is a second behaviour to reason about.
fn clamp_to_skew_window(max: Hlc) -> Hlc {
    let Some(now) = wall_clock_ms_checked() else {
        warn!("local clock is before UNIX_EPOCH; seeding the HLC generator unclamped");
        return max;
    };
    let ceiling = now.saturating_add(MAX_CLOCK_SKEW_MS);
    if max.physical_ms <= ceiling {
        return max;
    }
    warn!(
        ts = ?max,
        "op log holds a timestamp {}ms beyond the {MAX_CLOCK_SKEW_MS}ms skew window; \
         clamping the HLC seed (the op itself is untouched)",
        max.physical_ms - now,
    );
    Hlc::new(ceiling, 0, max.actor)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::{ActorId, NodeId};
    use crate::op::{LogOp, Op};
    use crate::storage::MemoryStorage;

    fn ws_with(ops: Vec<LogOp>) -> Workspace {
        let actor = ActorId::new();
        let mut ws = Workspace::open_with_storage(actor, Box::new(MemoryStorage::default()), None)
            .expect("open workspace");
        for op in ops {
            ws.apply(op).expect("apply");
        }
        ws
    }

    fn now_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock after epoch")
            .as_millis() as u64
    }

    fn create_at(ms: u64, actor: ActorId, n: u8) -> LogOp {
        LogOp {
            ts: Hlc::new(ms, 0, actor),
            actor,
            op: Op::Create {
                node: NodeId::from_seed(b"clock-test:", &n.to_string()),
                parent: NodeId::root(),
                position: crate::fractional::Fractional::first(),
            },
        }
    }

    #[test]
    fn an_empty_workspace_has_no_maximum_and_seeding_it_is_a_noop() {
        let ws = ws_with(vec![]);
        assert_eq!(ws.max_known_hlc().expect("max"), None);

        let gen = HlcGenerator::with_state(ws.actor, 0, 0);
        ws.seed_clock(&gen).expect("seed");
        // Still usable, and still anchored to the wall clock.
        assert!(gen.next().physical_ms > 0);
    }

    #[test]
    fn the_maximum_spans_every_actor_not_just_our_own() {
        let ours = ActorId::new();
        let theirs = ActorId::new();
        let ws = ws_with(vec![create_at(500, ours, 1), create_at(9_000, theirs, 2)]);

        let max = ws.max_known_hlc().expect("max").expect("some");
        assert_eq!(max.physical_ms, 9_000);
    }

    #[test]
    fn a_seeded_clock_issues_timestamps_above_the_log_even_when_the_wall_clock_went_backwards() {
        // A timestamp far in the future stands in for "the wall clock has
        // since moved backwards relative to what the log already holds" —
        // the two are indistinguishable to the generator, and this is the
        // direction that is reproducible without touching the system clock.
        let actor = ActorId::new();
        // Inside the skew window on purpose: past it the seed is clamped
        // (see `a_far_future_timestamp_in_the_log_cannot_pin_the_local_clock_in_the_future`),
        // and a clamped seed is not what this test is about.
        let far_future = now_ms() + MAX_CLOCK_SKEW_MS / 2;
        let ws = ws_with(vec![create_at(far_future, actor, 1)]);

        let gen = HlcGenerator::with_state(actor, 0, 0);
        let unseeded = gen.next();
        assert!(
            unseeded.physical_ms < far_future,
            "without seeding the generator sorts below the log — this is the bug"
        );

        let gen = HlcGenerator::with_state(actor, 0, 0);
        ws.seed_clock(&gen).expect("seed");
        let seeded = gen.next();
        let max = ws.max_known_hlc().expect("max").expect("some");
        assert!(
            seeded > max,
            "seeded {seeded:?} must sort strictly after the log maximum {max:?}"
        );
    }

    #[test]
    fn seeding_twice_is_idempotent_and_never_lowers_the_clock() {
        let actor = ActorId::new();
        let ws = ws_with(vec![create_at(5_000, actor, 1)]);

        let gen = HlcGenerator::with_state(actor, 0, 0);
        ws.seed_clock(&gen).expect("seed once");
        let first = gen.next();
        ws.seed_clock(&gen).expect("seed twice");
        let second = gen.next();

        assert!(second > first, "a second seed must not rewind the clock");
    }

    #[test]
    fn a_far_future_timestamp_in_the_log_cannot_pin_the_local_clock_in_the_future() {
        // `max_known_hlc` folds the maximum of *every* actor's log into this
        // device's generator. The seeded value is then stamped onto this
        // device's own ops and written into `ops-<own>.jsonl`, where the next
        // boot reads it back as the new maximum — so a single corrupt or
        // hostile far-future `physical_ms` would pin the local clock in the
        // future permanently, and irreversibly for that workspace. The only
        // future-HLC gate in the repo lives in `outl-sync-iroh`, and the file
        // transports never pass through it.
        let actor = ActorId::new();
        let now = now_ms();
        let hostile = now + MAX_CLOCK_SKEW_MS * 30;
        let ws = ws_with(vec![create_at(hostile, actor, 1)]);

        let gen = HlcGenerator::with_state(actor, 0, 0);
        ws.seed_clock(&gen).expect("seed");
        let issued = gen.next();

        assert!(
            issued.physical_ms <= now_ms() + MAX_CLOCK_SKEW_MS,
            "a hostile log entry raised the clock to {issued:?}, past the \
             now + 24h ceiling"
        );
        // The log entry itself is untouched: clamping lowers how far the
        // clock is raised, it never rewrites history.
        assert_eq!(
            ws.max_known_hlc().expect("max").expect("some").physical_ms,
            hostile
        );
    }

    #[test]
    fn a_seed_inside_the_skew_window_is_not_clamped() {
        // The clamp must not eat the case seeding exists for: a log ahead of
        // the wall clock by a plausible amount (a peer's op, a backwards NTP
        // correction) still has to raise the local clock.
        let actor = ActorId::new();
        let ahead = now_ms() + MAX_CLOCK_SKEW_MS / 2;
        let ws = ws_with(vec![create_at(ahead, actor, 1)]);

        let gen = HlcGenerator::with_state(actor, 0, 0);
        ws.seed_clock(&gen).expect("seed");
        let issued = gen.next();
        // Exact, not `>=`: an *unconditional* clamp would raise this to the
        // ceiling and satisfy a `>=` assertion, so the loose form pinned
        // nothing. What must hold is that the seed is used as it stands.
        assert_eq!(
            issued.physical_ms, ahead,
            "a seed well inside the skew window was not used verbatim: {issued:?}"
        );
    }
}
