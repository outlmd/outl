//! `[theme]` pair validation: is the light slot actually light, and the
//! dark slot actually dark.
//!
//! RFC 0022 gave [`outl_config::ThemeCfg`] a light/dark preset *pair*
//! (`preset` + `preset_dark`), but nothing in `outl-config` stops a
//! user putting two dark presets in it, or a dark preset in the light
//! slot — `ThemeCfg::dark()` just resolves a name, it never asks
//! whether that name is actually dark (see that struct's doc comment).
//! `outl doctor` is where that gets caught.

use outl_config::ThemeCfg;

use super::Builder;

/// Warn when a `[theme]` pair's light slot holds a dark preset, or its
/// dark slot holds a light one.
///
/// Reports; never errors — a misconfigured theme must not stop a user
/// from reaching their notes.
///
/// A config with no `preset_dark` is not a pair, and that shape is
/// every config written before RFC 0022: flagging it would make
/// `doctor` noisy on first run for every existing user, so it is
/// silently skipped.
///
/// Checked regardless of `mode`: `mode = "light"` resolving to a dark
/// preset is the same misconfiguration wearing a different label, and
/// a user who later flips `mode` inherits whatever the pair already
/// held.
///
/// An unknown preset name in either slot is not this check's job — it
/// silently falls back to [`outl_theme::default`] at render time
/// (`ThemeCfg`'s own doc comment), and that fallback, not a pair
/// mismatch, is what would need reporting.
pub(super) fn check_theme_pair(b: &mut Builder, cfg: &ThemeCfg) {
    let Some(dark_name) = cfg.preset_dark.as_deref() else {
        return;
    };

    if let Some(p) = outl_theme::by_name(&cfg.preset) {
        if !p.is_light() {
            b.warn(format!(
                "[theme] preset = \"{}\" is a dark palette in the light slot. \
                 Swap it with preset_dark, or set mode = \"dark\" to always use the dark side.",
                cfg.preset
            ));
        }
    }
    if let Some(p) = outl_theme::by_name(dark_name) {
        if p.is_light() {
            b.warn(format!(
                "[theme] preset_dark = \"{dark_name}\" is a light palette in the dark slot. \
                 Swap it with preset, or set mode = \"light\" to always use the light side."
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use outl_config::ThemeMode;

    fn builder() -> Builder {
        Builder::new("test".into(), "test".into())
    }

    #[test]
    fn a_configured_pair_has_a_light_side_and_a_dark_side() {
        // Checked for EVERY mode, not just auto: `mode = "light"`
        // resolving to a dark preset is the same misconfiguration
        // wearing a different label.
        for mode in [ThemeMode::Light, ThemeMode::Dark, ThemeMode::Auto] {
            let cfg = ThemeCfg {
                preset: "dracula".into(), // dark, in the light slot
                preset_dark: Some("nord".into()),
                mode,
            };
            let mut b = builder();
            check_theme_pair(&mut b, &cfg);
            assert_eq!(b.findings.len(), 1, "{mode:?}: {:?}", b.findings);
            assert!(
                b.findings[0].message.contains("preset"),
                "{:?}",
                b.findings[0]
            );
        }
    }

    #[test]
    fn a_config_without_a_pair_is_never_flagged() {
        // Every pre-RFC-0022 config looks like this, and they are all
        // still correct. Flagging them would turn `doctor` into noise
        // on first run for every existing user.
        let cfg = ThemeCfg {
            preset: "dracula".into(),
            preset_dark: None,
            mode: ThemeMode::Auto,
        };
        let mut b = builder();
        check_theme_pair(&mut b, &cfg);
        assert!(b.findings.is_empty());
    }

    #[test]
    fn a_well_formed_pair_is_silent() {
        let cfg = ThemeCfg {
            preset: "logseq-light".into(),
            preset_dark: Some("outl".into()),
            mode: ThemeMode::Auto,
        };
        let mut b = builder();
        check_theme_pair(&mut b, &cfg);
        assert!(b.findings.is_empty());
    }
}
