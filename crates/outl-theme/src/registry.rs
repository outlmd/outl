//! The catalog: built-in themes embedded in the binary, plus the user's
//! own read from a directory.
//!
//! ## Built-ins live in the binary
//!
//! outl ships as a binary, so a built-in theme cannot be a file the user
//! has to have. Each one is `include_str!`'d at compile time, which makes
//! the TOML part of the executable: nothing to install, nothing to find at
//! runtime, and a `cargo test` failure (not a user's broken launch) when a
//! theme file is malformed.
//!
//! They are TOML rather than Rust for one reason: a user's theme is TOML,
//! and two formats for one thing means two parsers and an `extends =
//! "dracula"` that cannot reach its base. See [`crate::spec`].
//!
//! ## The user's directory is a parameter
//!
//! [`Registry::with_user_dir`] takes the path. This module never resolves
//! `$HOME` or `XDG_CONFIG_HOME` on its own, and that is invariant 9's
//! third question ("how does a test get its own copy?") answered in the
//! signature: a crate that reaches for the real config directory is a
//! crate whose tests write into the developer's `~/.config/outl`. That
//! already happened in 0.11.0 and cost three flaky doctor tests. The
//! client resolves `outl_config::config_dir().join("themes")` and hands it
//! over.
//!
//! ## A theme that fails to load does not resolve
//!
//! Loading returns the errors alongside the registry rather than swallowing
//! them, and a failed theme is absent rather than half-painted. One bad
//! file does not take the others down. The caller is responsible for
//! surfacing the errors — `outl doctor` reports them, and the clients log
//! and carry on with the built-ins.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::OnceLock;

use crate::spec::{parse_family, Entry, ThemeError};
use crate::Palette;

/// The built-in theme files, in the order a picker shows them.
///
/// One line per family. This is the only list of built-ins: [`Registry`]
/// derives every name, every alias and the picker order from the files
/// themselves, so adding a theme is this line plus the `.toml`.
const BUILTIN: &[(&str, &str)] = &[
    ("outl", include_str!("../themes/outl.toml")),
    ("ansi", include_str!("../themes/ansi.toml")),
    ("logseq", include_str!("../themes/logseq.toml")),
    ("dracula", include_str!("../themes/dracula.toml")),
    ("solarized", include_str!("../themes/solarized.toml")),
    ("nord", include_str!("../themes/nord.toml")),
    ("monokai", include_str!("../themes/monokai.toml")),
    ("gruvbox", include_str!("../themes/gruvbox.toml")),
];

/// Names the filename + section derivation does not produce, kept so that
/// no `config.toml` written before this layout breaks.
///
/// `default-dark` and `light` were the two ANSI presets' names before they
/// became the two sides of `ansi`; `dark` and `default` were hand-written
/// aliases. Every other old name is produced by the derivation and needs
/// no row here (`solarized-dark`, `gruvbox-dark` and `logseq` all resolve
/// on their own).
///
/// **This list may only ever get shorter.** A new theme does not get a row:
/// name drift is debt to pay down, not a feature to extend.
/// `every_legacy_name_still_resolves` pins it.
const LEGACY_NAMES: &[(&str, &str)] = &[
    ("default-dark", "ansi"),
    ("dark", "ansi"),
    ("light", "ansi-light"),
    ("default", "outl"),
];

/// A resolved set of themes, looked up by any of their names.
#[derive(Debug, Clone, Default)]
pub struct Registry {
    /// `(canonical name, palette)` in picker order.
    themes: Vec<(String, Palette)>,
    /// Every spelling (canonical + derived alias) → index into `themes`.
    index: BTreeMap<String, usize>,
}

impl Registry {
    /// The built-in themes, parsed once per process.
    ///
    /// A malformed built-in is a bug caught by `cargo test`
    /// (`the_builtin_registry_lists_every_theme_in_picker_order`), never a
    /// user-visible failure, so the errors are dropped here rather than
    /// returned: there is nothing a caller could do about them.
    pub fn builtin() -> &'static Registry {
        static BUILT: OnceLock<Registry> = OnceLock::new();
        BUILT.get_or_init(|| {
            let (reg, errors) = Registry::default().extended_with(BUILTIN);
            debug_assert!(errors.is_empty(), "built-in theme failed: {errors:?}");
            reg
        })
    }

    /// The built-ins plus every `*.toml` in `dir`.
    ///
    /// A missing directory is the normal case and not an error. A user
    /// theme whose canonical name matches a built-in replaces it in place,
    /// keeping the picker order — it is the user's own file, and shadowing
    /// a built-in is a reasonable thing to want.
    ///
    /// Returns the errors rather than logging them: this crate does not
    /// know where a message should go, and a theme that silently failed to
    /// load is the failure mode worth avoiding most.
    pub fn with_user_dir(dir: &Path) -> (Registry, Vec<ThemeError>) {
        let mut sources: Vec<(String, String)> = Vec::new();
        let mut unreadable: Vec<ThemeError> = Vec::new();
        if let Ok(entries) = std::fs::read_dir(dir) {
            let mut paths: Vec<_> = entries
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "toml"))
                .collect();
            // Deterministic order, so an `extends` between two user themes
            // behaves the same on every machine.
            paths.sort();
            for path in paths {
                let Some(family) = path.file_stem().and_then(|s| s.to_str()) else {
                    continue;
                };
                match std::fs::read_to_string(&path) {
                    Ok(src) => sources.push((family.to_string(), src)),
                    Err(source) => unreadable.push(ThemeError::Unreadable {
                        family: family.to_string(),
                        source,
                    }),
                }
            }
        }
        let borrowed: Vec<(&str, &str)> = sources
            .iter()
            .map(|(family, src)| (family.as_str(), src.as_str()))
            .collect();
        let (reg, mut errors) = Registry::builtin().clone().extended_with(&borrowed);
        errors.append(&mut unreadable);
        (reg, errors)
    }

    /// Resolve a theme by any of its names. Case- and separator-insensitive,
    /// so `"Solarized Dark"`, `"solarized_dark"` and `"solarized-dark"` all
    /// land on the same palette.
    pub fn by_name(&self, name: &str) -> Option<&Palette> {
        let norm = normalize(name);
        let slot = self.index.get(&norm).or_else(|| {
            LEGACY_NAMES
                .iter()
                .find(|(from, _)| *from == norm)
                .and_then(|(_, to)| self.index.get(*to))
        })?;
        self.themes.get(*slot).map(|(_, p)| p)
    }

    /// Canonical names, in the order a picker shows them.
    pub fn names(&self) -> Vec<&str> {
        self.themes.iter().map(|(n, _)| n.as_str()).collect()
    }

    /// Every palette, in [`Registry::names`] order.
    pub fn all(&self) -> Vec<&Palette> {
        self.themes.iter().map(|(_, p)| p).collect()
    }

    /// Resolve `sources` on top of what is already here.
    ///
    /// Resolution runs in passes rather than in order, so a theme may
    /// extend one that appears later in the list. A pass that resolves
    /// nothing ends it, and whatever is left could not find its base —
    /// which is the same observation for a missing base and for a cycle,
    /// so both are reported as the base that could not be found.
    fn extended_with(mut self, sources: &[(&str, &str)]) -> (Registry, Vec<ThemeError>) {
        let mut errors = Vec::new();
        // Each pending entry carries the `UnknownBase` its last attempt
        // produced, so a stalled pass reports it without parsing again.
        let mut pending: Vec<(&(&str, &str), ThemeError)> = Vec::new();
        let mut round: Vec<&(&str, &str)> = sources.iter().collect();

        loop {
            let mut progressed = false;
            pending.clear();

            for source in round {
                let (family, src) = source;
                let parsed = {
                    let lookup = |name: &str| self.by_name(name).cloned();
                    parse_family(family, src, &lookup)
                };
                match parsed {
                    Ok(entries) => {
                        for entry in entries {
                            self.insert(entry);
                        }
                        progressed = true;
                    }
                    // Its base may be resolved by a later pass.
                    Err(e @ ThemeError::UnknownBase { .. }) => pending.push((source, e)),
                    Err(e) => {
                        errors.push(e);
                        progressed = true;
                    }
                }
            }

            if pending.is_empty() || !progressed {
                break;
            }
            round = pending.iter().map(|(source, _)| *source).collect();
        }

        // Nothing left can resolve: a missing base and a cycle are the
        // same observation, and both are reported as the base not found.
        errors.extend(pending.into_iter().map(|(_, e)| e));

        (self, errors)
    }

    /// Add one entry, replacing a same-named one in place so the picker
    /// order survives a user theme shadowing a built-in.
    fn insert(&mut self, entry: Entry) {
        let slot = match self.index.get(&entry.canonical) {
            Some(i) => {
                self.themes[*i].1 = entry.palette;
                *i
            }
            None => {
                self.themes.push((entry.canonical.clone(), entry.palette));
                self.themes.len() - 1
            }
        };
        self.index.insert(entry.canonical, slot);
        for alias in entry.aliases {
            // A canonical name always wins over another theme's alias.
            if !self.themes.iter().any(|(n, _)| *n == alias) {
                self.index.insert(alias, slot);
            }
        }
    }
}

/// Lowercase, and `_` / space folded to `-`, so a user's spelling of a
/// theme name does not have to match ours exactly.
fn normalize(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            'A'..='Z' => c.to_ascii_lowercase(),
            '_' | ' ' => '-',
            _ => c,
        })
        .collect()
}
