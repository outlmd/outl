//! Theme command bodies — surface the shared `outl-theme` presets
//! to every GUI frontend.
//!
//! Pure functions over `outl_theme`: no `AppHost`, no workspace, no
//! lock. They live here rather than in a client crate because RFC
//! 0022 makes `Palette` the single owner of colour on every client,
//! and a body only the desktop can call is how mobile ended up with
//! hardcoded hex in `styles.css` for three releases.

use outl_theme::Palette;

/// Every built-in palette name, in user-facing order.
pub fn list_themes() -> Vec<String> {
    outl_theme::PRESETS.iter().map(|s| s.to_string()).collect()
}

/// Resolve a palette by name, falling back to the default on an
/// unknown or empty name so malformed config cannot break boot.
pub fn get_theme(name: Option<String>) -> Palette {
    name.as_deref()
        .and_then(outl_theme::by_name)
        .unwrap_or_else(outl_theme::default)
}

/// The `[theme]` section as the frontends need it: which preset for
/// each side of the pair, and which side to use.
///
/// `preset_dark` is resolved here rather than sent as an `Option`,
/// so a client cannot forget the fallback and silently theme-switch
/// a config that only ever set `preset` (RFC 0022's backwards
/// compatibility guarantee).
#[derive(Debug, Clone, serde::Serialize)]
pub struct ThemeConfigDto {
    /// Light side of the pair.
    pub preset: String,
    /// Dark side, already resolved through `ThemeCfg::dark()`.
    pub preset_dark: String,
    /// `"light"` | `"dark"` | `"auto"`.
    pub mode: String,
}

/// Read `[theme]` out of the global config and resolve the pair for
/// a client. Neither GUI client wires `ThemeCfg` today (RFC 0022's
/// "who does not have this" gap) — this is the one command both can
/// call to close it, instead of mobile hardcoding a pair and desktop
/// only ever reading `preset`.
pub fn get_theme_config() -> ThemeConfigDto {
    theme_config_dto(&outl_config::load())
}

/// The pure mapping `Config -> ThemeConfigDto`, split out from
/// [`get_theme_config`] so tests can drive it with a constructed
/// `Config` instead of whatever `config.toml` happens to be on the
/// machine running the suite.
fn theme_config_dto(cfg: &outl_config::Config) -> ThemeConfigDto {
    let mode = match cfg.theme.mode {
        outl_config::ThemeMode::Light => "light",
        outl_config::ThemeMode::Dark => "dark",
        outl_config::ThemeMode::Auto => "auto",
    };
    ThemeConfigDto {
        preset: cfg.theme.preset.clone(),
        preset_dark: cfg.theme.dark().to_string(),
        mode: mode.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_name_falls_back_instead_of_panicking() {
        assert_eq!(get_theme(Some("vampire".into())).name, "outl");
        assert_eq!(get_theme(Some(String::new())).name, "outl");
        assert_eq!(get_theme(None).name, "outl");
    }

    #[test]
    fn list_themes_matches_the_preset_table() {
        assert_eq!(list_themes().len(), outl_theme::PRESETS.len());
    }

    #[test]
    fn a_default_config_yields_the_brand_pair() {
        let dto = theme_config_dto(&outl_config::Config::default());
        assert_eq!(dto.preset, "outl-light");
        assert_eq!(dto.preset_dark, "outl");
        assert_eq!(dto.mode, "auto");
    }

    #[test]
    fn a_configured_pair_keeps_the_two_sides_distinct() {
        let cfg = outl_config::Config {
            theme: outl_config::ThemeCfg {
                preset: "logseq-light".to_string(),
                preset_dark: Some("dracula".to_string()),
                mode: outl_config::ThemeMode::Light,
            },
            ..Default::default()
        };
        let dto = theme_config_dto(&cfg);
        assert_eq!(dto.preset, "logseq-light");
        assert_eq!(dto.preset_dark, "dracula");
        assert_eq!(dto.mode, "light");
        assert_ne!(dto.preset, dto.preset_dark);
    }
}
