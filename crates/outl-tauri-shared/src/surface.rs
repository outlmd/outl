//! Which surface a refusal reaching this backend is worded for.
//!
//! The two GUI clients share this crate and do **not** share the answer:
//! a desktop user is sitting next to a shell, a phone has no `outl`
//! binary at all. So a refusal cannot be handed one shared string
//! (`trash::list` used to name `outl doctor` and `outl block tree
//! <ULID>` on both).
//!
//! A `cfg` rather than an [`crate::host::AppHost`] method, the same way
//! `capture_inbox` picks its inbox location in this crate: the fact is a
//! property of the build target, not of the host's runtime state, and an
//! associated const with no default would make the ten test hosts in
//! `tests/` declare a client they are not.
//!
//! Its own module because two command bodies ask it now
//! (`commands::trash`'s listing and `commands::block`'s cross-page
//! refusal), and a second `cfg` pair would be a second owner of what
//! this build is — the one thing a `cfg` cannot be caught disagreeing
//! about, since only one arm ever compiles.

use outl_actions::refusal::Surface;

/// The surface this build speaks for. See the module doc.
#[cfg(any(target_os = "ios", target_os = "android"))]
pub const SURFACE: Surface = Surface::Mobile;
/// The surface this build speaks for. See the module doc.
#[cfg(not(any(target_os = "ios", target_os = "android")))]
pub const SURFACE: Surface = Surface::Desktop;
