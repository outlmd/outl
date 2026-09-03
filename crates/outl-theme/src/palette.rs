//! The `Palette` data type — named hex colors per semantic surface.
//!
//! Every renderer (TUI, desktop) consumes the same fields and maps
//! them to whatever its output expects. Adding a new field means a
//! coordinated change: the field here, a value in every preset, and
//! a render rule on each client.

use serde::{Deserialize, Serialize};

/// Hex-encoded named palette for every styled surface in outl.
///
/// Field naming follows "what the surface IS", not "what it looks
/// like". `ref_link_fg` is "the foreground color used for `[[ref]]`"
/// across every renderer — the TUI underlines it, the desktop
/// renders it as an accented pill, but the hue is shared. Don't add
/// compound names like `inner_bold_in_quote`; if two surfaces
/// genuinely share a style, share the field.
///
/// Wire format: every field is `String` so Serde just sees JSON
/// strings and the desktop frontend can drop each into a CSS custom
/// property without parsing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Palette {
    /// User-visible name (`"outl"`, `"dracula"`, …). Owned because
    /// the palette rides JSON across the Tauri wire — Serde can't
    /// borrow a `&'static str` out of a freshly-parsed `String`.
    pub name: String,

    // ── canvas ────────────────────────────────────────────────────
    /// Background of the main canvas. Terminal users see their
    /// emulator background when this is the same as the terminal
    /// default — we still ship an explicit hex so the desktop has
    /// something concrete to paint.
    pub bg: String,
    /// Background of elevated surfaces (popups, modals, picker).
    pub bg_elev: String,
    /// Primary foreground / body text.
    pub fg: String,
    /// Dim text (secondary metadata).
    pub fg_dim: String,
    /// Dimmer still (placeholders, separators).
    pub fg_dimmer: String,
    /// Panel / pane border.
    pub border: String,
    /// Footer hint text.
    pub hint: String,

    // ── accent rail ───────────────────────────────────────────────
    /// Primary accent (selection bg, ref-link color, active state).
    pub accent: String,
    /// Softer / lighter accent (caret, property value, italic).
    pub accent_soft: String,
    /// Secondary accent (code fg, todo_done) — a distinct hue from
    /// `accent` so success / code never disappears against the
    /// selection color.
    pub accent_alt: String,
    /// Warning hue (todo_open, transient status messages).
    pub warn: String,
    /// Destructive action — delete confirmations, the "remove peer"
    /// button, error toasts. Distinct from `warn`: `warn` is "look
    /// at this", `destructive` is "this cannot be undone".
    ///
    /// Added by RFC 0022. Before it, the TUI's delete confirmation
    /// and the desktop's `ErrorToast` each picked an ad-hoc red,
    /// and mobile had `--color-ios-destructive` that no other
    /// client could see.
    pub destructive: String,

    // ── inline markdown ──────────────────────────────────────────
    /// `[[page]]` reference foreground.
    pub ref_link_fg: String,
    /// `#tag` reference foreground.
    pub tag_link_fg: String,
    /// `[text](url)` link foreground.
    pub md_link_fg: String,
    /// `**bold**` foreground (modifier always BOLD).
    pub bold_fg: String,
    /// `*italic*` foreground (modifier always ITALIC).
    pub italic_fg: String,
    /// `~~strike~~` foreground (modifier always CROSSED_OUT).
    pub strike_fg: String,
    /// `==highlight==` marker background (the highlighter fill).
    pub highlight_bg: String,
    /// `==highlight==` text foreground (must read against `highlight_bg`).
    pub highlight_fg: String,
    /// `` `code` `` foreground.
    pub code_fg: String,

    // ── todo prefixes ────────────────────────────────────────────
    /// `TODO ` foreground.
    pub todo_open_fg: String,
    /// `DONE ` foreground.
    pub todo_done_fg: String,
    /// Body text foreground of a DONE block (struck through).
    pub todo_done_body_fg: String,

    // ── structural ────────────────────────────────────────────────
    /// `key:: value` — the key part.
    pub property_key_fg: String,
    /// `key:: value` — the value part.
    pub property_value_fg: String,
    /// Page heading in the header.
    pub heading_fg: String,
    /// Delimiter dim (`**`, `~~`, `` ` ``) in raw render.
    pub dim_fg: String,

    // ── selection / cursor ────────────────────────────────────────
    /// Selected bullet — background.
    pub selected_bullet_bg: String,
    /// Selected bullet — foreground.
    pub selected_bullet_fg: String,
    /// Block-style cursor (vim Normal mode) — background.
    pub cursor_block_bg: String,
    /// Block-style cursor — foreground.
    pub cursor_block_fg: String,
    /// Thin caret (Insert mode, end-of-line).
    pub cursor_caret_fg: String,

    // ── chrome ────────────────────────────────────────────────────
    /// Normal-mode badge background.
    pub status_normal_bg: String,
    /// Normal-mode badge foreground.
    pub status_normal_fg: String,
    /// Insert-mode badge background.
    pub status_insert_bg: String,
    /// Insert-mode badge foreground.
    pub status_insert_fg: String,
    /// Visual-mode badge background.
    pub status_visual_bg: String,
    /// Visual-mode badge foreground.
    pub status_visual_fg: String,
    /// Transient status message foreground.
    pub status_message_fg: String,
    /// Highlighted entry in popup lists.
    pub list_selected_bg: String,
    /// Foreground of the highlighted entry.
    pub list_selected_fg: String,
    /// Section title in the help overlay.
    pub help_title_fg: String,
}

impl Palette {
    /// Iterator over `(field_name, hex_value)` pairs. Used by tests
    /// to assert every field stays a valid hex string, and by the
    /// desktop's CSS-variable installer to walk the palette without
    /// hard-coding each name.
    pub fn fields(&self) -> Vec<(&'static str, &str)> {
        vec![
            ("bg", &self.bg),
            ("bg_elev", &self.bg_elev),
            ("fg", &self.fg),
            ("fg_dim", &self.fg_dim),
            ("fg_dimmer", &self.fg_dimmer),
            ("border", &self.border),
            ("hint", &self.hint),
            ("accent", &self.accent),
            ("accent_soft", &self.accent_soft),
            ("accent_alt", &self.accent_alt),
            ("warn", &self.warn),
            ("destructive", &self.destructive),
            ("ref_link_fg", &self.ref_link_fg),
            ("tag_link_fg", &self.tag_link_fg),
            ("md_link_fg", &self.md_link_fg),
            ("bold_fg", &self.bold_fg),
            ("italic_fg", &self.italic_fg),
            ("strike_fg", &self.strike_fg),
            ("highlight_bg", &self.highlight_bg),
            ("highlight_fg", &self.highlight_fg),
            ("code_fg", &self.code_fg),
            ("todo_open_fg", &self.todo_open_fg),
            ("todo_done_fg", &self.todo_done_fg),
            ("todo_done_body_fg", &self.todo_done_body_fg),
            ("property_key_fg", &self.property_key_fg),
            ("property_value_fg", &self.property_value_fg),
            ("heading_fg", &self.heading_fg),
            ("dim_fg", &self.dim_fg),
            ("selected_bullet_bg", &self.selected_bullet_bg),
            ("selected_bullet_fg", &self.selected_bullet_fg),
            ("cursor_block_bg", &self.cursor_block_bg),
            ("cursor_block_fg", &self.cursor_block_fg),
            ("cursor_caret_fg", &self.cursor_caret_fg),
            ("status_normal_bg", &self.status_normal_bg),
            ("status_normal_fg", &self.status_normal_fg),
            ("status_insert_bg", &self.status_insert_bg),
            ("status_insert_fg", &self.status_insert_fg),
            ("status_visual_bg", &self.status_visual_bg),
            ("status_visual_fg", &self.status_visual_fg),
            ("status_message_fg", &self.status_message_fg),
            ("list_selected_bg", &self.list_selected_bg),
            ("list_selected_fg", &self.list_selected_fg),
            ("help_title_fg", &self.help_title_fg),
        ]
    }

    /// Whether this palette reads as a light theme, by BT.601
    /// perceived luminance over [`Palette::bg`].
    ///
    /// The Rust-side owner of that question, but not the only
    /// implementation: it is a method, never a wire field, so it
    /// cannot cross the Tauri bridge. Its one non-test caller today
    /// is `outl doctor`'s light/dark pairing check
    /// (`crates/outl-cli/src/cmd/doctor/theme.rs`), which validates
    /// a configured `[theme]` pair has one light side and one dark
    /// side. `mode = "auto"` does **not** call this — the TUI's
    /// `resolve_preset_name` (`crates/outl-tui/src/runtime.rs`)
    /// matches on `ThemeMode` directly and always resolves `Auto` to
    /// the dark side (a terminal can't read the OS setting), and no
    /// GUI client resolves `auto` in Rust at all yet (see root
    /// `CLAUDE.md`'s theming decisions). The desktop's `color-scheme`
    /// CSS comes from `isLightHex` in
    /// `crates/outl-frontend-shared/src/theme/palette.ts`, a
    /// hand-synced client-side copy of this same luminance check —
    /// not this method, since nothing here reaches the DOM.
    ///
    /// A malformed hex is treated as dark, matching the boot default.
    pub fn is_light(&self) -> bool {
        match parse_hex(&self.bg) {
            Some((r, g, b)) => {
                0.299 * f64::from(r) + 0.587 * f64::from(g) + 0.114 * f64::from(b) > 128.0
            }
            None => false,
        }
    }
}

/// Parse `#rrggbb` (and `#rrggbbaa`) into raw `(r, g, b)` bytes.
/// Returns `None` on malformed input.
///
/// Lives here so every client uses the exact same parsing —
/// `ratatui::Color::Rgb(r, g, b)` and a CSS injection both lean on
/// it. Alpha is ignored (terminals don't have one; the desktop
/// uses Tailwind opacity utilities).
pub fn parse_hex(s: &str) -> Option<(u8, u8, u8)> {
    let bytes = s.strip_prefix('#')?.as_bytes();
    if bytes.len() != 6 && bytes.len() != 8 {
        return None;
    }
    let hex = |b: &u8| -> Option<u8> {
        match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            b'A'..=b'F' => Some(b - b'A' + 10),
            _ => None,
        }
    };
    let byte = |hi: &u8, lo: &u8| -> Option<u8> { Some(hex(hi)? * 16 + hex(lo)?) };
    Some((
        byte(&bytes[0], &bytes[1])?,
        byte(&bytes[2], &bytes[3])?,
        byte(&bytes[4], &bytes[5])?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hex_roundtrips_known_values() {
        assert_eq!(parse_hex("#000000"), Some((0, 0, 0)));
        assert_eq!(parse_hex("#ffffff"), Some((255, 255, 255)));
        assert_eq!(parse_hex("#a78bfa"), Some((167, 139, 250)));
        assert_eq!(parse_hex("#A78BFA"), Some((167, 139, 250)));
    }

    #[test]
    fn parse_hex_accepts_alpha_but_ignores_it() {
        assert_eq!(parse_hex("#a78bfaff"), Some((167, 139, 250)));
    }

    #[test]
    fn parse_hex_rejects_malformed() {
        assert!(parse_hex("a78bfa").is_none(), "missing # prefix");
        assert!(parse_hex("#xyzxyz").is_none(), "non-hex chars");
        assert!(parse_hex("#abc").is_none(), "short form not supported");
    }

    #[test]
    fn palette_round_trips_via_serde() {
        use crate::presets;
        let p = presets::outl();
        let json = serde_json::to_string(&p).unwrap();
        let back: Palette = serde_json::from_str(&json).unwrap();
        assert_eq!(back.name, p.name);
        assert_eq!(back.accent, p.accent);
    }

    #[test]
    fn is_light_reads_the_canvas_background() {
        // BT.601 perceived luminance over `bg`. This is the single
        // owner of "is this a light theme" — palette.ts used to have
        // its own copy (`isLightHex`), and two answers to one
        // question is what RFC 0022 exists to remove.
        assert!(crate::presets::light().is_light());
        assert!(crate::presets::logseq_light().is_light());
        assert!(!crate::presets::outl().is_light());
        assert!(!crate::presets::dracula().is_light());
        assert!(!crate::presets::nord().is_light());
    }

    #[test]
    fn every_preset_defines_destructive() {
        // The compiler forces the field to exist. This forces it to
        // mean something: a preset satisfying the struct with ""
        // would paint an invisible delete button.
        for name in crate::PRESETS {
            let p = crate::by_name(name).expect("preset in PRESETS must resolve");
            assert!(
                p.destructive.starts_with('#') && p.destructive.len() == 7,
                "preset {name} has no usable destructive colour: {:?}",
                p.destructive
            );
        }
    }
}
