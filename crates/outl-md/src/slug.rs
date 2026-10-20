//! Slugify page names for stable filesystem paths.
//!
//! `[[Avelino]]` (the user-visible name, kept verbatim in `title::`)
//! maps to `pages/avelino.md` on disk. The rule:
//!
//! - The name is normalised to NFC first, so one name spelled two
//!   ways maps to one slug — and therefore one page id — on every
//!   device.
//! - Lowercase.
//! - Common Latin diacritics fold to their ASCII base (`á` → `a`,
//!   `ç` → `c`, `ñ` → `n`, ...).
//! - Any other letter or digit keeps its own spelling, lowercased:
//!   `[[你好]]` is `pages/你好.md`, `[[Проект]]` is `pages/проект.md`.
//! - Everything else (punctuation, emoji, marks) becomes `-`.
//! - Consecutive `-` collapse; leading/trailing `-` are stripped.
//! - Empty input maps to `untitled`.
//!
//! **Two names that differ only by an accent outside the fold table
//! are two pages.** `CAFÉ` and `cafe` are one because the table holds
//! `é`; `ΛΟΓΟΣ` and `λόγος` are not, because it holds no Greek. That
//! is the table's scope, not a case problem — case is handled, down to
//! `ς`/`σ` and the Turkish dotless `ı`. Widening it is a separate
//! decision, and a latinising one for scripts whose accents carry
//! meaning.
//!
//! **The keep-or-drop decision follows the build's Unicode tables**
//! (`char::is_alphanumeric`, `char::to_lowercase`, NFC), so a codepoint
//! assigned between two toolchains slugifies differently on binaries
//! built either side of it, and the page id with it. Freezing a table
//! in this crate does not remove that — two versions of a frozen table
//! disagree the same way — it only adds that a newly assigned script
//! stays broken until someone ships the update, which is the bug this
//! rule exists to fix. The exposure is one name using a codepoint
//! assigned since the older binary shipped, and on that binary the
//! result is what every name used to get: `untitled`.
//!
//! Future autocomplete (`[[`, `#`) will search by `title::`
//! property — not by slug — so users can have `[[Meu Projeto]]` and
//! `[[meu-projeto]]` resolve to the same file via the slug, but the
//! human-facing display always uses the title.

use sha2::{Digest, Sha256};
use unicode_normalization::UnicodeNormalization;

/// Fallback name when the input slugifies to nothing.
pub const UNTITLED_SLUG: &str = "untitled";

/// Longest slug, in **bytes**, that `pages/<slug>.md` can carry.
///
/// Every mainstream filesystem caps a path component at 255 bytes, so
/// this is where a slug stops being projectable. `slugify` never
/// returns anything longer and `outl_actions::page::is_valid_slug`
/// reads this constant rather than its own copy, because a producer
/// and a validator that disagree leave a name no client can open.
///
/// 255 bytes is 255 ASCII characters but only 85 CJK ones, so a
/// sentence a user would plausibly type as a page name reaches it.
pub const MAX_SLUG_BYTES: usize = 255;

/// Bytes the slug's own filename adds on top of it, worst case.
///
/// The slug is not the filename. It projects to `pages/<slug>.md` and
/// `pages/<slug>.outl`, and each is written by renaming a hidden
/// sibling over it. The longest of those is `.<slug>.outl.tmp`, ten
/// bytes more than the slug. A slug that fills the whole component
/// leaves no room for that temp file, and `slugify` keeps this much
/// back rather than hand the op log a page no device can project.
const FILENAME_WRAPPER_BYTES: usize = 10;

/// Convert a user-visible page name into a filesystem-safe slug.
pub fn slugify(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut prev_dash = true; // suppress leading '-'
    for ch in name.nfc() {
        let before = out.len();
        match fold_char(ch) {
            Some(folded) => out.push(folded),
            // A letter or digit with no ASCII fold keeps its own
            // spelling. Dropping it is what put every CJK, Cyrillic,
            // Greek or Hangul name — and `ł`, `ř`, `ş`, `ộ` inside a
            // Latin one — on a shared slug, and the page id derives
            // from the slug (issue #405).
            //
            None if ch.is_alphanumeric() => out.extend(case_fold(ch)),
            None => {}
        }
        // Read off `out` rather than reported per arm, so a lowercase
        // expansion filtered down to nothing cannot claim it wrote.
        if out.len() > before {
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        return UNTITLED_SLUG.to_string();
    }
    shorten(out)
}

/// The lowercase form of a character the fold table has no answer for.
///
/// Uppercase first, then lowercase — **not `to_lowercase` alone.** A
/// case-insensitive filesystem (APFS and NTFS, both by default)
/// compares names by uppercasing them, and 21 groups of characters are
/// each already lowercase, stay distinct under `to_lowercase`, and
/// share an uppercase form: `s`/`ſ`, `µ`/`μ`, `ς`/`σ`, `i`/`ı`, most
/// of the Greek symbol variants, the Old Cyrillic ones. Each group
/// would give two slugs, two page ids, and one `pages/<slug>.md` to
/// hold both — so one page silently overwrites the other, or both
/// freeze on `PageMarkdownAheadOfLog`.
///
/// Routing through uppercase gives the same answer the filesystem
/// gives, so two names that must share a file also share a slug.
/// Pinned exhaustively by
/// `no_two_slugs_land_on_one_file_on_a_case_insensitive_disk`.
///
/// Only letters and digits survive: the expansion can carry a mark
/// (`İ` → `i` + a combining dot), and a mark in the slug would make
/// the next `slugify` of it return something else.
fn case_fold(ch: char) -> impl Iterator<Item = char> {
    ch.to_uppercase()
        .flat_map(char::to_lowercase)
        .filter(|c| c.is_alphanumeric())
}

/// Bring a slug under [`MAX_SLUG_BYTES`], keeping distinct names
/// distinct.
///
/// The head is cut on a character boundary and a digest of the whole
/// slug is appended, so two names that share a long prefix stay two
/// pages. Truncating alone would hand them one slug, and a page's
/// NodeId derives from its slug — that is issue #405 again, smaller.
///
/// The digest is of the slug, not of anything local, so two devices
/// that each type the same long name still converge on one page.
fn shorten(slug: String) -> String {
    let budget = MAX_SLUG_BYTES - FILENAME_WRAPPER_BYTES;
    if slug.len() <= budget {
        return slug;
    }
    // 64 bits, not 32: the suffix is the only thing keeping two long
    // names apart, and 32 bits is ~2^16 tries to collide two names on
    // purpose — cheap for a forged `[[ref]]` in an imported vault, and
    // the prize is two pages merged onto one node.
    let digest = hex::encode(&Sha256::digest(slug.as_bytes())[..8]);
    let mut cut = budget - digest.len() - 1; // room for `-<digest>`
    while !slug.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}-{digest}", slug[..cut].trim_end_matches('-'))
}

/// Map a single character to its ASCII fold, if any. Returns:
/// - `Some(c)`, always ASCII alphanumeric, for a character with a fold.
/// - `None` for everything else, which the caller keeps verbatim when
///   it is a letter or digit and turns into a `-` otherwise.
fn fold_char(ch: char) -> Option<char> {
    let folded = match ch {
        // Latin lowercase accents.
        'á' | 'à' | 'â' | 'ã' | 'ä' | 'å' | 'ā' => 'a',
        'é' | 'è' | 'ê' | 'ë' | 'ē' => 'e',
        'í' | 'ì' | 'î' | 'ï' | 'ī' => 'i',
        'ó' | 'ò' | 'ô' | 'õ' | 'ö' | 'ō' | 'ø' => 'o',
        'ú' | 'ù' | 'û' | 'ü' | 'ū' => 'u',
        'ç' => 'c',
        'ñ' => 'n',
        'ý' | 'ÿ' => 'y',
        'œ' => 'o', // collapse ligatures
        'æ' => 'a',
        'ß' => 's',
        // Latin uppercase: lowercase first, then fold next round.
        'A'..='Z' => return Some(ch.to_ascii_lowercase()),
        'Á' | 'À' | 'Â' | 'Ã' | 'Ä' | 'Å' | 'Ā' => 'a',
        'É' | 'È' | 'Ê' | 'Ë' | 'Ē' => 'e',
        'Í' | 'Ì' | 'Î' | 'Ï' | 'Ī' => 'i',
        'Ó' | 'Ò' | 'Ô' | 'Õ' | 'Ö' | 'Ō' | 'Ø' => 'o',
        'Ú' | 'Ù' | 'Û' | 'Ü' | 'Ū' => 'u',
        'Ç' => 'c',
        'Ñ' => 'n',
        'Ý' | 'Ÿ' => 'y',
        // The only uppercase letter whose lowercase form the table
        // folds but whose own form it did not: without this it reached
        // the keep-verbatim arm, became `ß`, and the next slugify of
        // that found the table — breaking the fixed point.
        'ẞ' => 's',
        'Œ' => 'o',
        'Æ' => 'a',
        // Anything else passes through unchanged for the caller to decide.
        c if c.is_ascii_alphanumeric() => c,
        _ => return None,
    };
    Some(folded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_lowercase_passthrough() {
        assert_eq!(slugify("avelino"), "avelino");
        assert_eq!(slugify("Avelino"), "avelino");
    }

    #[test]
    fn spaces_become_single_dash() {
        assert_eq!(slugify("meu projeto"), "meu-projeto");
        assert_eq!(slugify("  meu   projeto  "), "meu-projeto");
    }

    #[test]
    fn accents_fold_to_ascii() {
        assert_eq!(slugify("ação"), "acao");
        assert_eq!(slugify("orçamento"), "orcamento");
        assert_eq!(slugify("está"), "esta");
        assert_eq!(slugify("Não"), "nao");
        assert_eq!(slugify("São Paulo"), "sao-paulo");
        assert_eq!(slugify("café"), "cafe");
    }

    #[test]
    fn punctuation_becomes_dash() {
        assert_eq!(slugify("meu/projeto"), "meu-projeto");
        assert_eq!(slugify("meu.projeto"), "meu-projeto");
        assert_eq!(slugify("meu_projeto"), "meu-projeto");
        assert_eq!(slugify("meu:projeto"), "meu-projeto");
    }

    #[test]
    fn iso_date_stays_intact() {
        assert_eq!(slugify("2026-05-24"), "2026-05-24");
    }

    #[test]
    fn empty_or_all_punct_becomes_untitled() {
        assert_eq!(slugify(""), "untitled");
        assert_eq!(slugify("   "), "untitled");
        assert_eq!(slugify("---"), "untitled");
        assert_eq!(slugify("///"), "untitled");
    }

    #[test]
    fn emoji_collapses_to_dashes() {
        // Emojis aren't ASCII alphanumerics → become dashes → trimmed.
        assert_eq!(slugify("ship 🚀"), "ship");
        assert_eq!(slugify("🚀launch🚀"), "launch");
    }

    #[test]
    fn a_name_in_another_script_keeps_its_own_spelling() {
        // Issue #405: dropping every unfoldable letter put `你好`, `世界`
        // and `#中文` all on `untitled`, and the page NodeId derives from
        // the slug, so they were one page — on every client, and across
        // devices.
        assert_eq!(slugify("你好"), "你好");
        assert_ne!(slugify("你好"), slugify("世界"));
        assert_ne!(slugify("世界"), UNTITLED_SLUG);
        assert_eq!(slugify("こんにちは world"), "こんにちは-world");
        assert_eq!(slugify("Проект"), "проект");
    }

    #[test]
    fn a_latin_name_keeps_the_part_in_another_script() {
        // `[[Rust 笔记]]` used to open `[[Rust]]`: the Chinese stem fell
        // away and what was left slugified to an existing page.
        assert_eq!(slugify("Rust 笔记"), "rust-笔记");
        assert_ne!(slugify("Rust 笔记"), slugify("Rust"));
    }

    #[test]
    fn a_latin_letter_outside_the_fold_table_survives() {
        // Polish `ł`, Czech `ř`, Turkish `ş`, Vietnamese `ộ`: dropping
        // these collapsed distinct names in otherwise-Latin scripts.
        // The fold table still wins where it has an entry, so `ó` is
        // `o` here exactly as in `café` → `cafe`; only the letters it
        // has no answer for keep their own spelling.
        assert_eq!(slugify("Łódź"), "łodź");
        assert_ne!(slugify("miał"), slugify("mia"));
        assert_ne!(slugify("Řeka"), slugify("eka"));
    }

    #[test]
    fn a_decomposed_spelling_slugs_the_same_as_a_composed_one() {
        // The slug decides the page's NodeId, so two devices spelling one
        // name differently — macOS hands back NFD, a Korean IME composes
        // Hangul out of jamo — must not mint two pages. Normalising to
        // NFC first is what makes the id converge.
        //
        // The `ne` is what keeps #405's collapse-everything-to-`untitled`
        // from passing this test: two names that both vanish are equal
        // for the wrong reason.
        let composed = slugify("한글");
        assert_ne!(composed, UNTITLED_SLUG);
        assert_eq!(
            composed,
            slugify("\u{1112}\u{1161}\u{11ab}\u{1100}\u{1173}\u{11af}")
        );
        assert_eq!(slugify("você nota"), slugify("voc\u{65}\u{302} nota"));
    }

    #[test]
    fn ligatures_fold() {
        assert_eq!(slugify("œuvre"), "ouvre"); // œ → o
        assert_eq!(slugify("Æsop"), "asop");
        // German ß folds to a single 's' (we don't expand to "ss" — that
        // would require multi-char output and adds little for slug clarity).
        assert_eq!(slugify("straße"), "strase");
    }

    #[test]
    fn numbers_preserved() {
        assert_eq!(slugify("Q4 2026 plan"), "q4-2026-plan");
    }

    #[test]
    fn every_character_slugifies_to_a_fixed_point() {
        // Every caller treats a slug as a fixed point: the resolution
        // ladder slugifies a name that may already be one, and `outl
        // page create --slugify` is documented as idempotent on a clean
        // slug. A character whose slug slugifies to something else
        // gives one name two slugs, two page ids and two files.
        //
        // Exhaustive because the one character that broke this — `ẞ`,
        // folded only in its lowercase form — is not a shape anyone
        // thinks to test, and the three hand-written cases in
        // `slug_is_idempotent` all missed it.
        let mut broken: Vec<String> = Vec::new();
        for cp in 0..=0x10_FFFF_u32 {
            let Some(ch) = char::from_u32(cp) else {
                continue;
            };
            let once = slugify(&ch.to_string());
            let twice = slugify(&once);
            if once != twice {
                broken.push(format!("U+{cp:04X}: {once:?} -> {twice:?}"));
            }
        }
        assert!(
            broken.is_empty(),
            "{} character(s) are not a fixed point: {:?}",
            broken.len(),
            &broken[..broken.len().min(6)]
        );
    }

    #[test]
    fn no_two_slugs_land_on_one_file_on_a_case_insensitive_disk() {
        // APFS and NTFS are case-insensitive by default, and they
        // compare names by uppercasing them. So two characters that are
        // each already lowercase, stay distinct under `to_lowercase`,
        // and share an uppercase form give two slugs, two page ids, and
        // ONE `pages/<slug>.md` plus ONE `.outl`. Whichever page writes
        // second overwrites the first, or both freeze on
        // `PageMarkdownAheadOfLog` — silent either way, and the
        // rebuilt sidecar hides it afterwards.
        //
        // 21 groups do this, `s`/`ſ`, `µ`/`μ`, `ς`/`σ` and `i`/`ı`
        // among them. None was reachable while every slug was
        // `[a-z0-9-]`: only the ASCII member of each group survived.
        //
        // Exhaustive rather than a list of the ones we thought of,
        // because the cost of a missing row is a page overwriting
        // another page.
        let mut by_upper: std::collections::HashMap<char, Vec<String>> =
            std::collections::HashMap::new();
        for cp in 0..=0x10_FFFF_u32 {
            let Some(ch) = char::from_u32(cp) else {
                continue;
            };
            if !ch.is_alphanumeric() {
                continue;
            }
            let mut up = ch.to_uppercase();
            let (first, rest) = (up.next(), up.next());
            // A multi-character uppercase (`ﬁ` → `FI`) is not one
            // filename entry, so it cannot collide this way.
            if rest.is_some() {
                continue;
            }
            let Some(first) = first else { continue };
            by_upper
                .entry(first)
                .or_default()
                .push(slugify(&ch.to_string()));
        }
        let clashes: Vec<_> = by_upper
            .iter()
            .filter(|(_, slugs)| slugs.iter().any(|s| s != &slugs[0]))
            .map(|(up, slugs)| format!("U+{:04X} -> {slugs:?}", *up as u32))
            .collect();
        assert!(
            clashes.is_empty(),
            "{} group(s) of names share a file but not a slug: {:?}",
            clashes.len(),
            &clashes[..clashes.len().min(6)]
        );
    }

    #[test]
    fn a_slug_leaves_room_for_the_filename_it_becomes() {
        // The slug is not the filename. `pages/<slug>.md` is written by
        // renaming `pages/.<slug>.md.tmp` over it, and the sidecar's
        // temp is `.<slug>.outl.tmp` — ten bytes more than the slug.
        // A slug that fills the whole 255-byte component therefore
        // commits a page to the op log whose `.md` no device can ever
        // write, which is worse than the refusal it replaced.
        let long = slugify(&"语".repeat(100));
        assert!(
            long.len() + FILENAME_WRAPPER_BYTES <= MAX_SLUG_BYTES,
            "slug is {} bytes, so its temp file is {}",
            long.len(),
            long.len() + FILENAME_WRAPPER_BYTES,
        );
    }

    #[test]
    fn capital_sharp_s_folds_like_the_lowercase_one() {
        // `ß` is in the fold table and `ẞ` (U+1E9E) was not, so it fell
        // to the keep-verbatim arm, lowercased to `ß`, and the *next*
        // slugify of that found the table — two slugs, two NodeIds, one
        // German word. It is the only uppercase letter whose lowercase
        // form the table folds but whose own form it did not.
        assert_eq!(slugify("STRAẞE"), "strase");
        assert_eq!(slugify("STRAẞE"), slugify("straße"));
    }

    #[test]
    fn slug_is_idempotent() {
        let s = slugify("Meu Projeto X");
        assert_eq!(slugify(&s), s);
        // Every caller treats a slug as a fixed point — the resolution
        // ladder slugifies a name it may already have slugified. So the
        // two paths that rewrite characters have to land on one too.
        let script = slugify("Rust 笔记");
        assert_eq!(slugify(&script), script);
        let long = slugify(&"语".repeat(100));
        assert_eq!(slugify(&long), long);
        // A character the table folds only in its lowercase form is the
        // shape that breaks this, and the two cases above cannot catch
        // it: both land on characters the table has no entry for.
        let sharp = slugify("STRAẞE");
        assert_eq!(slugify(&sharp), sharp);
    }
}
