//! The one subject `outl doctor` inspects that is **not** inside the
//! workspace: the machine-global device store (`<device_dir>/actors/`).
//!
//! Two things this file never does. It forms no opinion of its own
//! about whether a binding is safe to drop — `outl_core`'s
//! `device/gc.rs` owns that verdict, and `--repair` re-asks it at write
//! time rather than trusting the plan built here. And it never opens
//! the store itself: the store is machine-global, so a pass that
//! resolved it would have every test in the battery judging, and
//! deleting from, the developer's own (root `CLAUDE.md` invariant 9,
//! third question).

use outl_core::device::{ActorBinding, DeviceStore, STALE_BINDING_TTL, STALE_SCRATCH_TTL};

use super::{Builder, Plan};

/// The one check whose subject is outside this workspace. The device
/// store is machine-global and has never had a GC, so a workspace the
/// user deleted keeps its actor binding forever (issue #211 item 3).
/// Reported here because `doctor` is the surface a user already runs,
/// and because the store's health is what decides whether the *next*
/// open of any workspace forks an actor.
/// An error here is an empty list, never a finding: the store is
/// outside this workspace, so a permission problem there says nothing
/// about this graph and must not fail an otherwise-clean run.
pub(super) fn check(b: &mut Builder, store: &DeviceStore, plan: &mut Plan) {
    plan.prune_bindings = store
        .stale_actor_bindings(STALE_BINDING_TTL)
        .unwrap_or_default();
    plan.prune_scratch = store.stale_scratch(STALE_SCRATCH_TTL).unwrap_or_default();
    report_stale_bindings(b, &plan.prune_bindings);
}

/// Say what the device store is carrying, in the read-only pass too.
///
/// **Info, never a warning.** A stale binding costs ~190 bytes and breaks
/// nothing: the workspace it names is gone, so there is no sync to be
/// wrong about. Ranking tidiness alongside a torn op log is how the loud
/// lines in this report stop being read. The user learns the number, and
/// `--repair` is where they act on it.
fn report_stale_bindings(b: &mut Builder, stale: &[ActorBinding]) {
    if stale.is_empty() {
        return;
    }
    b.info(format!(
        "device store: {} actor binding(s) name a workspace that no longer exists — \
         `outl doctor --repair` drops them (a binding whose volume is merely unmounted \
         is never counted here)",
        stale.len()
    ));
}
