//! Where a workspace open spent its time.
//!
//! The app's premise is that its text input is live when it opens, and
//! that premise is a number. Until this existed the number could only be
//! taken on a developer's Mac against a copy of a workspace, which
//! answers a different question: a phone's CPU, its storage and above
//! all its memory ceiling are what decide whether a launch is fast or
//! gets killed, and none of the three are reproducible off-device.
//!
//! So the open reports its own breakdown, once per open, at `info`.
//! It costs one `Instant::now()` per phase and a single line, which is
//! cheap enough to leave on in release — a measurement you have to
//! rebuild to collect is a measurement nobody takes on the device where
//! it matters.
//!
//! Read it on a simulator or a tethered phone with:
//!
//! ```text
//! xcrun simctl spawn <udid> log stream --predicate 'eventMessage CONTAINS "boot phases"'
//! idevicesyslog -u <udid> | grep "boot phases"
//! ```

use std::time::Instant;

use tracing::info;

/// Accumulates one `(name, millis)` pair per phase of a workspace open.
pub struct BootTimer {
    started: Instant,
    mark: Instant,
    phases: Vec<(&'static str, u128)>,
}

impl BootTimer {
    /// Start timing. The first [`Self::phase`] measures from here.
    pub fn start() -> Self {
        let now = Instant::now();
        Self {
            started: now,
            mark: now,
            phases: Vec::new(),
        }
    }

    /// Close the phase that ends here and open the next one.
    ///
    /// Phases are recorded even when they measure zero: a phase missing
    /// from the line reads as "did not run", and a reader cannot tell
    /// that from "was not instrumented".
    pub fn phase(&mut self, name: &'static str) {
        let now = Instant::now();
        self.phases
            .push((name, now.duration_since(self.mark).as_millis()));
        self.mark = now;
    }

    /// Emit the breakdown. `detail` carries whatever the caller knows
    /// that explains the numbers — on a snapshot boot, how many ops the
    /// delta replayed, since that is the one input that moves them.
    pub fn finish(self, detail: &str) {
        let total = self.started.elapsed().as_millis();
        let phases = self
            .phases
            .iter()
            .map(|(name, ms)| format!("{name}={ms}ms"))
            .collect::<Vec<_>>()
            .join(" ");
        info!("boot phases: total={total}ms {phases} {detail}");
    }
}
