//! Contrast, which no other test in this crate can see.
//!
//! `tokens.rs` answers structural questions: does this token name have a
//! `Palette` field behind it, does a component use one that does not.
//! A field can pass every one of those and still be unreadable, because
//! none of them knows which two fields get painted on top of each other.
//!
//! This file owns that question, and it is split out rather than living
//! beside them because it needs a different toolkit (WCAG relative
//! luminance) and a different kind of exemption list (a preset to fix,
//! not a name to rename).

/// Every `_bg` / `_fg` pair in every preset clears WCAG AA (4.5:1).
///
/// A palette field can be a valid hex, have a `@theme` token, be used by
/// a real component, and still be unreadable. None of the other tests in
/// this file can see that, because none of them knows which two fields
/// are painted on top of each other.
///
/// This is not hypothetical either. `text-white` on `bg-(--color-outl-accent)`
/// shipped on twelve mobile surfaces and failed AA on **seven of the ten
/// presets**, including `outl`, the default: 2.72:1, which misses even
/// the 3.0 large-text floor, and 1.38:1 on `dracula`. Nothing caught it
/// because "white" is not a palette field, so no test was looking.
///
/// `accent` is checked against `bg` rather than an `accent_fg`, and that
/// asymmetry is deliberate: an accent is chosen to contrast with the
/// canvas, so the canvas reads on it, which the numbers bear out in nine
/// of ten presets. `destructive` is red at mid-luminance and had no such
/// luck, so it carries a real `destructive_fg`.
///
/// **`monokai` is the documented exception.** Its accent `#f92672`
/// against its own `bg` is 3.93:1 — over the 3.0 large-text floor, under
/// AA. That is a flaw in the preset's accent, not in the token model, so
/// it is exempted by name rather than papered over by lowering the bar
/// for everyone.
#[test]
fn every_fill_pair_passes_contrast() {
    /// WCAG AA for normal-size text.
    const AA: f64 = 4.5;

    /// `(preset, pair)` combinations that do not clear AA today, each
    /// with the reason. A row here is a preset to fix, not a rule to
    /// relax — and it may only ever get shorter.
    const KNOWN_BELOW_AA: &[(&str, &str, &str)] = &[
        // Monokai's pink accent `#f92672` is 3.93:1 on its own canvas,
        // and the preset reuses `accent` as the background for three
        // more roles, so one root cause shows up four times. Over the
        // 3.0 large-text floor, under AA. Fixing it means picking a new
        // Monokai accent, which is a preset decision, not a token one.
        (
            "monokai",
            "accent on bg",
            "accent #f92672 is 3.93:1 on bg #272822",
        ),
        (
            "monokai",
            "selected_bullet",
            "same accent reused as the fill",
        ),
        ("monokai", "status_normal", "same accent reused as the fill"),
        ("monokai", "list_selected", "same accent reused as the fill"),
        // The visual-mode status background is a mid purple in three
        // presets, and the canvas is what gets painted on it. All three
        // are inherited from the upstream colour scheme.
        (
            "default-dark",
            "status_visual",
            "ANSI magenta #aa00aa is 3.29:1",
        ),
        (
            "solarized-dark",
            "status_visual",
            "Solarized magenta #d33682 is 3.30:1",
        ),
        (
            "nord",
            "status_visual",
            "nord15 #b48ead is 4.41:1, just under",
        ),
        // outl-light is ours, so this one is a real bug rather than an
        // inherited constraint: the lime insert-mode background wants a
        // dark foreground, not the near-white canvas.
        (
            "outl-light",
            "status_insert",
            "lime #65a30d with the canvas on top is 2.83:1",
        ),
    ];

    let mut offenders = Vec::new();
    for name in outl_theme::PRESETS {
        let p = outl_theme::by_name(name).unwrap_or_else(|| panic!("{name} resolves"));
        for (role, bg, fg) in fill_pairs(&p) {
            if KNOWN_BELOW_AA
                .iter()
                .any(|(x, r, _)| *x == *name && *r == role)
            {
                continue;
            }
            let ratio = contrast(bg, fg);
            if ratio < AA {
                offenders.push(format!(
                    "{name}: {role} — {fg} on {bg} is {ratio:.2}:1, needs {AA}"
                ));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "these fill pairs are unreadable:\n  {}\n\
         A user cannot read text painted on a background it does not \
         contrast with, whatever the tokens say. Pick a foreground that \
         clears 4.5:1, or change the background. If the pair genuinely \
         cannot be fixed without a preset redesign, add it to \
         KNOWN_BELOW_AA with the reason.",
        offenders.join("\n  ")
    );

    // A stale exemption hides the next real one.
    for (preset, role, _why) in KNOWN_BELOW_AA {
        let p = outl_theme::by_name(preset).unwrap_or_else(|| panic!("{preset} resolves"));
        let (_, bg, fg) = fill_pairs(&p)
            .into_iter()
            .find(|(r, _, _)| r == role)
            .unwrap_or_else(|| panic!("KNOWN_BELOW_AA names an unchecked role: {role}"));
        assert!(
            contrast(bg, fg) < AA,
            "{preset} / {role} now clears AA — drop its KNOWN_BELOW_AA row"
        );
    }
}

/// Every `(role, background, foreground)` a preset paints on top of
/// itself.
///
/// The single owner of that mapping. It used to exist twice — once as
/// the list the check walks and once as a `match` the stale-exemption
/// loop used to resolve a role name back to its two fields — and keeping
/// two copies of one fact in step by hand is the thing this crate's own
/// tests exist to prevent.
fn fill_pairs(p: &outl_theme::Palette) -> Vec<(&'static str, &str, &str)> {
    vec![
        ("highlight", &p.highlight_bg, &p.highlight_fg),
        (
            "selected_bullet",
            &p.selected_bullet_bg,
            &p.selected_bullet_fg,
        ),
        ("cursor_block", &p.cursor_block_bg, &p.cursor_block_fg),
        ("status_normal", &p.status_normal_bg, &p.status_normal_fg),
        ("status_insert", &p.status_insert_bg, &p.status_insert_fg),
        ("status_visual", &p.status_visual_bg, &p.status_visual_fg),
        ("list_selected", &p.list_selected_bg, &p.list_selected_fg),
        // A fill with its own foreground.
        ("destructive", &p.destructive, &p.destructive_fg),
        // A fill whose foreground is the canvas — see `destructive_fg`'s
        // doc for why `accent` is unpaired.
        ("accent on bg", &p.accent, &p.bg),
    ]
}

/// WCAG 2.1 relative luminance.
fn relative_luminance(hex: &str) -> f64 {
    let hex = hex.trim_start_matches('#');
    let channel = |i: usize| {
        let v = u8::from_str_radix(&hex[i..i + 2], 16)
            .unwrap_or_else(|e| panic!("{hex} is not a hex colour: {e}")) as f64
            / 255.0;
        if v <= 0.040_45 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(0) + 0.7152 * channel(2) + 0.0722 * channel(4)
}

/// WCAG 2.1 contrast ratio, 1.0 (identical) to 21.0 (black on white).
fn contrast(a: &str, b: &str) -> f64 {
    let (la, lb) = (relative_luminance(a), relative_luminance(b));
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}
