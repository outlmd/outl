//! External-markdown page metadata extraction: YAML frontmatter and
//! leading-H1 titles.
//!
//! Markdown produced by other tools (Obsidian, Bear, Jekyll/Hugo
//! exports, …) carries page metadata in a leading `---` fenced YAML
//! block and/or a leading `# H1` heading.
//!
//! Two consumers, two different jobs:
//!
//! - **The importers** ([`parse_frontmatter`]) *translate* the YAML into
//!   outl `key:: value` properties, because an import is a one-way
//!   conversion into the dialect and the source file is left behind.
//!   Source-specific policy — which keys a given tool considers app-only
//!   metadata, how a `date` value should be normalized — stays with the
//!   caller: the drop-list is a parameter and values come back verbatim.
//! - **The dialect parser** ([`crate::parse::parse`]) *preserves* it. A
//!   workspace folder that is also an Obsidian vault is the interop story
//!   `transport = "file"` exists for, so the fence is split off with
//!   [`split_frontmatter_counted`], carried verbatim in
//!   [`crate::ParsedPage::frontmatter`], written back byte-for-byte by
//!   [`crate::render::render`], and stored in the op log under
//!   [`PAGE_FRONTMATTER_KEY`] so a projection from the tree can re-emit
//!   it. See [`PAGE_FRONTMATTER_KEY`] for why the log is not optional
//!   here.
//!
//! Both routes read the same scan, so they can never disagree about
//! where the fence ends — and `strip_bom` lives here, rather than in
//! `parse`, so they cannot disagree about where it *starts* either.

use serde_yaml_ng::Value as YamlValue;

/// Page-property key that carries a page's verbatim YAML frontmatter
/// through the **op log**.
///
/// The value is the fence's *body* — no delimiters, no trailing newline —
/// exactly as [`split_frontmatter`] returns it.
/// [`crate::render::render`] puts the `---` lines back.
///
/// # Why the op log and not just the file
///
/// Preserving the fence in the parser alone leaves it as content the log
/// has never seen, and invariant 8 then has only two moves, both bad: it
/// refuses every re-projection (the page freezes, so no Obsidian page can
/// ever be appended to again) or it allows one (the fence is deleted the
/// first time any client renders the tree over the `.md`). Page-level
/// metadata that must converge between devices belongs in an `Op` —
/// invariant 7 — and the page-property channel already is one
/// (`Op::SetProp` on the page root), so the fence rides it.
///
/// Reserved, like `page-slug` / `page-kind`, and in
/// `outl_actions::tree::is_page_model_key` alongside them: it names a fact
/// the dialect renders with its own syntax, so no surface offers it as a
/// user-editable property. This crate cannot enforce that — the predicate
/// lives one layer up — so the enforcement is there, not in this comment.
///
/// **The renderer has to see it, which is an ordering rule, not an
/// exemption.** `outl_actions::journal::render_page_md_with` lifts the key
/// into [`crate::ast::ParsedPage::frontmatter`] *before* filtering the
/// page-model keys out of the property list. Filtering first drops the
/// fence from every projection — the write issue #281 reported.
pub const PAGE_FRONTMATTER_KEY: &str = "page-frontmatter";

/// Drop a leading UTF-8 BOM.
///
/// An encoding artifact, not content. It is not whitespace
/// (`char::is_whitespace` is false for U+FEFF), so `trim` leaves it glued
/// to the first `- ` and that line stops being a bullet: the whole first
/// block is recovered as verbatim text with the marker inside it, warning
/// and all. Any `.md` written by a Windows editor lost its first block's
/// identity on import, and a leading `title::` stopped being a page
/// property the same way.
///
/// Dropped rather than preserved: no renderer re-emits it, so keeping it
/// would leave the file changing shape on every save.
///
/// # Why it lives in this module
///
/// Because [`scan_fence`] and [`crate::parse::parse_fragment`] both have
/// to reach the **same** answer about where the file's first real byte is,
/// and this module is the one they already share. It used to live in
/// `parse`, where only the grammar called it, so the scan below read a
/// BOM'd Obsidian page as having no fence at all while the parser read it
/// as having one. The two disagreed on exactly the files that carry a BOM,
/// and `frontmatter_line_count` is what tells [`crate::unlogged`] which
/// leading lines are page metadata — a verdict that decides whether bytes
/// get overwritten. A second copy of "where the BOM ends" is the same
/// class of divergence as a second copy of "where the fence ends".
pub(crate) fn strip_bom(text: &str) -> &str {
    text.strip_prefix('\u{feff}').unwrap_or(text)
}

/// Where a well-formed leading `---` fence ends.
struct Fence {
    /// Byte range of the YAML body inside the (LF-normalized) text.
    yaml_end: usize,
    /// Byte offset of the first body byte after the closing delimiter.
    body_offset: usize,
    /// How many source **lines** the whole fence occupied, delimiters
    /// included.
    lines: usize,
}

/// Locate the closing delimiter of a leading `---` fence.
///
/// Returns `None` when `text` does not open with `---` on its own line,
/// or when no closing delimiter follows — a malformed fence must never
/// swallow the file.
///
/// Tolerates CRLF without normalizing, so a caller that only wants the
/// line count pays no allocation.
///
/// **A leading BOM is skipped, and the returned byte offsets index the
/// stripped text.** A caller that slices has to slice the same string —
/// [`split_frontmatter_counted`] strips for that reason, not for tidiness.
/// [`Fence::lines`] is unaffected: the BOM shares the opening delimiter's
/// line either way.
fn scan_fence(text: &str) -> Option<Fence> {
    let text = strip_bom(text);
    let open = if text.starts_with("---\n") {
        "---\n".len()
    } else if text.starts_with("---\r\n") {
        "---\r\n".len()
    } else {
        return None;
    };
    let after_open = &text[open..];

    let mut cursor = 0usize;
    // `seen` counts the lines after the opening delimiter; a closing one is
    // found at `seen`, so the fence spans `open + seen + close` lines.
    for (seen, line) in after_open.split_inclusive('\n').enumerate() {
        let trimmed = line.trim_end_matches(['\n', '\r']);
        // YAML's `...` document-end marker closes a fence too.
        if trimmed == "---" || trimmed == "..." {
            return Some(Fence {
                yaml_end: open + cursor,
                body_offset: open + cursor + line.len(),
                lines: seen + 2,
            });
        }
        cursor += line.len();
    }
    None
}

/// Split a leading `---\n...\n---\n` block from the file. Returns
/// `(Some(yaml_text), body)` when present and well-formed; otherwise
/// `(None, original_text)`. YAML's `...` document-end marker is also
/// honoured as a closing fence.
pub fn split_frontmatter(text: &str) -> (Option<String>, String) {
    let (yaml, body, _) = split_frontmatter_counted(text);
    (yaml, body)
}

/// [`split_frontmatter`] plus the number of source **lines** the fence
/// occupied (`0` when there is none).
///
/// The count exists so [`crate::parse::parse`] can keep
/// [`crate::ParseWarning::line`] file-relative after handing the body to
/// the outline grammar. A warning that points at the wrong row sends the
/// user to the wrong line of their own file, and `doctor` prints it
/// verbatim.
///
/// Shares one scan with [`split_frontmatter`] and
/// [`frontmatter_line_count`] — "where does the fence end" has one owner.
pub fn split_frontmatter_counted(text: &str) -> (Option<String>, String, usize) {
    // [`scan_fence`] skips a BOM, and the offsets it hands back index the
    // text *it* read — so slice that same string, and return a body with
    // no BOM in it. Stripping only inside the scan would leave the two
    // three bytes apart and the slices would land mid-fence.
    let text = strip_bom(text);
    // A CRLF file is normalized before slicing, because the byte offsets
    // below index the returned body. `str::lines` already strips `\r`, so
    // the outline grammar downstream never sees the difference; the
    // renderer writes LF either way.
    if text.starts_with("---\r\n") {
        let normalized = text.replace("\r\n", "\n");
        let (yaml, body, lines) = split_frontmatter_counted(&normalized);
        return (yaml, body, lines);
    }
    let Some(fence) = scan_fence(text) else {
        // No closing fence — treat the whole file as body so we don't
        // drop user content.
        return (None, text.to_string(), 0);
    };
    let yaml = &text["---\n".len()..fence.yaml_end];
    let yaml = yaml.strip_suffix('\n').unwrap_or(yaml).to_string();
    let body = text
        .get(fence.body_offset..)
        .unwrap_or_default()
        .to_string();
    (Some(yaml), body, fence.lines)
}

/// How many leading lines of `text` a well-formed `---` frontmatter fence
/// occupies — `0` when there is none.
///
/// Allocation-free, which is why it exists next to
/// [`split_frontmatter_counted`]: `crate::unlogged` asks this per page on
/// every sweep and only needs to know which lines to skip, not what they
/// said.
///
/// A leading BOM is skipped but **counted as part of the first line**, so
/// the result stays a `lines()` index into the caller's own text.
pub fn frontmatter_line_count(text: &str) -> usize {
    scan_fence(text).map_or(0, |f| f.lines)
}

/// Render a frontmatter body back to its fenced form, delimiters and
/// trailing newline included.
///
/// The inverse of [`split_frontmatter`]'s first element, and paired with
/// it deliberately: an encoder and its decoder ship together so the next
/// consumer does not re-derive half of one.
///
/// Normalizes a `...` closing delimiter to `---`. Both mean "the fence
/// ends here", so nothing is lost, and the file settles on one spelling
/// after the first save instead of carrying two.
pub fn render_frontmatter(yaml: &str) -> String {
    let mut out = String::with_capacity(yaml.len() + 9);
    out.push_str("---\n");
    if !yaml.is_empty() {
        out.push_str(yaml);
        out.push('\n');
    }
    out.push_str("---\n");
    out
}

/// Parsed frontmatter, ready to re-emit as outl `key:: value`
/// properties.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Frontmatter {
    /// The `title` key, when present as a scalar.
    pub title: Option<String>,
    /// Remaining scalar properties in source order. Values are
    /// verbatim — any source-specific normalization (dates, …) is the
    /// caller's job.
    pub props: Vec<(String, String)>,
    /// Count of keys that were dropped: either listed in `drop_keys`
    /// or carrying a non-scalar value we can't represent as
    /// `key:: value`.
    pub dropped: usize,
}

/// Parse a YAML frontmatter block into a flat [`Frontmatter`].
///
/// - `title` (scalar) is lifted into [`Frontmatter::title`].
/// - `tags` accepts a scalar (comma / space separated), an inline
///   list, or a block list; the result is normalized to `#name` form
///   and joined with spaces under a single `tags` property.
/// - Keys listed in `drop_keys`, and any key whose value is a
///   sequence / mapping we can't flatten, are counted in
///   [`Frontmatter::dropped`].
/// - Every other scalar key passes through verbatim.
///
/// Returns `None` when the YAML itself fails to parse. Callers should
/// restore the original fenced block verbatim into the body so the
/// user's content isn't silently lost.
pub fn parse_frontmatter(yaml: &str, drop_keys: &[&str]) -> Option<Frontmatter> {
    let parsed: YamlValue = serde_yaml_ng::from_str(yaml).ok()?;
    let map = match parsed {
        YamlValue::Mapping(m) => m,
        _ => return Some(Frontmatter::default()),
    };

    let mut fm = Frontmatter::default();
    for (k, v) in map.into_iter() {
        let YamlValue::String(key) = k else {
            continue;
        };
        if drop_keys.contains(&key.as_str()) {
            fm.dropped += 1;
            continue;
        }
        match key.as_str() {
            "title" => {
                if let Some(s) = scalar_string(&v) {
                    fm.title = Some(s);
                } else {
                    fm.dropped += 1;
                }
            }
            "tags" => {
                let tags = tags_from_yaml(&v);
                if !tags.is_empty() {
                    fm.props.push(("tags".to_string(), tags.join(" ")));
                } else {
                    fm.dropped += 1;
                }
            }
            _ => {
                if let Some(s) = scalar_string(&v) {
                    fm.props.push((key, s));
                } else {
                    // Non-scalar value we can't represent as `key:: v`.
                    fm.dropped += 1;
                }
            }
        }
    }
    Some(fm)
}

/// Render a scalar YAML value (string / number / bool) to a String.
/// Returns `None` for sequences and mappings.
fn scalar_string(v: &YamlValue) -> Option<String> {
    match v {
        YamlValue::String(s) => Some(s.clone()),
        YamlValue::Number(n) => Some(n.to_string()),
        YamlValue::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// Extract tags from a YAML value. Frontmatter dialects allow three
/// shapes:
/// - scalar: `tags: foo` (also comma / space separated)
/// - inline list: `tags: [foo, bar]`
/// - block list: `tags:\n  - foo\n  - bar`
///
/// Returned tags are normalized to `#name` form (no leading `#` in
/// the YAML, but `#`-prefixed in outl's `tags::` property).
fn tags_from_yaml(v: &YamlValue) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    match v {
        YamlValue::String(s) => {
            for t in s.split([',', ' ']) {
                let t = t.trim();
                if !t.is_empty() {
                    out.push(tag_form(t));
                }
            }
        }
        YamlValue::Sequence(seq) => {
            for item in seq {
                if let Some(s) = scalar_string(item) {
                    let s = s.trim();
                    if !s.is_empty() {
                        out.push(tag_form(s));
                    }
                }
            }
        }
        _ => {}
    }
    out
}

/// Normalize a tag to `#name` form. Strips any leading `#` the user
/// might have written (source tools accept both `foo` and `#foo`).
fn tag_form(raw: &str) -> String {
    let stripped = raw.trim_start_matches('#');
    format!("#{stripped}")
}

/// If `body` opens (after optional blank lines) with a single H1 line
/// (`# Heading`), return `(Some(title), rest_of_body)` with the H1
/// line stripped. Otherwise return `(None, body_unchanged)`. Only the
/// very first non-blank line is considered — a heading buried inside
/// the body stays as content.
pub fn extract_leading_h1(body: &str) -> (Option<String>, String) {
    let lines: Vec<&str> = body.lines().collect();
    let mut idx = 0;
    while idx < lines.len() && lines[idx].trim().is_empty() {
        idx += 1;
    }
    if idx >= lines.len() {
        return (None, body.to_string());
    }
    let trimmed = lines[idx].trim_start();
    let Some(rest) = trimmed.strip_prefix("# ") else {
        return (None, body.to_string());
    };
    let title = rest.trim().to_string();
    if title.is_empty() {
        return (None, body.to_string());
    }
    let remaining = if lines.len() > idx + 1 {
        lines[idx + 1..].join("\n")
    } else {
        String::new()
    };
    (Some(title), remaining)
}

#[cfg(test)]
mod tests;
