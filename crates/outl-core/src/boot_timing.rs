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

use std::sync::{Mutex, OnceLock};
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

// ---------------------------------------------------------------------------
// Client-side marks
// ---------------------------------------------------------------------------

/// When this process started, as close to `main` as the binary sets it.
///
/// A client mark is only comparable to a `boot phases` line if both count
/// from the same zero. [`BootTimer`] starts when the workspace open
/// starts; a webview's `html_parsed` lands *before* that on a cold launch
/// and after it on a workspace switch, so it has to count from the
/// process instead.
static PROCESS_START: OnceLock<Instant> = OnceLock::new();

/// Marks reported so far, in arrival order.
static CLIENT_MARKS: OnceLock<Mutex<Vec<(String, u128)>>> = OnceLock::new();

/// The mark a client reports last. Reaching it emits the summary line.
const LAST_MARK: &str = "interactive";

/// Record the moment this process started. Call it from `main`, first
/// statement, before any work.
///
/// Idempotent, and the first call wins: a binary that forgets gets the
/// first [`mark_client_phase`] as its zero instead, which under-reports
/// rather than panicking. Under-reporting is the right failure here — a
/// missing zero should not be able to make a slow launch look fast *and*
/// take the process down.
pub fn note_process_start() {
    let _ = PROCESS_START.set(Instant::now());
}

fn process_start() -> Instant {
    *PROCESS_START.get_or_init(Instant::now)
}

/// Milliseconds since [`note_process_start`].
pub fn since_process_start_ms() -> u128 {
    process_start().elapsed().as_millis()
}

/// Record a client-reported boot milestone.
///
/// The four a GUI client reports are `html_parsed`, `theme_applied`,
/// `first_content` and `interactive` — the frontend half of a launch,
/// which no Rust-side timer can see. Each is logged on arrival so a
/// launch that never reaches the last one still says how far it got, and
/// `interactive` additionally emits the whole sequence on one line, in
/// the same shape [`BootTimer::finish`] uses.
///
/// Cheap by construction: one `Instant::elapsed`, one short lock, one
/// `info!`. It is on in release for the reason the rest of this module
/// is — a measurement you have to rebuild to collect is one nobody takes
/// on the device where it matters.
///
/// A poisoned lock drops the mark rather than propagating: losing a
/// measurement must never be able to fail a boot.
pub fn mark_client_phase(name: &str) {
    let at = since_process_start_ms();
    info!("boot mark: {name}={at}ms");
    let marks = CLIENT_MARKS.get_or_init(|| Mutex::new(Vec::new()));
    let Ok(mut marks) = marks.lock() else {
        return;
    };
    marks.push((name.to_string(), at));
    if name != LAST_MARK {
        return;
    }
    let line = marks
        .iter()
        .map(|(name, at)| format!("{name}={at}ms"))
        .collect::<Vec<_>>()
        .join(" ");
    info!("boot marks: {line}");
}

/// Every mark reported so far, as `(name, ms since process start)`.
///
/// For a test or a diagnostic command; the production reader is the log.
pub fn client_marks() -> Vec<(String, u128)> {
    CLIENT_MARKS
        .get()
        .and_then(|m| m.lock().ok())
        .map(|m| m.clone())
        .unwrap_or_default()
}
