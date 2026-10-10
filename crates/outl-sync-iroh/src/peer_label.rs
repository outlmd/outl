//! How a log line names a peer.
//!
//! Pairing stores a human alias for every device (`outl peer pair` defaults it
//! to the hostname) and `outl peer list` / `outl peer status` render it, but the
//! sync and serve log paths identified every peer by `iroh`'s `fmt_short()`
//! alone. On a two-machine setup that means cross-referencing `peers.json` to
//! learn which laptop pushed ops ([issue #337]).
//!
//! A label is `macbook-pro (b75b908373)`: the alias **plus** the same short hex
//! the lines carried before, so an aliased line still greps against one that
//! could not resolve an alias. An unknown id degrades to the bare short hex.
//!
//! ## Deliberate non-changes
//!
//! Three groups of `fmt_short()` call sites stay on hex, and each is a verdict
//! rather than an omission:
//!
//! - **This device's own node id** (`identity`, the bound-endpoint line, the
//!   transport's `Debug`). `peers.json` lists the *other* devices; there is no
//!   alias for self to resolve.
//! - **A refused dialer** (`authz`). It is refused precisely *because* it is not
//!   in `peers.json`, so the lookup would always fall back — and the two failure
//!   arms there cannot read the file at all.
//! - **`SyncProgress.peer`** (`engine_sync`, `engine_snapshot`, `engine_assets`
//!   each keep a `peer_short`). That is a wire field, not a log line: the
//!   desktop resolves aliases client-side by prefix-matching it
//!   (`SyncProgressView.tsx`), and widening it would break that match.
//!
//! The low-level connect-retry helpers (`connect_sync`, `connect_snapshot`,
//! `connect_asset`, the connection pool) also stay on hex. They hold an endpoint
//! and an addr, never a workspace path, and they log at `debug`.
//!
//! **The list above is enforced, not just written down**, by
//! `every_fmt_short_in_this_crate_is_an_accounted_for_exemption`. A prose list
//! would not have held: `engine_assets.rs` reused its `peer_short` in three log
//! lines — including the `warn!` on the corrupt-peer path — while its
//! byte-identical sibling `engine_snapshot::pull_snapshot_from_peer` was
//! converted. Nothing failed, and it took a reviewer to notice.
//!
//! [issue #337]: https://github.com/outlmd/outl/issues/337

use std::fmt;
use std::path::Path;

use crate::peers::{PeerEntry, PeersStore};

/// How many hex chars of a node id a label carries.
///
/// Matches `iroh::EndpointId::fmt_short` (the first 5 bytes, hex-encoded), which
/// is what every one of these lines printed before an alias was available. Keep
/// them equal: a user grepping a hex prefix out of an older log must still hit
/// the aliased line.
const SHORT_HEX_LEN: usize = 10;

/// Longest alias a label will print, in `char`s.
///
/// The alias defaults to the machine hostname, so a real one is far under this.
/// The bound exists because the value is **remote input**: a peer's pairing
/// payload supplies it and the only other limit on it is the 64 KiB payload cap,
/// which is 64 KiB of log line, reprinted on every sync pass.
const MAX_ALIAS_CHARS: usize = 64;

/// An alias as it is safe to render, or `None` when nothing survives.
///
/// A peer sends its own alias at pairing time (`PairingPayload`) and the
/// membership gossip merge persists one for a device this user never paired
/// with. Neither path validates it, so this is attacker-controlled text on its
/// way to an operator's terminal.
///
/// Two properties, neither cosmetic:
///
/// - **One log line stays one record.** An alias carrying `\n` forges a whole
///   fabricated line — a plausible `WARN … refusing an unknown / revoked peer`
///   attributed to another device — and every line-oriented reader (`grep`, a
///   log shipper, `journalctl`) believes it. That is exactly the
///   grep-correlation property this module promises.
/// - **The log cannot drive the terminal.** `\x1b[2J\x1b[H` clears the screen of
///   whoever is tailing `outl serve`, which is this module's whole audience.
///
/// Neither can be delegated to the subscriber. Measured against
/// `tracing-subscriber` 0.3.23: its `EscapeGuard` wraps only the `message`
/// field, so `info!(peer = %label, …)` emits raw ESC bytes while
/// `info!("… {label}")` escapes them — and **neither form escapes `\n`**, so a
/// forged record gets through both.
///
/// So control characters go, and the bidi/format overrides go with them — not
/// control characters by `char::is_control`, yet they reorder rendered text (the
/// trojan-source trick). Everything else stays: a hostname may legitimately
/// carry an accent, a kanji or an emoji, and a whitelist would mangle it.
///
/// `None` on empty is what makes an alias of nothing but newlines fall through
/// to the bare hex instead of rendering as `" (b75b908373)"`.
///
/// **Call this where an alias enters, as well as where it is shown.** The two
/// are not interchangeable: sanitizing at the boundary keeps hostile text out of
/// `peers.json` from here on, and sanitizing at the render is what covers a file
/// an older binary already wrote. Both callers are cheap; only one of them is
/// retroactive.
pub fn sanitize_alias(alias: Option<&str>) -> Option<String> {
    let kept = alias?.chars().filter(|c| {
        !c.is_control()
            && !matches!(
                c,
                '\u{200e}' | '\u{200f}' | '\u{2066}'..='\u{2069}' | '\u{202a}'..='\u{202e}'
            )
    });
    let mut out: String = kept.clone().take(MAX_ALIAS_CHARS).collect();
    if kept.count() > MAX_ALIAS_CHARS {
        out.push('\u{2026}');
    }
    let trimmed = out.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// The `fmt_short()` of a stored node id — the **only** place this crate derives
/// the short hex from a `peers.json` string.
///
/// It parses rather than slicing because `EndpointId::from_str` accepts base32
/// as well as hex, so a hand-edited or foreign entry could hold an id whose
/// first 10 characters are not hex at all. Slicing there would print a prefix
/// that matches nothing, while the unlisted-peer fallback — which holds a parsed
/// id — printed the real one. Two sources for one fact, disagreeing exactly
/// where a log line is the only evidence available.
///
/// An unparseable id degrades to its own first characters: a damaged
/// `peers.json` is not worth losing the line over.
fn short_hex(node_id: &str) -> String {
    node_id
        .parse::<iroh::EndpointId>()
        .map(|id| id.fmt_short().to_string())
        .unwrap_or_else(|_| node_id.chars().take(SHORT_HEX_LEN).collect())
}

/// How a log line should name `peer`: `macbook-pro (b75b908373)` when pairing
/// stored an alias, the short hex alone when it did not.
///
/// A free function rather than an inherent `impl` on [`PeerEntry`], matching
/// `authz` — the crate's other single owner over `peers.json`. Every other
/// inherent `impl` in this crate lives in the module that declares its type, so
/// an `impl PeerEntry` here would be the one place a reader of `peers.rs` could
/// not find.
pub(crate) fn log_label(peer: &PeerEntry) -> String {
    match sanitize_alias(peer.alias.as_deref()) {
        Some(alias) => format!("{alias} ({})", short_hex(&peer.node_id)),
        None => short_hex(&peer.node_id),
    }
}

/// How a command's output should name `peer`: the alias alone, or the short hex
/// when there is none.
///
/// The log's second shape. A log line carries both because it is grepped and
/// correlated across devices; `outl sync`'s summary is read once, by someone who
/// already knows which machines they own, so the hex there is noise.
///
/// Sanitized like `log_label` and for the same reason: `outl sync` prints this
/// with `println!`, which escapes nothing at all.
pub fn display_label(peer: &PeerEntry) -> String {
    sanitize_alias(peer.alias.as_deref()).unwrap_or_else(|| short_hex(&peer.node_id))
}

/// [`display_label`] for `node_id` within `store`, or the short hex when the
/// store holds no entry for it.
pub fn display_label_in(store: &PeersStore, node_id: &str) -> String {
    store
        .list()
        .iter()
        .find(|peer| peer.node_id == node_id)
        .map(display_label)
        .unwrap_or_else(|| short_hex(node_id))
}

/// [`display_label_in`] against the `peers.json` at `peers_path`.
///
/// The form a one-shot command wants: it reloads per call, which `outl sync`'s
/// summary pays once per peer on a handful of sub-KB reads, and in exchange the
/// fallback for an unreadable store lives here rather than in every caller. A
/// caller in a loop over many peers should load the store once and use
/// [`display_label_in`].
pub fn display_label_at(peers_path: &Path, node_id: &str) -> String {
    match PeersStore::load_or_default(peers_path) {
        Ok(store) => display_label_in(&store, node_id),
        Err(_) => short_hex(node_id),
    }
}

/// [`log_label`] for `node_id` within `store`, or the short hex when the store
/// holds no entry for it.
///
/// Unlisted is an ordinary case, not an error: a stranger's dial, a device the
/// user just removed, a `peers.json` written before aliases existed.
pub(crate) fn log_label_in(store: &PeersStore, node_id: &str) -> String {
    store
        .list()
        .iter()
        .find(|peer| peer.node_id == node_id)
        .map(log_label)
        .unwrap_or_else(|| short_hex(node_id))
}

/// [`log_label_in`], resolved off disk at format time.
///
/// For the many log sites that hold a workspace path but no loaded store. The
/// `peers.json` read happens inside [`fmt::Display::fmt`], which `tracing` only
/// calls once a subscriber has accepted the event — so a filtered-out line and a
/// quiet sync pass both cost nothing.
pub(crate) struct PeerLogLabel<'a> {
    peers_path: &'a Path,
    node_id: iroh::EndpointId,
}

/// Name `node_id` using the aliases in the `peers.json` at `peers_path`.
///
/// Prefer [`log_label`] where the entry is already in hand; this is the form
/// that needs only a path.
pub(crate) fn peer_log_label(peers_path: &Path, node_id: iroh::EndpointId) -> PeerLogLabel<'_> {
    PeerLogLabel {
        peers_path,
        node_id,
    }
}

impl fmt::Display for PeerLogLabel<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // An unreadable peer list degrades to hex. A log line is the wrong place
        // to report that the credential store is damaged — `authz` fails closed
        // on the same file and says so.
        match PeersStore::load_or_default(self.peers_path) {
            Ok(store) => f.write_str(&log_label_in(&store, &self.node_id.to_string())),
            Err(_) => write!(f, "{}", self.node_id.fmt_short()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peers::workspace_peers_path;

    /// A real 64-char lowercase-hex node id; `fmt_short()` of it is `b75b908373`.
    const HEX_ID: &str = "b75b908373aabbccddeeff00112233445566778899aabbccddeeff0011223344";

    fn entry(node_id: &str, alias: Option<&str>) -> PeerEntry {
        PeerEntry {
            node_id: node_id.to_string(),
            alias: alias.map(str::to_string),
            relay_url: None,
            endpoint_addr: None,
            added_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    /// The command-output shape, ported from PR #353: the alias alone, because
    /// `outl sync`'s summary is read once by someone who owns the machines.
    #[test]
    fn the_display_shape_drops_the_hex_but_keeps_the_name() {
        assert_eq!(
            display_label(&entry(HEX_ID, Some("macbook-pro"))),
            "macbook-pro"
        );
        assert_eq!(display_label(&entry(HEX_ID, None)), "b75b908373");
    }

    /// `outl sync` prints this with `println!`, which escapes nothing. The
    /// display shape needs the same guard as the log shape, not a weaker one.
    #[test]
    fn the_display_shape_is_sanitized_too() {
        let label = display_label(&entry(HEX_ID, Some("mac\nFORGED")));
        assert_eq!(label.lines().count(), 1);
        assert_eq!(display_label(&entry(HEX_ID, Some("\u{1b}[2J"))), "[2J");
        assert_eq!(display_label(&entry(HEX_ID, Some("\n\r"))), "b75b908373");
    }

    /// Both shapes derive the hex from the same place, so a line that resolved
    /// an alias and one that could not still name the same prefix.
    #[test]
    fn an_unreadable_store_still_names_the_peer_by_the_same_hex() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = workspace_peers_path(tmp.path());
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(&path, b"{ not json").expect("write junk");
        let node_id = iroh::SecretKey::generate().public();

        let expected = node_id.fmt_short().to_string();
        assert_eq!(display_label_at(&path, &node_id.to_string()), expected);
        assert_eq!(peer_log_label(&path, node_id).to_string(), expected);
    }

    /// `EndpointId::from_str` accepts BASE32_NOPAD as well as hex, and base32 is
    /// not hypothetical here: `lan::instance_label` publishes the id that way
    /// over DNS-SD, because 64 hex chars is one byte over RFC 6763's cap. An id
    /// that reached `peers.json` in that spelling would, under a slice, print 10
    /// characters that are not hex and match nothing else in the log. Parsing
    /// normalizes it.
    #[test]
    fn a_base32_stored_id_still_prints_the_hex_prefix() {
        let node_id = iroh::SecretKey::generate().public();
        let base32 = crate::lan::instance_label(&node_id);
        assert_ne!(base32, node_id.to_string(), "base32 and hex must differ");
        assert_eq!(
            log_label(&entry(&base32, None)),
            node_id.fmt_short().to_string()
        );
    }

    /// An id that parses as nothing at all degrades to its own first
    /// characters, by `chars` and not bytes. `&str[..10]` panics on a multibyte
    /// boundary, which is how PR #353's `node_id[..min(8)]` could have taken a
    /// log line down.
    #[test]
    fn an_unparseable_stored_id_truncates_without_panicking() {
        assert_eq!(
            log_label(&entry("héllo-wörld-not-an-id", None)),
            "héllo-wörl"
        );
        assert_eq!(display_label(&entry("ünïcödé", Some("   "))), "ünïcödé");
    }

    /// A peer supplies its own alias and nothing on the wire path validates it,
    /// so a label must not let it forge a log record. Measured before the fix:
    /// this alias produced a genuine second line ending in `(b75b908373)`.
    #[test]
    fn an_alias_cannot_forge_a_second_log_line() {
        let evil = "laptop\n2026-10-09T12:00:00.000000Z  WARN outl_sync_iroh::authz: refusing an unknown / revoked peer";
        let label = log_label(&entry(HEX_ID, Some(evil)));

        assert!(!label.contains('\n'), "label spans two records: {label:?}");
        assert!(
            !label.contains('\r'),
            "a CR re-anchors the line too: {label:?}"
        );
        assert_eq!(label.lines().count(), 1);
    }

    /// The audience for these lines is someone tailing `outl serve`, so the log
    /// must not be able to drive their terminal.
    #[test]
    fn an_alias_cannot_carry_terminal_escapes() {
        let label = log_label(&entry(HEX_ID, Some("\u{1b}[2Jmac\u{1b}[H")));
        assert_eq!(label, "[2Jmac[H (b75b908373)");
        assert!(!label.contains('\u{1b}'));
    }

    /// Not control characters, but they reorder what the operator reads — the
    /// trojan-source trick. An alias must not be able to reverse the line.
    #[test]
    fn an_alias_cannot_reorder_the_line_with_bidi_overrides() {
        let label = log_label(&entry(HEX_ID, Some("mac\u{202e}gnp.eqri")));
        assert_eq!(label, "macgnp.eqri (b75b908373)");
    }

    /// The payload cap is 64 KiB, and the label is reprinted on every sync pass.
    #[test]
    fn an_oversized_alias_is_truncated_not_reprinted_whole() {
        let label = log_label(&entry(HEX_ID, Some(&"A".repeat(65_536))));

        assert_eq!(label, format!("{}… (b75b908373)", "A".repeat(64)));
        assert!(
            label.chars().count() < 100,
            "still floods the line: {label:?}"
        );
    }

    /// An alias of nothing but control characters is not a name. It has to fall
    /// through to the hex, not render as `" (b75b908373)"` — which is why the
    /// sanitize runs before the empty check.
    #[test]
    fn an_alias_that_sanitizes_to_nothing_falls_back_to_hex() {
        assert_eq!(log_label(&entry(HEX_ID, Some("\n\r\u{1b}"))), "b75b908373");
    }

    /// The filter is a blacklist on purpose: a hostname may legitimately carry
    /// an accent, a kanji or an emoji, and a whitelist would mangle it.
    #[test]
    fn a_legitimate_non_ascii_alias_survives_intact() {
        for alias in ["avelino-macbook", "josé-pro", "のーとぶっく", "mac 🍎"] {
            assert_eq!(
                log_label(&entry(HEX_ID, Some(alias))),
                format!("{alias} (b75b908373)"),
            );
        }
    }

    #[test]
    fn an_aliased_peer_carries_its_name_and_the_short_hex() {
        let peer = log_label(&entry(HEX_ID, Some("macbook-pro")));
        assert_eq!(peer, "macbook-pro (b75b908373)");
    }

    #[test]
    fn a_peer_without_an_alias_reads_exactly_as_it_did_before() {
        let peer = log_label(&entry(HEX_ID, None));
        assert_eq!(peer, "b75b908373");
    }

    /// An alias that is present but blank is not a name. Rendering
    /// `" (b75b908373)"` would be strictly worse than the hex it replaced.
    #[test]
    fn a_blank_alias_falls_back_to_hex() {
        let peer = log_label(&entry(HEX_ID, Some("   ")));
        assert_eq!(peer, "b75b908373");
    }

    /// The hex a label carries is byte-for-byte what `fmt_short()` printed, so
    /// a prefix grepped out of an older log still matches an aliased line.
    #[test]
    fn the_short_hex_matches_iroh_fmt_short() {
        let node_id = iroh::SecretKey::generate().public();
        let peer = log_label(&entry(&node_id.to_string(), Some("phone")));
        assert_eq!(peer, format!("phone ({})", node_id.fmt_short()));
    }

    /// An id the store does not list is the ordinary case, and it must read
    /// as the same hex the line carried before aliases existed.
    #[test]
    fn an_unlisted_id_resolves_to_its_short_hex() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = workspace_peers_path(tmp.path());
        let mut store = PeersStore::load_or_default(&path).expect("load");
        store
            .add(entry(
                &iroh::SecretKey::generate().public().to_string(),
                Some("thinkpad"),
            ))
            .expect("add");
        let stranger = iroh::SecretKey::generate().public();

        assert_eq!(
            peer_log_label(&path, stranger).to_string(),
            stranger.fmt_short().to_string()
        );
    }

    #[test]
    fn the_lazy_form_resolves_the_alias_on_disk() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = workspace_peers_path(tmp.path());
        let node_id = iroh::SecretKey::generate().public();
        let mut store = PeersStore::load_or_default(&path).expect("load");
        store
            .add(entry(&node_id.to_string(), Some("thinkpad")))
            .expect("add");

        assert_eq!(
            peer_log_label(&path, node_id).to_string(),
            format!("thinkpad ({})", node_id.fmt_short())
        );
    }

    /// A damaged peer list must not swallow the id the line was about.
    #[test]
    fn an_unreadable_peer_list_still_names_the_peer_by_hex() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = workspace_peers_path(tmp.path());
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(&path, b"{ this is not json").expect("write junk");
        let node_id = iroh::SecretKey::generate().public();

        assert_eq!(
            peer_log_label(&path, node_id).to_string(),
            node_id.fmt_short().to_string()
        );
    }

    /// Every `fmt_short()` in this crate is accounted for.
    ///
    /// The module header lists the groups that print bare hex on purpose, and a
    /// prose list is not a guard: a new `info!("… {}", nid.fmt_short())` compiles,
    /// passes every test, and silently undoes [issue #337]. That is the same
    /// invisible-regression shape `bind.rs` defends with
    /// `every_endpoint_in_this_crate_binds_through_the_one_owner`, so it gets the
    /// same treatment — and it is not hypothetical: `engine_assets.rs` shipped
    /// three such lines while its byte-identical sibling
    /// `engine_snapshot::pull_snapshot_from_peer` was converted, which is how the
    /// gap was found in review.
    ///
    /// Counts, not just file names, because every remaining file is *already* on
    /// the list for a reason that covers only the sites it has today. Coarse, and
    /// it fails for the right reason: adding a hex-named peer to a file that
    /// already has an exempt one is exactly the change this exists to stop.
    ///
    /// Changing a count is allowed — state which group the new site joins.
    /// If it joins none, it belongs in [`peer_log_label`].
    ///
    /// [issue #337]: https://github.com/outlmd/outl/issues/337
    #[test]
    fn every_fmt_short_in_this_crate_is_an_accounted_for_exemption() {
        // (path relative to `src/`, occurrences, why it is not a peer label)
        let exempt: &[(&str, usize, &str)] = &[
            ("authz.rs", 3, "a refused dialer: unlisted by definition, and two arms cannot read the file"),
            ("engine.rs", 2, "this device's own node id: the transport's `Debug` and the bound-endpoint line"),
            ("engine_assets.rs", 3, "2 in `connect_asset` (no path in scope), 1 is the `SyncProgress` wire field"),
            ("engine_snapshot.rs", 3, "2 in `connect_snapshot` (no path in scope), 1 is the `SyncProgress` wire field"),
            ("engine_sync.rs", 3, "2 in `connect_with_fallback` (no path in scope), 1 is the `SyncProgress` wire field"),
            ("identity.rs", 1, "this device's own node id"),
            ("pairing.rs", 3, "2 in an identity-mismatch error (pre-pairing, no entry yet), 1 is our own ready addr"),
            ("peer_conn.rs", 2, "the connection pool: holds an endpoint and an id, no workspace path"),
        ];

        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut scanned = 0usize;
        let mut unaccounted = Vec::new();
        let mut drifted = Vec::new();
        let mut seen = Vec::new();

        for entry in walkdir::WalkDir::new(&src) {
            let entry = entry.expect("walk src/");
            if !entry.file_type().is_file() || entry.path().extension() != Some("rs".as_ref()) {
                continue;
            }
            let rel = entry
                .path()
                .strip_prefix(&src)
                .expect("walkdir yields paths under its root")
                .to_string_lossy()
                .replace('\\', "/");
            // This file is the owner: it renders the fallback and documents the rest.
            if rel == "peer_label.rs" {
                continue;
            }
            scanned += 1;
            let text = std::fs::read_to_string(entry.path()).expect("read source file");
            let found = text.matches("fmt_short").count();
            if found == 0 {
                continue;
            }
            match exempt.iter().find(|(name, _, _)| *name == rel) {
                None => unaccounted.push(format!("{rel} ({found}x)")),
                Some((_, expected, why)) => {
                    seen.push(rel.clone());
                    if found != *expected {
                        drifted.push(format!("{rel}: {expected} -> {found} ({why})"));
                    }
                }
            }
        }

        // A scan that reads nothing passes vacuously, which is worse than no guard.
        assert!(scanned > 1, "the scan found no source files to check");
        assert!(
            unaccounted.is_empty(),
            "these files name a peer by bare hex with no recorded verdict \
             (issue #337) — resolve it through `peer_log_label`, or add a row \
             to `exempt` saying why hex is right: {unaccounted:?}",
        );
        assert!(
            drifted.is_empty(),
            "the `fmt_short` count changed; say which exempt group the new site \
             joins, or route it through `peer_log_label`: {drifted:?}",
        );
        let mut missing: Vec<&str> = exempt
            .iter()
            .map(|(name, _, _)| *name)
            .filter(|name| !seen.iter().any(|s| s == name))
            .collect();
        missing.sort();
        assert!(
            missing.is_empty(),
            "`exempt` lists files that no longer use `fmt_short`; drop the stale \
             rows so the list keeps meaning something: {missing:?}",
        );
    }
}
