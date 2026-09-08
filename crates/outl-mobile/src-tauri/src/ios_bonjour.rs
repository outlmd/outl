//! iOS LAN peer discovery bridge — Rust ↔ `OutlBonjour.swift` (issue #149).
//!
//! # Why iOS is the only platform with a file like this
//!
//! Every other client discovers same-LAN peers with
//! `iroh-mdns-address-lookup`, which speaks mDNS over a plain BSD socket:
//! `bind` on `0.0.0.0:5353`, then `IP_ADD_MEMBERSHIP` on `224.0.0.251`. Since
//! iOS 14 that group join needs `com.apple.developer.networking.multicast`, a
//! restricted entitlement Apple grants by request — and commonly declines,
//! pointing applicants at Bonjour instead.
//!
//! So iOS asks `mDNSResponder`, the system's own mDNS daemon, through
//! `NetService` / `NetServiceBrowser` in `OutlBonjour.swift`. Apple's Local
//! Network Privacy FAQ names exactly two Bonjour operations that still need the
//! entitlement: working with **arbitrary** service types, and browsing for
//! advertised service types (the `_services._dns-sd._udp.local.` meta-query).
//! This does neither — one fixed type, declared in `NSBonjourServices`. There
//! is nothing to apply for and nothing to wait on.
//!
//! # Why the bridge is here and not in `outl-sync-iroh`
//!
//! That crate is `#![forbid(unsafe_code)]`, which is the right answer rather
//! than an obstacle: a sync transport should not carry a platform bridge. It
//! exposes a plain-Rust seam instead (`outl_sync_iroh::lan`), and the `unsafe`
//! lives here, beside the background-sync exports that already cross the same
//! boundary.
//!
//! # What crosses
//!
//! Two C functions out, one function pointer back in. The callback is handed to
//! Swift at startup rather than exported as a `#[no_mangle]` symbol, because a
//! pointer passed at runtime cannot be stripped by dead-code elimination.
//!
//! Addresses cross as `"ip:port[,ip:port…]"` and are parsed by `SocketAddr`'s
//! own `FromStr` on the Rust side, so text→address conversion has one owner
//! rather than a hand-rolled parser per language.

use std::ffi::{c_char, CStr, CString};
use std::sync::Arc;

use outl_sync_iroh::lan::{self, LanAdvertiser};
use tracing::debug;

unsafe extern "C" {
    /// Begin browsing, reporting each resolved peer to `callback`.
    fn outl_ios_bonjour_start(callback: extern "C" fn(*const c_char, *const c_char, *const c_char));
    /// Advertise this endpoint. `relay` may be null.
    fn outl_ios_bonjour_publish(endpoint_id: *const c_char, port: u16, relay: *const c_char);
}

/// Publishes through `OutlBonjour.publish`.
#[derive(Debug)]
struct BonjourAdvertiser;

impl LanAdvertiser for BonjourAdvertiser {
    fn publish(&self, endpoint_id: &str, port: u16, relay: Option<&str>) {
        // An interior NUL cannot occur in either value (an endpoint id is
        // z-base-32, a relay URL is percent-encoded), but a silent skip beats
        // an unwrap on a value that came through two languages.
        let Ok(id) = CString::new(endpoint_id) else {
            return;
        };
        let relay = relay.and_then(|r| CString::new(r).ok());
        let relay_ptr = relay.as_ref().map_or(std::ptr::null(), |c| c.as_ptr());
        // SAFETY: both pointers are valid NUL-terminated buffers for the whole
        // call, and Swift copies out of them before returning.
        unsafe { outl_ios_bonjour_publish(id.as_ptr(), port, relay_ptr) };
    }
}

/// Invoked from Swift's main run loop for each peer it resolves.
///
/// Every pointer is borrowed for the duration of the call only — Swift keeps
/// the backing strings alive across it — so nothing is retained past the
/// return. `lan::peer_discovered` copies what it keeps.
extern "C" fn on_resolved(endpoint_id: *const c_char, addrs: *const c_char, relay: *const c_char) {
    let (Some(id), Some(addrs)) = (borrow(endpoint_id), borrow(addrs)) else {
        return;
    };
    lan::peer_discovered(id, addrs, borrow(relay).unwrap_or_default());
}

/// Borrow a C string, or `None` when null or not UTF-8.
fn borrow<'a>(ptr: *const c_char) -> Option<&'a str> {
    if ptr.is_null() {
        return None;
    }
    // SAFETY: Swift passes a NUL-terminated buffer that outlives this call.
    unsafe { CStr::from_ptr(ptr) }.to_str().ok()
}

/// Register the advertiser and start browsing.
///
/// Idempotent, and safe to call before the sync transport exists: registration
/// order does not matter, because `PlatformAddressLookup::publish` is a no-op
/// until an advertiser is installed and iroh republishes on every address
/// change.
pub fn install() {
    use std::sync::OnceLock;
    static INSTALLED: OnceLock<()> = OnceLock::new();
    INSTALLED.get_or_init(|| {
        lan::register_advertiser(Arc::new(BonjourAdvertiser));
        // SAFETY: `on_resolved` is a plain `extern "C"` fn with no captured
        // state, so the pointer stays valid for the life of the process.
        unsafe { outl_ios_bonjour_start(on_resolved) };
        debug!("bonjour: browsing {} for LAN peers", lan::SERVICE_TYPE);
    });
}
