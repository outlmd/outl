//! Fractional indexing for sibling order.
//!
//! Each sibling's position is a lexicographically sortable ASCII string
//! over the lowercase alphabet `a..=z`, never empty. Inserting between
//! two positions produces a key in finite bytes.
//!
//! Properties, all of them stated by [`Fractional::between`] and pinned
//! by the proptest in `tests/fractional_index.rs`:
//!
//! - `between(None, None)` returns a midpoint of the full space.
//! - The result always sorts **after** `left`, when `left` is given.
//! - The result sorts **before** `right` whenever a key strictly between
//!   the two bounds exists. When none does — the bounds are tied, or
//!   `right` is `left` plus a single `a`, or `right` is the floor `"a"`
//!   itself — the right bound is dropped and the lower bound wins.
//! - It never panics, on any pair of positions.
//!
//! **`between` is a pure function, and that is the whole reason ties
//! exist.** Two replicas reaching for the "same gap" do not each mint
//! their own key: given the same bounds they mint the *same* key, so two
//! devices creating the first child of one parent while offline end up
//! with two siblings at one position. Both ops are valid and both
//! replay; `outl_actions::tree::sort_siblings` is what turns
//! `(position, NodeId)` into the total order that renders them the same
//! way on every device. A comment here used to claim the opposite —
//! "two replicas produce two different positions" — which is what made
//! the tie look unreachable and kept an `assert!` on it for months
//! (issue #282).

use serde::{Deserialize, Serialize};
use std::fmt;

const FIRST_BYTE: u8 = b'a';
const LAST_BYTE: u8 = b'z';
const BELOW_FIRST: u8 = FIRST_BYTE - 1; // 0x60 — sentinel: "before any valid byte"
const ABOVE_LAST: u8 = LAST_BYTE + 1; // 0x7b  — sentinel: "after any valid byte"

/// A lexicographically sortable position string.
///
/// Values are non-empty strings of ASCII lowercase letters (`a..=z`).
/// All constructors guarantee that invariant; consumers may rely on it.
///
/// **Including `Deserialize`.** The derived impl filled the `String`
/// with whatever the input held, which made the one constructor that
/// takes *untrusted* bytes the one that skipped [`Fractional::parse`] —
/// a line of `ops-<actor>.jsonl` arriving over iroh, over iCloud, or
/// half-written after a crash. A `position` of `""` or `"!"` then
/// reached [`Fractional::between`], where it decided the process's
/// lifetime. `#[serde(try_from = "String")]` routes deserialization
/// through `parse`, so a malformed position is an ingestion error at
/// the boundary: the op-log reader skips that line and names it in the
/// log (`JsonlStorage::read_ops_file_into`), and every op after it
/// still replays (invariant 5, no silent loss).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct Fractional(String);

impl TryFrom<String> for Fractional {
    type Error = FractionalError;

    /// The only way in. See the `#[serde(try_from = "String")]` on
    /// [`Fractional`].
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::parse(s)
    }
}

/// Errors when constructing a position from a raw string.
#[derive(Debug, thiserror::Error)]
pub enum FractionalError {
    /// Empty input.
    #[error("fractional position must be non-empty")]
    Empty,
    /// Input contains a byte outside the `a..=z` alphabet.
    #[error("fractional position byte {0:#04x} is outside a..=z")]
    InvalidByte(u8),
}

impl Fractional {
    /// Construct a `Fractional` from a raw string, validating the alphabet.
    pub fn parse(s: impl Into<String>) -> Result<Self, FractionalError> {
        let s = s.into();
        if s.is_empty() {
            return Err(FractionalError::Empty);
        }
        for b in s.bytes() {
            if !(FIRST_BYTE..=LAST_BYTE).contains(&b) {
                return Err(FractionalError::InvalidByte(b));
            }
        }
        Ok(Self(s))
    }

    /// Returns the canonical "first" position.
    pub fn first() -> Self {
        Self(String::from("a"))
    }

    /// Returns the canonical "last" position.
    pub fn last() -> Self {
        Self(String::from("z"))
    }

    /// Returns a position for a new sibling: strictly after `left`, and
    /// strictly before `right` whenever a key between the two exists.
    ///
    /// # When the gap is empty
    ///
    /// Two siblings holding the **same** position is ordinary state, not
    /// corruption. This function is pure, so two devices that each
    /// create the first child of one parent while offline mint the same
    /// key; both ops are valid, both replay, and
    /// `outl_actions::tree::sort_siblings` breaks the tie by `NodeId`
    /// precisely because ties exist. There is then no key strictly
    /// between those two siblings — a tie is separable only by *moving*
    /// one of them.
    ///
    /// `a..=z` has a second empty gap, one that has nothing to do with
    /// ties: nothing sorts between `"a"` and `"aa"`. Every non-empty
    /// tail starts at `a` or above, so `left + tail >= "aa"` for any
    /// tail. One `a` of suffix is empty; two (`"a"` / `"aaa"`) has
    /// exactly `"aa"` in it.
    ///
    /// So "there is no answer" is a state ordinary data reaches, and
    /// this used to `assert!(left < right)` on it — killing the process
    /// every time a user asked for a slot above a tied sibling (vim
    /// `O`), on every boot, because the tie lives in the op log and
    /// replays (issue #282).
    ///
    /// # Guarantees
    ///
    /// - The result is always a valid position: non-empty, `a..=z`.
    /// - The result is always strictly greater than `left`.
    /// - The result is strictly less than `right` **iff** a key strictly
    ///   between the bounds exists. **When the gap is empty the lower
    ///   bound wins**: `right` is dropped and the result is the next key
    ///   above `left`.
    /// - The result is a pure function of the bounds, so two devices
    ///   performing the same insert converge on the same key.
    /// - It never panics.
    ///
    /// A caller that needs the upper bound honoured compares the result
    /// against `right` itself — `outl_actions::tree::position_before`
    /// does, returning `None`, and `create_before` then moves the anchor
    /// up and takes its key, which is the only way to open a slot in a
    /// gap that has none. Returning `Result` was the alternative: it
    /// buys the same check at every call site, and most of them pass no
    /// upper bound at all and so can never fail.
    pub fn between(left: Option<&Self>, right: Option<&Self>) -> Self {
        let left_bytes: &[u8] = left.map(|f| f.0.as_bytes()).unwrap_or(&[]);
        let right_bytes: &[u8] = right.map(|f| f.0.as_bytes()).unwrap_or(&[]);

        // `right` constrains the result only while the result is still a
        // prefix of it; once a byte strictly below `right`'s lands, every
        // later byte is free. Tracking that is not an optimization —
        // reading `right` past the divergence is what made the old
        // in-loop `debug_assert!(r_byte >= l_byte)` fire on bounds as
        // ordinary as `("ab", "ba")`, a debug-only abort on data any
        // outline produces.
        //
        // `left >= right` starts unbound: the upper bound is
        // unsatisfiable (a tie, or a pair a peer's reorder left the
        // wrong way round), so it is dropped instead of asserted on.
        let mut right_binding = match (left, right) {
            (Some(l), Some(r)) => l < r,
            (None, Some(_)) => true,
            (_, None) => false,
        };

        // Terminates by construction, no runaway guard needed: iteration
        // `i` reads byte `i` of each bound and pushes exactly one byte,
        // so by `i == max(len(left), len(right))` both bounds are
        // exhausted, the window is the full `(BELOW_FIRST, ABOVE_LAST)`
        // and the midpoint branch always breaks.
        let bound = left_bytes.len().max(right_bytes.len());
        let mut result = Vec::<u8>::with_capacity(bound + 1);
        for i in 0..=bound {
            debug_assert_eq!(result.len(), i, "exactly one byte pushed per byte index");
            let l_byte = left_bytes.get(i).copied().unwrap_or(BELOW_FIRST);
            if right_binding && i >= right_bytes.len() {
                // Every byte so far equalled `right`'s and `right` has no
                // more, so `result == right`: `left` is a proper prefix
                // of `right` whose tail is a single `a`. That is the
                // empty gap — drop the bound.
                right_binding = false;
            }
            let r_byte = if right_binding {
                right_bytes[i]
            } else {
                ABOVE_LAST
            };

            // Widened to `u16` so the `+ 1` cannot overflow: every
            // constructor validates the alphabet, but a comparison whose
            // correctness depends on that is one `Fractional` away from
            // the panic this function exists to stop having.
            if (r_byte as u16) > (l_byte as u16) + 1 {
                // Room for a byte strictly inside the window.
                result.push((((l_byte as u16) + (r_byte as u16)) / 2) as u8);
                break;
            }

            // Window is tight: `r_byte == l_byte` (still inside the
            // common prefix) or `r_byte == l_byte + 1` (adjacent, so
            // taking `left`'s byte drops us below `right` for good).
            // Descend a level.
            let pushed = if l_byte == BELOW_FIRST {
                FIRST_BYTE
            } else {
                l_byte
            };
            result.push(pushed);
            if right_binding && pushed < right_bytes[i] {
                right_binding = false;
            }
            if right_binding && i >= left_bytes.len() && i + 1 < right_bytes.len() {
                // `left` is exhausted and `result` matches `right` byte
                // for byte with bytes to spare, so `result` is a proper
                // prefix of `right`: already `> left` (same prefix, one
                // byte longer) and `< right`. Descending further walks
                // past the only slot the gap has — this is the
                // `("a", "aaa")` case, whose answer is `"aa"`.
                break;
            }
        }

        debug_assert!(
            result.iter().all(|b| (FIRST_BYTE..=LAST_BYTE).contains(b)),
            "every pushed byte is in a..=z: the midpoint is strictly inside a \
             window the sentinels bound to a..=z, and the descent pushes either \
             a byte of `left` (validated by `parse`) or FIRST_BYTE"
        );
        // Byte-to-char instead of `String::from_utf8(..).expect(..)`:
        // every byte above is ASCII (the `debug_assert!` says why), so
        // there is nothing to fail — and a `Fractional` that somehow
        // carried a non-ASCII byte would widen it here rather than abort
        // the process. `#[serde(try_from)]` on the type is what closes
        // the route that made such a value reachable.
        Self(result.into_iter().map(char::from).collect())
    }

    /// Returns the underlying string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Fractional {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn between_none_none_is_middle() {
        let mid = Fractional::between(None, None);
        assert!(mid.0.bytes().all(|b| b.is_ascii_lowercase()));
    }

    #[test]
    fn between_left_and_right_is_strict() {
        let a = Fractional::parse("a").unwrap();
        let c = Fractional::parse("c").unwrap();
        let b = Fractional::between(Some(&a), Some(&c));
        assert!(a < b);
        assert!(b < c);
    }

    #[test]
    fn between_adjacent_chars_descends() {
        let a = Fractional::parse("a").unwrap();
        let b = Fractional::parse("b").unwrap();
        let mid = Fractional::between(Some(&a), Some(&b));
        assert!(a < mid);
        assert!(mid < b);
    }

    #[test]
    fn between_left_only_is_greater() {
        let m = Fractional::parse("m").unwrap();
        let g = Fractional::between(Some(&m), None);
        assert!(m < g);
    }

    #[test]
    fn between_right_only_is_less() {
        let m = Fractional::parse("m").unwrap();
        let l = Fractional::between(None, Some(&m));
        assert!(l < m);
    }

    /// Two siblings at the same position is ordinary state (see the
    /// `between` doc), and asking for a slot between them used to abort
    /// the process. Issue #282.
    #[test]
    fn between_equal_bounds_is_defined() {
        let a = Fractional::parse("a").unwrap();
        let got = Fractional::between(Some(&a), Some(&a));
        assert!(
            got > a,
            "{got} must sort after the bound that is satisfiable"
        );
    }

    /// Same rule for a pair a peer's reorder left out of order: the
    /// upper bound is unsatisfiable, so it is dropped rather than
    /// asserted on.
    #[test]
    fn between_reversed_bounds_is_defined() {
        let c = Fractional::parse("c").unwrap();
        let a = Fractional::parse("a").unwrap();
        let got = Fractional::between(Some(&c), Some(&a));
        assert!(got > c, "{got} must sort after the left bound");
    }

    /// Convergence: the answer is a pure function of the bounds, so two
    /// devices performing the same insert offline mint the same key.
    #[test]
    fn between_is_deterministic_for_an_empty_gap() {
        let a = Fractional::parse("a").unwrap();
        let first = Fractional::between(Some(&a), Some(&a));
        let again = Fractional::between(Some(&a), Some(&a));
        assert_eq!(first, again);
    }

    /// The other empty gap: nothing sorts between `a` and `aa`, because
    /// every non-empty tail starts at or above `a`. The bound is dropped
    /// for the same reason a tie's is.
    #[test]
    fn between_a_key_and_its_single_a_suffix_drops_the_right_bound() {
        let a = Fractional::parse("a").unwrap();
        let aa = Fractional::parse("aa").unwrap();
        let got = Fractional::between(Some(&a), Some(&aa));
        assert!(got > a, "{got} must sort after the left bound");
    }

    /// One `a` of tail is an empty gap; two is not. `"aa"` is the answer
    /// and the old bisection walked straight past it, returning a key
    /// **above** the right bound.
    #[test]
    fn between_a_key_and_its_double_a_suffix_has_room() {
        let a = Fractional::parse("a").unwrap();
        let aaa = Fractional::parse("aaa").unwrap();
        let got = Fractional::between(Some(&a), Some(&aaa));
        assert!(a < got && got < aaa, "{got} must land inside (a, aaa)");
    }

    /// `("ab", "ba")` is an ordinary pair of sibling keys: the result
    /// diverges below `right` at byte 0, after which `right` stops
    /// constraining anything. Reading it past that point is what made
    /// the loop's `debug_assert!(r_byte >= l_byte)` fire on this input —
    /// a debug-only abort on data a real outline produces.
    #[test]
    fn between_diverged_bounds_do_not_trip_the_loop_invariant() {
        let ab = Fractional::parse("ab").unwrap();
        let ba = Fractional::parse("ba").unwrap();
        let got = Fractional::between(Some(&ab), Some(&ba));
        assert!(ab < got && got < ba, "{got} must land inside (ab, ba)");
    }

    /// A position is only a position if it came through `parse`, and
    /// `Deserialize` is the one constructor that takes untrusted input:
    /// a line of `ops-<actor>.jsonl` arriving over iroh, over iCloud, or
    /// half-written after a crash.
    #[test]
    fn deserialize_rejects_what_parse_rejects() {
        assert!(serde_json::from_str::<Fractional>("\"!\"").is_err());
        assert!(serde_json::from_str::<Fractional>("\"\"").is_err());
        assert!(serde_json::from_str::<Fractional>("\"A\"").is_err());
        assert_eq!(
            serde_json::from_str::<Fractional>("\"abc\"").unwrap(),
            Fractional::parse("abc").unwrap()
        );
    }

    #[test]
    fn between_deep_descent_terminates() {
        // Force a long common prefix and adjacent divergence.
        let l = Fractional::parse("aaabb").unwrap();
        let r = Fractional::parse("aaabc").unwrap();
        let m = Fractional::between(Some(&l), Some(&r));
        assert!(l < m);
        assert!(m < r);
        assert!(m.0.len() < 1024);
    }

    #[test]
    fn parse_rejects_invalid_bytes() {
        assert!(Fractional::parse("A").is_err());
        assert!(Fractional::parse("a1").is_err());
        assert!(Fractional::parse("").is_err());
    }

    #[test]
    fn many_inserts_in_same_gap_remain_distinct() {
        // 50 inserts between "a" and "z", picking the new midpoint each
        // time. Real outliner workloads do this constantly.
        let mut left = Fractional::parse("a").unwrap();
        let right = Fractional::parse("z").unwrap();
        let mut seen = std::collections::HashSet::new();
        seen.insert(left.0.clone());
        seen.insert(right.0.clone());
        for _ in 0..50 {
            let m = Fractional::between(Some(&left), Some(&right));
            assert!(left < m);
            assert!(m < right);
            assert!(seen.insert(m.0.clone()), "duplicate position generated");
            left = m;
        }
    }
}
