//! "Does the op log know this line?" — the one owner of that verdict.
//!
//! Three callers need the same answer and must never disagree:
//!
//! - `reconcile_md` asks it **before advancing `last_synced_hash`**, so
//!   the sidecar never claims the log holds content this pass could not
//!   emit an op for (invariant 8);
//! - `outl_actions::apply_page_md_with_sidecar_if_stale` asks it before
//!   re-projecting the tree over a `.md`, so a page holding unlogged
//!   content is refused instead of overwritten;
//! - `outl doctor` asks it in its read-only listing, so the listing can
//!   never promise a repair the writing pass then refuses.
//!
//! It lives in `outl-md` rather than `outl-actions` because it reads
//! nothing but a `.md` string and `SidecarBlock`s — both owned here —
//! and because the producer (`reconcile_md`) is in this crate and
//! cannot depend upwards. `outl-actions` re-exports it so every
//! existing `outl_actions::content_lines_missing_from` path still
//! resolves.

use std::collections::HashMap;

use crate::sidecar::SidecarBlock;

/// Whether `blocks` can answer "does the op log know this line" at all.
///
/// `false` for a sidecar written before `SidecarBlock::text` existed
/// (0.11): every entry carries `text: ""`, so it describes which blocks
/// the page had and nothing about what they said.
///
/// This is the **same condition** [`content_lines_missing_from`] stands
/// down on, exported so callers can distinguish its two empty results,
/// which mean opposite things:
///
/// - *"I checked, nothing is at risk"* → safe to write;
/// - *"I could not check"* → **not** permission to write.
///
/// Reading the second as the first is how a page holding unlogged
/// content gets overwritten by a caller that did ask the question. The
/// two live together so they cannot drift: change the stand-down rule
/// below and this predicate has to move with it, in the same diff.
///
/// An **empty** block list is the opposite case and answers `true` — a
/// page with no blocks has nothing to lose, and treating it as
/// unanswerable would freeze every freshly created page.
///
/// **A write caller wants [`sidecar_can_vouch_for`], not this.** This
/// predicate answers a question about the *sidecar*; a caller about to
/// overwrite a file needs the one about the file, because a page holding
/// only bare bullets is unanswerable here and has nothing to lose
/// (issue #332). Gating a write on this alone is the defect, and the
/// shared-primitives catalogue says to flag it in review.
pub fn sidecar_can_answer(blocks: &[SidecarBlock]) -> bool {
    blocks.is_empty() || blocks.iter().any(|b| !b.text.is_empty())
}

/// Whether `blocks` can vouch for the `.md` currently on `disk` — the
/// question a caller about to **overwrite** that file actually has.
///
/// [`sidecar_can_answer`] answers "can these blocks say what the log
/// held", and a caller that stops there refuses a page it had no reason
/// to: the two questions come apart for every page whose blocks are all
/// empty, because that is the shape of a pre-0.11 sidecar *and* the shape
/// of a page nobody has written in yet.
///
/// `outl init` leaves two pages holding a single empty block, every page
/// created for a `[[link]]` that was never filled is the same shape, and
/// so is a journal day the first time it is opened. Reading those as
/// "cannot vouch" froze them: the tree took a peer's blocks, the `.md`
/// kept its lone bare bullet, and no open, sweep or repair would
/// re-project it (issue #332). Measured on a 2,874-page workspace: one
/// page in that state and **zero** genuine pre-0.11 sidecars, so every
/// decline the first arm produced there was a page with nothing to lose.
///
/// The second arm is the comparison the write it guards would make,
/// through the same owner, so the two cannot disagree about what a
/// content line is: a pre-0.11 page holds real text and still refuses,
/// a fresh one holds only bare bullets and properties, which
/// [`content_lines_missing_from`] already normalises away.
///
/// **This narrows which pages are refused, never which content is
/// protected.** A line the log does not hold still refuses, and it is
/// `unlogged_content_error`, not this predicate, that phrases the
/// refusal.
pub fn sidecar_can_vouch_for(disk: &str, blocks: &[SidecarBlock]) -> bool {
    sidecar_can_answer(blocks)
        || content_lines_missing_from(disk, blocks)
            .iter()
            .all(String::is_empty)
}

/// Lines of `disk`'s YAML frontmatter fence that `rendered` would not
/// write back — `0` when there is nothing to lose.
///
/// The fence travels a **different channel** from block text: the op log
/// holds it as one `Op::SetProp` on the page root
/// ([`crate::frontmatter::PAGE_FRONTMATTER_KEY`]), so
/// [`content_lines_missing_from`] cannot see it — its reference is a block
/// list, and skips the fence for exactly that reason. This is the same
/// "would this write delete bytes" question asked of the channel that can
/// answer it, and it is why both live in this module rather than one
/// growing a second meaning.
///
/// Two shapes count as a loss:
///
/// - **`rendered` has no fence at all.** The tree does not know the
///   channel exists, which is the state a workspace is in between
///   upgrading to a binary that reads the fence and that page's first
///   reconcile: the log holds the fence as bullets from the older parser,
///   the render therefore has none, and writing it back is the
///   [issue #281] rewrite happening once more.
/// - **`rendered` carries a *different* fence and `disk` is not what outl
///   last wrote** (`last_synced_hash` does not match it). The fence was
///   edited on disk after the last agreement, so the log has never seen
///   the disk version, and a post-mutation projection that runs before
///   the orphan reconcile would replace it with the tree's older one.
///
/// A differing fence over bytes outl **did** write last is not a loss: the
/// fence on disk is the one the log held at the last agreement (reconcile
/// logs it before stamping the hash, and a projection renders it from the
/// log), so the render differs only because a peer edited the frontmatter.
/// That is a legitimate remote change and no more a loss than a remote
/// block edit; refusing it would freeze the page, which is issue #166
/// with the blame moved (see [`content_lines_missing_from`]).
///
/// The sidecar keeps no copy of the fence, so the hash is the witness for
/// "the log held this fence", exactly as the sidecar's blocks are the
/// witness for block text. Fences are compared by body, so a CRLF file, a
/// BOM or a `...` closing delimiter never reads as an edit.
///
/// `outl reconcile --ahead-of-log` — the recovery
/// `ActionError::PageMarkdownAheadOfLog` already names — re-reads the
/// fence into the log and clears both shapes.
///
/// [issue #281]: https://github.com/outlmd/outl/issues/281
pub fn frontmatter_lines_missing_from(disk: &str, rendered: &str, last_synced_hash: &str) -> usize {
    let lines = crate::frontmatter::frontmatter_line_count(disk);
    if lines == 0 {
        return 0;
    }
    if crate::frontmatter::frontmatter_line_count(rendered) == 0 {
        return lines;
    }
    if crate::sidecar::file_hash(disk) == last_synced_hash {
        return 0;
    }
    let (on_disk, _) = crate::frontmatter::split_frontmatter(disk);
    let (in_render, _) = crate::frontmatter::split_frontmatter(rendered);
    if on_disk == in_render {
        0
    } else {
        lines
    }
}

/// The content lines in `disk` that **no block the op log knows** can
/// account for.
///
/// `sidecar_blocks` is the reference, not a fresh render of the tree, and
/// the distinction is the whole point. The sidecar's blocks are what the
/// log held when the two last agreed, so comparing against them answers
/// *"does the op log know this line"*. Comparing against a render answers
/// *"do disk and tree disagree"*, which is also yes for every remote
/// edit, every remote delete and every reorder — a guard built on that
/// question refuses to re-project any page a peer has touched, which is
/// issue #166 with the blame moved.
///
/// Compared as a multiset, so a line duplicated on disk but present once
/// in the log still counts as at-risk.
///
/// Lines are normalised before comparison: the bullet marker, the indent
/// and trailing whitespace all come from the renderer's layout, not from
/// the content, so a pure indent or outdent must not read as new text.
/// Blank lines are dropped. Everything else is compared verbatim — this
/// decides whether bytes get deleted, so it errs toward calling a line
/// at-risk.
pub fn content_lines_missing_from(disk: &str, sidecar_blocks: &[SidecarBlock]) -> Vec<String> {
    content_lines_missing_from_texts(disk, sidecar_blocks.iter().map(|b| b.text.as_str()))
}

/// [`content_lines_missing_from`], taking only the block **texts**.
///
/// The verdict is byte-identical — the comparison reads nothing but
/// `SidecarBlock::text`, and [`content_lines_missing_from`] is this
/// function with that projection applied. It exists because one caller
/// has no sidecar at all: `outl_actions`' survey measures what a
/// re-projection would remove, so its reference is the *render*, and
/// wrapping every rendered block in a throwaway `SidecarBlock` minted a
/// ULID and a SHA-256 per block to feed a function that reads neither.
/// On a 2.5k-page graph that is tens of thousands of both, per sweep,
/// every 30 seconds.
///
/// Pinned equivalent by `the_texts_entry_point_agrees_with_the_block_one`
/// — this is a data-loss guard, so the two forms are compared rather
/// than assumed identical.
pub fn content_lines_missing_from_texts<'a>(
    disk: &str,
    logged_texts: impl IntoIterator<Item = &'a str>,
) -> Vec<String> {
    /// One line of the `.md`, reduced to the text a sidecar block would
    /// hold, or `None` when the line is not block content at all.
    ///
    /// The two sides are deliberately asymmetric: the `.md` carries the
    /// renderer's layout (indent, `- ` marker) and a sidecar's `text` does
    /// not, so only this side strips.
    fn disk_line(line: &str) -> Option<&str> {
        let t = line.trim();
        if t.is_empty() {
            return None;
        }
        // Strip the marker exactly once. Repeating it would turn
        // `- - - x` into `x` and let it match a logged block `x`, which is
        // a false negative: unlogged content walking past the guard.
        // A bare `-` is an empty block, a first-class state (every Enter in
        // the TUI makes one), and normalises to the empty text its sidecar
        // entry carries.
        if let Some(body) = t.strip_prefix("- ") {
            return Some(body.trim_start());
        }
        if t == "-" {
            return Some("");
        }
        // No marker. Either a `key:: value` line the renderer emits for a
        // page or block property — which never lives in a block's `text`,
        // so comparing it would flag every page that has one — or a
        // continuation line, which is content. `parse_property_line` is
        // the single owner of that distinction, and it is the same one the
        // parser applies, so the two cannot disagree.
        //
        // Note this is decided *after* the marker check on purpose: a
        // bullet whose own text looks like a property (`- note:: remember`)
        // is stored by the parser as the block's text, so skipping it here
        // would hide real content.
        if crate::parse::parse_property_line(t).is_some() {
            return None;
        }
        Some(t)
    }
    /// A sidecar block's `text` split into the lines it occupies in the
    /// `.md`. No marker to strip and no property filtering: whatever is in
    /// `text` is, by definition, content the log holds.
    ///
    /// An empty block still occupies one line (the renderer emits a bare
    /// `-`), and `"".lines()` yields nothing, so it is contributed
    /// explicitly. Without this the bare `-` on disk matches no block and
    /// every page holding an empty block reads as unlogged.
    fn logged_lines(text: &str) -> impl Iterator<Item = &str> {
        std::iter::once("")
            .take(usize::from(text.is_empty()))
            .chain(text.lines().map(str::trim))
    }

    // **No stand-down here.** A caller that hands over blocks which
    // cannot answer the question must say so itself, via
    // [`sidecar_can_answer`], because only the caller knows where the
    // blocks came from.
    //
    // This function used to stand down on its own when every block had
    // an empty `text` — right for a pre-0.11 sidecar, and wrong for the
    // other kind of caller. `outl doctor` builds its reference from a
    // **render**, and a render of empty blocks is a definitive answer
    // ("the tree holds no text"), not an inability to answer. It hit the
    // same branch and the count came back `0`, so the doctor printed
    // "removes nothing" and the volume guard stayed quiet for a repair
    // that would empty the page — the exact run that guard exists to
    // stop. Measured: disk with 4 content lines, tree rendering `-\n-\n`,
    // reported 0 lines removed.
    //
    // The condition is unchanged, only its owner: it lives at each call
    // site now, next to the knowledge of what the blocks are.

    // A block's text can span lines (continuation), and each of those
    // lands as its own line in the `.md`, so index the pieces.
    let mut known: HashMap<&str, usize> = HashMap::new();
    for text in logged_texts {
        for piece in logged_lines(text) {
            *known.entry(piece).or_insert(0) += 1;
        }
    }

    // Skip a leading YAML frontmatter fence. Its lines are page-level
    // metadata the log holds as one `Op::SetProp` on the page root (see
    // `frontmatter::PAGE_FRONTMATTER_KEY`), never as a block's `text`, so
    // comparing them against block texts would flag every page that has a
    // fence — the same reason `disk_line` skips a `key:: value` line.
    //
    // A false positive here is the expensive direction: it withholds
    // `last_synced_hash` and refuses re-projection, so every page of an
    // Obsidian vault would be frozen in both directions with nothing wrong
    // with it. `frontmatter_line_count` is the same scan `parse` uses to
    // decide the region is metadata, so the two cannot disagree about
    // which lines these are — and an unterminated fence counts as `0`
    // there, which leaves its lines as the content they are.
    //
    // **"Cannot disagree" is a property of one shared scan**, not of two
    // written alike, and it was false in exactly one class of file:
    // `scan_fence` did not skip a UTF-8 BOM and `parse` did. So
    // `\u{feff}---` was frontmatter to the parser and no fence at all
    // here, the skip came back `0`, every line of the fence was reported
    // as unlogged, and the page froze in both directions — permanently,
    // since the projection that would have rewritten the BOM is the same
    // projection being refused. `strip_bom` moved next to `scan_fence` so
    // one function answers for both.

    // Read the file from the same first byte the parser did, for the same
    // reason. U+FEFF is not whitespace, so nothing downstream trims it:
    // left in, it stays glued to the first `- ` and that line matches no
    // block the log holds — a `.md` a Windows editor wrote reads as
    // carrying one unlogged line whether or not it has a fence. This
    // cannot hide real content: it removes three bytes from the file's
    // very first line, and those bytes are in no block's text because
    // `parse` dropped them before the log ever saw the page.
    let disk = crate::frontmatter::strip_bom(disk);
    let mut missing = Vec::new();
    let skip = crate::frontmatter::frontmatter_line_count(disk);
    for raw in disk.lines().skip(skip) {
        let Some(line) = disk_line(raw) else { continue };
        // Try the stripped form first, then the line as written.
        //
        // `disk_line` removes the bullet because the renderer *adds* one
        // — for a block's first line. It does not add one inside a block's
        // text, so a bullet that lives in a code fence, or in a pasted
        // list a block carries as continuation, is part of the text
        // verbatim, marker and all. Asking only the stripped form makes
        // `- endpoint:` on disk fail to match the `- endpoint:` the log
        // holds, and the page reads as carrying unlogged content.
        //
        // That is a false positive, and its cost is the one this whole
        // RFC calls worst: the page is refused for re-projection, its
        // `last_synced_hash` is withheld, and it reconciles on every boot
        // forever — frozen, with nothing wrong with it. Measured on the
        // reporting workspace: 8 pages, 49 lines, every one a bullet
        // inside a fence, and every one verified to survive
        // `parse → render` intact.
        //
        // The second shape does not widen *which* lines the log knows,
        // only how an indented one may match, so a line absent from the
        // log still fails both lookups.
        // Only an **indented** line gets the second chance. The renderer
        // never writes a continuation line at column 0, so a bullet
        // there is a block's own marker and nothing else. Allowing the
        // verbatim form for it would let a root-level `- secret` match a
        // fenced `- secret` the log holds somewhere else on the page —
        // narrow, but this is a data-loss guard and the comment below
        // used to claim it could not happen at all.
        let verbatim = if raw.starts_with([' ', '\t']) {
            Some(raw.trim())
        } else {
            None
        };
        // **Order matters, and for an indented line the verbatim form
        // goes first.** Both forms were already tried; trying the
        // stripped one first is what broke it.
        //
        // The renderer only writes `- ` as a *marker* on a block's first
        // line, which is never indented. So for an indented line the
        // verbatim reading is the likely one and the stripped reading is
        // the fallback — the reverse of what the unindented case wants.
        //
        // Getting it backwards does not merely fail to match: it matches
        // the **wrong** entry and consumes it, because `known` is a
        // multiset that decrements. On `- a` / ` ``` ` / ` - j` / ` ``` `
        // / ` j`, the continuation line `- j` stripped to `j`, spent the
        // single `j` the log held for the *last* line, and that last
        // line then found nothing — so a page whose every line the log
        // holds reported one unlogged line and froze. Found by
        // `a_rendered_page_never_reports_content_its_own_log_holds`;
        // pinned deterministically by
        // `a_fenced_bullet_does_not_steal_a_later_lines_match`.
        let mut take = |key: &str| match known.get_mut(key) {
            Some(n) if *n > 0 => {
                *n -= 1;
                true
            }
            _ => false,
        };
        let hit = match verbatim {
            Some(v) => take(v) || take(line),
            None => take(line),
        };
        if !hit {
            missing.push(line.to_string());
        }
    }
    missing
}

#[cfg(test)]
mod tests {
    use super::*;
    use outl_core::id::NodeId;

    fn blk(text: &str) -> SidecarBlock {
        SidecarBlock::from_text(NodeId::new(), 1, 0, text)
    }

    /// A bullet inside a code fence lives in the block's text **with its
    /// marker**, because the renderer does not add one there.
    ///
    /// Asking only the marker-stripped form made every such page read as
    /// carrying unlogged content — which withholds its hash and refuses
    /// its re-projection, freezing a page that has nothing wrong with it.
    /// Measured on a real workspace: 8 pages, 49 lines, all of this shape,
    /// every one verified to survive `parse → render` intact.
    #[test]
    fn a_bullet_inside_a_code_fence_is_not_reported_as_unlogged() {
        let disk = "- intro\n  ```yaml\n  - endpoint:\n    method: POST\n  ```\n";
        let logged = [blk("intro\n```yaml\n- endpoint:\n  method: POST\n```")];
        assert!(
            content_lines_missing_from(disk, &logged).is_empty(),
            "a fenced bullet the log holds verbatim must not read as unlogged"
        );
    }

    /// The same shape one level down, which is how it actually appeared:
    /// a fenced list pasted from Roam under a nested block.
    #[test]
    fn a_fenced_list_under_a_nested_block_is_not_reported_as_unlogged() {
        let disk = "- parent\n  - ```md\n    ## Objetivos\n    - Aumentar resistência\n    - Desenvolver força\n    ```\n";
        let logged = [
            blk("parent"),
            blk("```md\n## Objetivos\n- Aumentar resistência\n- Desenvolver força\n```"),
        ];
        assert!(
            content_lines_missing_from(disk, &logged).is_empty(),
            "got: {:?}",
            content_lines_missing_from(disk, &logged)
        );
    }

    /// **The guard must not have gone blind.** Accepting a second shape
    /// per line widens how a known line may match; it must not widen
    /// *which* lines count as known.
    #[test]
    fn content_the_log_does_not_have_is_still_reported() {
        let disk = "- intro\n  ```yaml\n  - endpoint:\n  ```\n- a line the log never saw\n";
        let logged = [blk("intro\n```yaml\n- endpoint:\n```")];
        let missing = content_lines_missing_from(disk, &logged);
        assert_eq!(missing, vec!["a line the log never saw".to_string()]);
    }

    /// A bullet on disk whose text the log does not hold in *either* form
    /// fails both lookups — the widening is per-shape, not per-line.
    #[test]
    fn an_unlogged_bullet_is_reported_whichever_shape_it_takes() {
        let disk = "- known\n  - unknown child\n";
        let logged = [blk("known")];
        let missing = content_lines_missing_from(disk, &logged);
        assert_eq!(missing, vec!["unknown child".to_string()]);
    }

    /// Multiset semantics survive the second lookup: a line the log holds
    /// once and disk holds twice still reports one.
    #[test]
    fn a_line_duplicated_on_disk_still_counts_the_surplus() {
        let disk = "- dup\n- dup\n";
        let logged = [blk("dup")];
        assert_eq!(content_lines_missing_from(disk, &logged).len(), 1);
    }

    /// The stand-down moved to the callers, so this function must now
    /// answer even when every block is empty.
    ///
    /// `outl doctor` builds its reference from a **render**, and a render
    /// of empty blocks says "the tree holds no text" — an answer. While
    /// the rule lived in here it silenced that caller too, so the doctor
    /// reported `0` lines removed for a repair that would empty the page
    /// and the volume guard never fired. Measured: 4 content lines on
    /// disk, tree rendering `-\n-\n`, reported 0.
    #[test]
    fn empty_reference_blocks_still_report_the_disk_content() {
        let disk = "- line one\n- line two\n- line three\n- line four\n";
        let empty_render = [blk(""), blk("")];
        assert_eq!(
            content_lines_missing_from(disk, &empty_render).len(),
            4,
            "a reference of empty blocks is an answer, not an absence of one"
        );
    }

    /// The texts-only entry point must reach the **same** verdict as the
    /// block one, on every shape the block one is pinned against.
    ///
    /// It exists for speed (the survey's reference is a render, so
    /// wrapping each line in a `SidecarBlock` minted a ULID and a
    /// SHA-256 it never reads). Speed is not a reason to take
    /// equivalence on faith here: this decides whether a background pass
    /// deletes bytes, so the two forms are compared rather than assumed.
    #[test]
    fn the_texts_entry_point_agrees_with_the_block_one() {
        let cases: [(&str, &[&str]); 7] = [
            (
                "- intro\n  ```yaml\n  - endpoint:\n    method: POST\n  ```\n",
                &["intro\n```yaml\n- endpoint:\n  method: POST\n```"],
            ),
            (
                "- intro\n  ```yaml\n  - endpoint:\n  ```\n- a line the log never saw\n",
                &["intro\n```yaml\n- endpoint:\n```"],
            ),
            ("- known\n  - unknown child\n", &["known"]),
            ("- dup\n- dup\n", &["dup"]),
            ("- line one\n- line two\n", &["", ""]),
            ("- secret\n", &["```\n- secret\n```"]),
            ("- a\n  ```\n  - j\n  ```\n  j\n", &["a\n```\n- j\n```\nj"]),
        ];
        for (disk, texts) in cases {
            let blocks: Vec<SidecarBlock> = texts.iter().map(|t| blk(t)).collect();
            assert_eq!(
                content_lines_missing_from(disk, &blocks),
                content_lines_missing_from_texts(disk, texts.iter().copied()),
                "the two entry points disagree on {disk:?}"
            );
        }
    }

    /// A root-level bullet does not get the verbatim second chance.
    ///
    /// The renderer never writes a continuation at column 0, so a bullet
    /// there is a block's own marker. Letting it match a fenced
    /// `- secret` the log holds elsewhere on the page would hide real
    /// unlogged content — narrow, but this decides whether bytes are
    /// deleted.
    #[test]
    fn a_root_level_bullet_does_not_match_a_fenced_copy_of_itself() {
        let disk = "- secret\n";
        let logged = [blk("```\n- secret\n```")];
        assert_eq!(
            content_lines_missing_from(disk, &logged),
            vec!["secret".to_string()],
            "a root bullet must not borrow a fenced line's identity"
        );
    }

    /// An indented bullet must not spend a later line's match.
    ///
    /// `known` is a **multiset** that decrements, so trying the wrong
    /// form first does not merely fail — it consumes the entry another
    /// line needed. Here the continuation `- j` strips to `j` and takes
    /// the single `j` the log holds for the final line, which then
    /// matches nothing: every line is logged and one is reported.
    ///
    /// A false positive is the expensive direction. It withholds
    /// `last_synced_hash`, refuses re-projection, and `outl reconcile
    /// --ahead-of-log` — the recovery the error names — re-runs this
    /// same computation and refuses again. The page is frozen in both
    /// directions with nothing wrong with it.
    ///
    /// Found by `a_rendered_page_never_reports_content_its_own_log_holds`
    /// on a later proptest seed, which is why this deterministic twin
    /// exists: a seed that finds a defect once is not a test.
    #[test]
    fn a_fenced_bullet_does_not_steal_a_later_lines_match() {
        // Exactly what `render` emits for a single block whose text is
        // "a\n```\n- j\n```\nj".
        let disk = "- a\n  ```\n  - j\n  ```\n  j\n";
        let logged = [blk("a\n```\n- j\n```\nj")];
        assert!(
            content_lines_missing_from(disk, &logged).is_empty(),
            "every line here is in the block's own text"
        );
    }

    /// The same shape, with the bullet's stripped form genuinely absent.
    ///
    /// Guards the fix from being "always prefer verbatim": the indented
    /// `- k` is not in the log in any form, so it must still be
    /// reported. Without this, a fix that stopped stripping altogether
    /// would pass the test above and let unlogged content through —
    /// which is the direction that deletes bytes.
    #[test]
    fn an_indented_bullet_the_log_lacks_is_still_reported() {
        let disk = "- a\n  ```\n  - k\n  ```\n";
        let logged = [blk("a\n```\n```")];
        assert_eq!(
            content_lines_missing_from(disk, &logged),
            vec!["k".to_string()],
            "an indented line absent from the log must fail both forms"
        );
    }
}
