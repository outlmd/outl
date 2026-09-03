//! Cross-client token guards for RFC 0022.
//!
//! These exist to FAIL if someone re-splits the token namespace or
//! reintroduces a stylesheet colour with no `Palette` field behind
//! it. Do not delete them as "just greps" — the bug they guard
//! shipped for three releases and made the OS appearance setting
//! silently change markdown block elevation on the desktop: the
//! desktop's `--color-iosd-*` meant "elevated", mobile's meant
//! "dark", and the shared `MarkdownInline` component read both
//! through Tailwind's `dark:` variant, so the OS dark-mode toggle —
//! a signal with no business deciding block elevation — silently
//! picked which meaning won.
//!
//! Both tests below read the checked-out source tree directly
//! (`env!("CARGO_MANIFEST_DIR")` walked up to the workspace root),
//! not a build artifact, so they see exactly what a contributor's
//! diff changed.

use std::collections::HashSet;
use std::path::Path;

/// `crates/outl-theme` -> workspace root.
fn workspace_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root must exist")
}

// ─────────────────────────────────────────────────────────────────
// 1. no_client_references_the_legacy_ios_namespace
// ─────────────────────────────────────────────────────────────────

/// The legacy namespace must never come back — but a *mention* of
/// the string `--color-ios` is not the bug. RFC 0022's own
/// migration left three deliberate mentions behind: a comment in
/// `outl-desktop/src/styles.css` and in
/// `outl-frontend-shared/src/theme/palette.ts` recording that the
/// namespace was deleted, and the assertion in
/// `outl-frontend-shared/src/theme/palette.test.ts` that proves
/// `applyPaletteToRoot` never writes it. A test that greps for the
/// bare string fails on a *correct* tree, and a test that fails on
/// a clean tree gets deleted by the first person who hits it —
/// taking the real guard with it.
///
/// So this checks functional USE, not mention: does some line of
/// non-comment code actually read or write a `--color-ios*` custom
/// property? Four shapes count as use (matching the ones RFC 0022's
/// migration replaced):
///
/// - `var(--color-ios...)` — a CSS custom-property read.
/// - `(--color-ios...)` — Tailwind v4 arbitrary-value syntax, e.g.
///   `bg-(--color-ios-bg)` (this also matches the `var(...)` case
///   above, since both have an open-paren directly against the
///   token).
/// - `set("--color-ios...` / `setProperty("--color-ios...` (either
///   quote style) — a JS write.
/// - `--color-ios...:` — a CSS declaration defining one.
///
/// A bare mention inside a comment is excluded by stripping `//`
/// and `/* */` comments line-by-line before matching (tracking
/// block-comment state across lines — the tricky case is exactly
/// the multi-line `/* */` block in `styles.css` that names the
/// namespace across two lines). `#`-comments are deliberately NOT
/// stripped: every file this test walks is `.ts`/`.tsx`/`.css`,
/// where `#` is never a comment marker and IS the first character
/// of every hex colour literal (`#fbbf24`) — treating `#` as a
/// comment opener here would silently truncate real declarations
/// instead of excluding prose.
///
/// This function does not need to be lexically perfect (it doesn't
/// understand string literals, so `"// not a comment"` would be
/// mis-stripped) — it only needs to correctly classify the shapes
/// RFC 0022's migration produced, and it is unit-tested against the
/// exact four existing mentions below plus each of the four use
/// shapes it must catch.
#[test]
fn no_client_references_the_legacy_ios_namespace() {
    let root = workspace_root();
    let mut offenders = Vec::new();

    for client in ["outl-desktop", "outl-mobile", "outl-frontend-shared"] {
        let src = root.join("crates").join(client).join("src");
        walk(&src, &mut |path, text| {
            for (lineno, hit) in functional_uses(text) {
                offenders.push(format!("{}:{lineno}: {hit}", path.display()));
            }
        });
    }

    assert!(
        offenders.is_empty(),
        "--color-ios-* / --color-iosd-* was deleted by RFC 0022 and is \
         functionally referenced again (not just mentioned in a comment) \
         in:\n  {}\n\
         This is the exact bug RFC 0022 fixed: the same token name meant \
         a different colour on each client, and the OS dark-mode setting \
         silently picked which meaning won. Route through `outl_theme::Palette` \
         and `applyPaletteToRoot()` instead of writing a `--color-ios*` token.",
        offenders.join("\n  ")
    );
}

/// Recursive read of every text file under `dir`, skipping
/// `node_modules`, `dist` and `target` (build/dependency output —
/// never source this test needs to guard, and `node_modules` alone
/// can hold tens of thousands of files).
fn walk(dir: &Path, f: &mut impl FnMut(&Path, &str)) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        if name == "node_modules" || name == "dist" || name == "target" {
            continue;
        }
        if path.is_dir() {
            walk(&path, f);
        } else if let Ok(text) = std::fs::read_to_string(&path) {
            f(&path, &text);
        }
    }
}

/// Every `(line_number, matched_line)` pair in `text` where the
/// legacy namespace is functionally used (see the test's doc
/// comment for the four shapes), after stripping `//` and `/* */`
/// comments.
fn functional_uses(text: &str) -> Vec<(usize, String)> {
    let mut hits = Vec::new();
    let mut in_block_comment = false;
    for (i, raw_line) in text.lines().enumerate() {
        let code = strip_comment(raw_line, &mut in_block_comment);
        if line_uses_legacy_namespace(&code) {
            hits.push((i + 1, raw_line.trim().to_string()));
        }
    }
    hits
}

/// Strips `//` line comments and `/* */` block comments from one
/// line, carrying block-comment state across calls via
/// `in_block_comment`. Doesn't understand string literals — good
/// enough here because none of the files this test walks put a
/// `--color-ios` token inside a string that also contains `//` or
/// `/*` on the same line.
fn strip_comment(line: &str, in_block_comment: &mut bool) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    loop {
        if *in_block_comment {
            match rest.find("*/") {
                Some(end) => {
                    rest = &rest[end + 2..];
                    *in_block_comment = false;
                }
                None => return out,
            }
            continue;
        }
        let line_comment = rest.find("//");
        let block_comment = rest.find("/*");
        match (line_comment, block_comment) {
            (Some(lc), Some(bc)) if lc < bc => {
                out.push_str(&rest[..lc]);
                return out;
            }
            (Some(lc), None) => {
                out.push_str(&rest[..lc]);
                return out;
            }
            (_, Some(bc)) => {
                out.push_str(&rest[..bc]);
                rest = &rest[bc + 2..];
                *in_block_comment = true;
            }
            (None, None) => {
                out.push_str(rest);
                return out;
            }
        }
    }
}

/// Does this already-comment-stripped line functionally use
/// `--color-ios*`? See the test's doc comment for the four shapes.
fn line_uses_legacy_namespace(code: &str) -> bool {
    const TOKEN: &str = "--color-ios";

    // Shape 1 + 2: `var(--color-ios...)` and Tailwind arbitrary-value
    // syntax `bg-(--color-ios-bg)` both put an open-paren directly
    // against the token, no quote in between — which is exactly what
    // distinguishes them from a quoted string mention like
    // `toContain("--color-ios")` (paren, then a quote, then the token).
    if code.contains("(--color-ios") {
        return true;
    }

    // Shape 3: a JS write via `set(...)` / `setProperty(...)`, either
    // quote style.
    for call in [
        "set(\"--color-ios",
        "set('--color-ios",
        "setProperty(\"--color-ios",
        "setProperty('--color-ios",
    ] {
        if code.contains(call) {
            return true;
        }
    }

    // Shape 4: a CSS custom-property declaration, `--color-ios...:`.
    // Walk every occurrence of the bare token and check whether the
    // identifier it's part of is immediately followed (after only
    // whitespace) by a colon.
    let mut cursor = code;
    while let Some(pos) = cursor.find(TOKEN) {
        let after = &cursor[pos + TOKEN.len()..];
        let ident_end = after
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
            .unwrap_or(after.len());
        if after[ident_end..].trim_start().starts_with(':') {
            return true;
        }
        cursor = &after[ident_end..];
    }

    false
}

#[test]
fn legacy_namespace_detector_finds_every_use_shape() {
    for use_line in [
        r#"background: var(--color-ios-bg);"#,
        r#"class="text-(--color-ios-accent)""#,
        r#"decoration-(--color-ios-md-link-fg)/40"#,
        r#"set("--color-ios-bg", palette.bg);"#,
        r#"set('--color-iosd-bg', palette.bg_elev);"#,
        r#"root.style.setProperty("--color-ios-fg", value);"#,
        r#"--color-ios-bg: #f6f4fb;"#,
        r#"  --color-iosd-card:   #241a33;"#,
    ] {
        let mut in_comment = false;
        let stripped = strip_comment(use_line, &mut in_comment);
        assert!(
            line_uses_legacy_namespace(&stripped),
            "expected a legacy-namespace use to be detected in: {use_line:?}"
        );
    }
}

#[test]
fn legacy_namespace_detector_ignores_the_deliberate_mentions() {
    // The exact lines RFC 0022's own migration left behind — see the
    // module doc. If this test starts failing, the detector has become
    // too aggressive and will delete these mentions' rationale along
    // with the namespace it's meant to guard.
    let comment_block = "\
/*
 * is one namespace now (RFC 0022 deleted the legacy `--color-ios-*`
 * / `--color-iosd-*` pair once the shared renderers migrated off it).
 */";
    let doc_comment = "\
/**
 * `--color-ios-*` / `--color-iosd-*` namespace this used to also
 * write.
 */";
    let line_comment = "    // RFC 0022 deleted --color-ios-* / --color-iosd-*. On the desktop";
    let assertion = r#"    expect(style).not.toContain("--color-ios");"#;

    for text in [comment_block, doc_comment, line_comment, assertion] {
        assert!(
            functional_uses(text).is_empty(),
            "expected no functional use in a deliberate mention: {text:?}"
        );
    }
}

// ─────────────────────────────────────────────────────────────────
// 2. the_theme_tokens_match_the_palette
// ─────────────────────────────────────────────────────────────────

/// Every `--color-outl-*` name either client's `styles.css` declares
/// in its `@theme` block must correspond to a real `outl_theme::Palette`
/// field. `applyPaletteToRoot()` writes `--color-outl-${kebab(field)}`
/// for every field (`crates/outl-frontend-shared/src/theme/palette.ts`),
/// so a stylesheet name that doesn't reverse-map to a field is a
/// colour with no owner in `Palette` — a second, disconnected
/// definition of a colour, which is the exact failure mode RFC 0022's
/// invariant exists to prevent. `outl_theme::Palette` is meant to be
/// the *only* source of every colour on every client; a boot value
/// under an invented token name is silently unreachable from Rust
/// and can drift from the palette forever without anyone noticing.
///
/// Kebab/snake conversion mirrors `applyPaletteToRoot`'s `kebab()`
/// exactly: `selected_bullet_bg` (Rust field) <-> `--color-outl-selected-bullet-bg`
/// (CSS name).
#[test]
fn the_theme_tokens_match_the_palette() {
    let root = workspace_root();
    let valid_fields: HashSet<&'static str> = outl_theme::default()
        .fields()
        .into_iter()
        .map(|(name, _)| name)
        .collect();

    let mut offenders = Vec::new();
    for client in ["outl-desktop", "outl-mobile"] {
        let css_path = root
            .join("crates")
            .join(client)
            .join("src")
            .join("styles.css");
        let css = std::fs::read_to_string(&css_path)
            .unwrap_or_else(|e| panic!("reading {}: {e}", css_path.display()));

        for name in theme_block_color_outl_names(&css) {
            let snake = name.replace('-', "_");
            if !valid_fields.contains(snake.as_str()) {
                offenders.push(format!(
                    "{}: --color-outl-{name} (no Palette field `{snake}`)",
                    css_path.display()
                ));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "a client stylesheet declares a --color-outl-* token with no matching \
         `outl_theme::Palette` field:\n  {}\n\
         A hex added to a stylesheet under an invented token name is a second, \
         disconnected definition of a colour — exactly what RFC 0022's invariant \
         (`outl_theme::Palette` is the single owner of every colour on every \
         client) exists to rule out. Either add the field to `Palette` \
         (crates/outl-theme/src/palette.rs) and give every preset a value, or \
         fix the stylesheet name to match the field `applyPaletteToRoot` \
         actually writes.",
        offenders.join("\n  ")
    );
}

/// Extracts the `<name>` in every `--color-outl-<name>: <value>;`
/// declaration inside the first `@theme { ... }` block of `css`.
/// Returns the kebab-case suffix only (without the `--color-outl-`
/// prefix).
fn theme_block_color_outl_names(css: &str) -> Vec<String> {
    let Some(theme_start) = css.find("@theme") else {
        return Vec::new();
    };
    let Some(brace_start) = css[theme_start..].find('{') else {
        return Vec::new();
    };
    let body_start = theme_start + brace_start + 1;

    // Brace-depth walk rather than `find('}')`, in case a future
    // `@theme` block ever contains a nested `{}` (it doesn't today,
    // but `color-mix(...)` inside a *value* only uses parens, so this
    // is cheap insurance against a silently-wrong truncation).
    let mut depth = 1usize;
    let mut body_end = css.len();
    for (offset, ch) in css[body_start..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    body_end = body_start + offset;
                    break;
                }
            }
            _ => {}
        }
    }
    let body = &css[body_start..body_end];

    const PREFIX: &str = "--color-outl-";
    let mut names = Vec::new();
    for line in body.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix(PREFIX) else {
            continue;
        };
        let Some(colon) = rest.find(':') else {
            continue;
        };
        names.push(rest[..colon].trim().to_string());
    }
    names
}
