//! Hybrid Logical Clock.
//!
//! The total order is `(physical_ms, logical_counter, actor)` lexicographic.
//! Actor is the final tiebreak so concurrent ops from different replicas
//! cannot sort identically. See `docs/crdt.md`.

use crate::id::ActorId;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// A Hybrid Logical Clock timestamp.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Hlc {
    /// Wall-clock component in milliseconds since the Unix epoch.
    pub physical_ms: u64,
    /// Logical counter; increments when two physical components collide.
    pub logical: u32,
    /// Producer of the op; final tiebreak.
    pub actor: ActorId,
}

impl Hlc {
    /// Compose an HLC from its parts.
    pub fn new(physical_ms: u64, logical: u32, actor: ActorId) -> Self {
        Self {
            physical_ms,
            logical,
            actor,
        }
    }
}

impl PartialOrd for Hlc {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Hlc {
    fn cmp(&self, other: &Self) -> Ordering {
        self.physical_ms
            .cmp(&other.physical_ms)
            .then(self.logical.cmp(&other.logical))
            .then(self.actor.0.cmp(&other.actor.0))
    }
}

/// Generates monotonic HLCs for one actor.
///
/// `next()` is cheap and safe to call concurrently via `Arc`. `observe()`
/// folds in a remote timestamp so future local ops are guaranteed to sort
/// after anything we've seen.
#[derive(Debug, Clone)]
pub struct HlcGenerator {
    actor: ActorId,
    state: Arc<Mutex<State>>,
}

#[derive(Debug, Clone, Copy)]
struct State {
    physical_ms: u64,
    logical: u32,
}

impl HlcGenerator {
    /// Build a fresh generator with logical counter at zero.
    pub fn new(actor: ActorId) -> Self {
        Self {
            actor,
            state: Arc::new(Mutex::new(State {
                physical_ms: 0,
                logical: 0,
            })),
        }
    }

    /// Build a generator at a given starting state. Useful for tests.
    pub fn with_state(actor: ActorId, physical_ms: u64, logical: u32) -> Self {
        Self {
            actor,
            state: Arc::new(Mutex::new(State {
                physical_ms,
                logical,
            })),
        }
    }

    /// Actor id of this generator.
    pub fn actor(&self) -> ActorId {
        self.actor
    }

    /// Produce the next monotonic HLC.
    pub fn next(&self) -> Hlc {
        let mut s = self.state.lock();
        tick(&mut s, self.actor)
    }

    /// Raise the clock to at least `ts` **without** producing a timestamp.
    ///
    /// [`Self::new`] starts at `physical_ms: 0` and [`Self::next`] is
    /// monotonic only against that in-memory state, which is rebuilt from
    /// nothing on every boot. So a generator knows nothing about the ops
    /// already on disk, and after the wall clock moves *backwards* — an
    /// NTP correction, a timezone-confused VM, a dual-boot, a restored
    /// backup — `next()` happily issues timestamps that sort **before**
    /// ops the log already holds.
    ///
    /// That is not a convergence bug: `apply_op` reorders to the same
    /// materialized tree either way, which is exactly what the CRDT is
    /// for. It is a *cost* bug. Each such op forces the paper's undo/redo
    /// window over every newer entry, and on a 217k-op log one late op
    /// measured ~74 ms — paid on a foreground keystroke. The maintainer's
    /// real workspace carries 11 such rollback events, the largest 2.7
    /// days wide. It also writes out-of-order lines that
    /// [`crate::log::OpLog::append`]'s ordering `debug_assert` exists to
    /// catch.
    ///
    /// Seeding from the log's maximum at boot removes the whole class.
    /// Callers should not hand-roll it — use
    /// [`Workspace::seed_clock`](crate::workspace::Workspace::seed_clock),
    /// which reads the authoritative per-actor maximum and calls this.
    pub fn seed(&self, ts: Hlc) {
        let mut s = self.state.lock();
        raise_to(&mut s, ts);
    }

    /// Fold a remote HLC into the local clock, then return a fresh HLC
    /// guaranteed to sort after both the previous local state and the
    /// observed remote.
    ///
    /// Exactly [`Self::seed`] followed by [`Self::next`], but under one
    /// lock acquisition so a concurrent `next()` cannot slip between the
    /// two and consume the tick this call is entitled to.
    pub fn observe(&self, remote: Hlc) -> Hlc {
        let mut s = self.state.lock();
        raise_to(&mut s, remote);
        tick(&mut s, self.actor)
    }
}

/// Advance `s` by one tick and return the resulting timestamp.
///
/// The tick rule is shared by `next` and `observe` so the two cannot
/// drift: take the wall clock when it is ahead (resetting the logical
/// counter), otherwise bump the counter.
///
/// **The counter carries instead of saturating.** A saturating bump was
/// safe while `logical` could only grow inside a single wall-clock
/// millisecond — `u32::MAX` was not reachable in one millisecond of
/// keystrokes. [`HlcGenerator::seed`] made it reachable: the counter is
/// now raised from a `u32` read off the op log. Pinned at the ceiling
/// with `physical_ms` at or ahead of the wall clock, `next()` returns the
/// *same* `Hlc` forever, and `Workspace::apply` dedups every op carrying
/// an already-seen `ts` and returns `Ok(())` without persisting — silent,
/// total write loss reported as success.
///
/// Carrying preserves the ordering contract: `(p, u32::MAX)` is the
/// largest timestamp issuable at `p`, and `(p + 1, 0)` sorts strictly
/// after it on the lexicographic order, so the tick still dominates
/// everything this generator has ever issued.
fn tick(s: &mut State, actor: ActorId) -> Hlc {
    let now = wall_clock_ms();
    if now > s.physical_ms {
        s.physical_ms = now;
        s.logical = 0;
    } else if let Some(bumped) = s.logical.checked_add(1) {
        s.logical = bumped;
    } else {
        // `saturating_add` on the physical component is the same pin one
        // level up, but at `u64::MAX` milliseconds (year ~584 million)
        // there is no larger timestamp to issue and no caller to report
        // to. The reachable case is the logical one, and it carries.
        s.physical_ms = s.physical_ms.saturating_add(1);
        s.logical = 0;
    }
    Hlc::new(s.physical_ms, s.logical, actor)
}

/// Raise `s` so the next tick is guaranteed to sort strictly after `ts`.
///
/// Shared by [`HlcGenerator::seed`] and [`HlcGenerator::observe`] — the
/// two used to carry separate copies of this rule, and a second copy of
/// "what does it mean to have seen this timestamp" is the kind of thing
/// that drifts silently, because both readings converge and only the op
/// ordering differs.
fn raise_to(s: &mut State, ts: Hlc) {
    match ts.physical_ms.cmp(&s.physical_ms) {
        Ordering::Greater => {
            s.physical_ms = ts.physical_ms;
            s.logical = ts.logical;
        }
        Ordering::Equal => s.logical = s.logical.max(ts.logical),
        Ordering::Less => {}
    }
}

/// How far ahead of local wall-clock time a timestamp read off disk is
/// allowed to raise this device's clock.
///
/// The same 24h window `outl-sync-iroh` uses to drop an incoming op with a
/// future HLC. It is declared here, in the leaf module that owns the
/// timestamp type, because `outl-sync-iroh` depends on `outl-core` and not
/// the other way round — the number cannot live in the transport and still
/// be reachable from the seeding path.
pub const MAX_CLOCK_SKEW_MS: u64 = 24 * 60 * 60 * 1_000;

/// Local wall clock in milliseconds since the Unix epoch, or `None` when
/// the system clock is set before the epoch.
///
/// The fallible form exists because the two callers want opposite
/// fallbacks. [`tick`] wants a number and treats an unreadable clock as
/// "not ahead of us" (`0`), which is harmless: it just bumps the logical
/// counter. A caller deciding a *ceiling* cannot do that — `0` would put
/// the ceiling in January 1970 and clamp away every legitimate seed.
pub(crate) fn wall_clock_ms_checked() -> Option<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .ok()
}

fn wall_clock_ms() -> u64 {
    wall_clock_ms_checked().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordering_is_lexicographic_on_physical_logical_actor() {
        let a = ActorId::new();
        let b = ActorId::new();
        let h1 = Hlc::new(10, 0, a);
        let h2 = Hlc::new(10, 0, b);
        let h3 = Hlc::new(10, 1, a);
        let h4 = Hlc::new(11, 0, a);
        // Different actor breaks tie.
        assert_ne!(h1.cmp(&h2), Ordering::Equal);
        // Logical wins over actor.
        assert!(h1 < h3);
        // Physical wins over everything.
        assert!(h3 < h4);
    }

    #[test]
    fn generator_is_monotonic() {
        let actor = ActorId::new();
        let g = HlcGenerator::with_state(actor, 0, 0);
        let mut last = g.next();
        for _ in 0..100 {
            let next = g.next();
            assert!(next > last, "HLC went backwards: {last:?} → {next:?}");
            last = next;
        }
    }

    #[test]
    fn observe_advances_clock_past_remote() {
        let me = ActorId::new();
        let them = ActorId::new();
        let g = HlcGenerator::with_state(me, 0, 0);
        let remote = Hlc::new(1_000_000_000_000, 5, them);
        let after = g.observe(remote);
        assert!(after > remote, "observed HLC must dominate remote");
    }

    #[test]
    fn observe_is_exactly_seed_then_next() {
        // `observe` used to carry its own copy of the "raise the clock to
        // at least this timestamp" rule; it now shares `raise_to` with
        // `seed`. This pins the refactor: the two spellings must agree on
        // every branch of the old four-arm match — remote ahead, local
        // ahead, equal physical with either logical ahead, and remote far
        // in the past.
        let me = ActorId::new();
        let them = ActorId::new();
        let cases = [
            (0u64, 0u32, 2_000_000_000_000u64, 5u32),
            (2_000_000_000_000, 7, 1_000_000_000_000, 5),
            (2_000_000_000_000, 7, 2_000_000_000_000, 5),
            (2_000_000_000_000, 3, 2_000_000_000_000, 9),
            (2_000_000_000_000, 3, 1, 0),
        ];

        for (lp, ll, rp, rl) in cases {
            let remote = Hlc::new(rp, rl, them);

            let via_observe = HlcGenerator::with_state(me, lp, ll).observe(remote);

            let split = HlcGenerator::with_state(me, lp, ll);
            split.seed(remote);
            let via_seed_then_next = split.next();

            assert_eq!(
                via_observe, via_seed_then_next,
                "observe and seed+next disagreed for local=({lp},{ll}) remote=({rp},{rl})"
            );
        }
    }

    #[test]
    fn seeding_never_rewinds_the_clock() {
        let me = ActorId::new();
        let them = ActorId::new();
        let g = HlcGenerator::with_state(me, 5_000, 9);
        // A timestamp strictly below the current state must not lower it.
        g.seed(Hlc::new(1, 0, them));
        let next = g.next();
        assert!(
            next >= Hlc::new(5_000, 10, me),
            "a stale seed rewound the clock to {next:?}"
        );
    }

    #[test]
    fn a_seeded_generator_dominates_the_timestamp_it_was_seeded_with() {
        let me = ActorId::new();
        let them = ActorId::new();
        // Far future stands in for "the wall clock is behind the log",
        // which is the condition seeding exists to survive.
        let ahead = Hlc::new(wall_clock_ms() + MAX_CLOCK_SKEW_MS, 3, them);
        let g = HlcGenerator::with_state(me, 0, 0);
        g.seed(ahead);
        let mut last = g.next();
        assert!(last > ahead, "seeded generator must dominate its seed");
        for _ in 0..50 {
            let next = g.next();
            assert!(next > last, "monotonicity broke after seeding");
            last = next;
        }
    }

    #[test]
    fn the_logical_counter_carries_into_the_physical_component_instead_of_pinning() {
        // Before seeding existed, `logical` only ever grew inside a single
        // wall-clock millisecond, so `u32::MAX` was unreachable and a
        // saturating bump was harmless. `seed` now raises the counter from a
        // `u32` read off the op log, so the ceiling is reachable — and a
        // saturating counter pins `next()` on one timestamp forever while the
        // wall clock is behind `physical_ms`. `Workspace::apply` dedups by
        // `ts`, so every later local write is then dropped and reported as
        // `Ok(())`: silent, total write loss.
        let me = ActorId::new();
        let future = wall_clock_ms() + 3_600_000;
        let g = HlcGenerator::with_state(me, future, u32::MAX - 2);

        let mut last = g.next();
        for _ in 0..5 {
            let next = g.next();
            assert!(
                next > last,
                "HLC stopped advancing at the u32 ceiling: {last:?} -> {next:?}"
            );
            last = next;
        }
        assert!(
            last.physical_ms > future,
            "the carry must reach the physical component, got {last:?}"
        );
    }
}
