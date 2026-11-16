//! The theme file format: one `.toml` per family, `[dark]` / `[light]`
//! inside it, every name derived from the filename and the section.
//!
//! This module is the **single owner of what a theme file is**, and it
//! answers that question identically for a built-in theme (embedded with
//! `include_str!`, see [`crate::registry`]) and for a user's theme in
//! `~/.config/outl/themes/`. That is the whole reason the built-ins are
//! TOML rather than Rust: a second definition would mean two parsers, two
//! validators, and an `extends = "dracula"` in a user's file having to
//! reach a Rust `fn`. The formats would drift, and the drift would reach
//! the user as "my theme looks different from the built-in it extends".
//!
//! ## The format
//!
//! ```toml
//! # themes/gruvbox.toml
//! [dark]
//! bg = "#282828"
//! fg = "#ebdbb2"
//! # … every Palette field
//!
//! [light]
//! bg = "#fbf1c7"   # inherits [dark], so only the delta is written
//! fg = "#3c3836"
//! ```
//!
//! ## What is derived, never written
//!
//! - **`name`** comes from the filename plus the section. Writing it is an
//!   error rather than an override, because two spellings of one name is
//!   exactly the drift this module exists to remove.
//! - **The `-dark` / `-light` suffix** and the bare family name. A family
//!   with both sides resolves `<family>` (dark), `<family>-dark` and
//!   `<family>-light`; a family with one side resolves `<family>` too, so
//!   a single-sided theme never needs a suffix to be reachable.
//!
//! ## Inheritance
//!
//! `[light]` inherits `[dark]` by default, which is the ordinary case: a
//! light side is its dark sibling with ten fields changed. `extends =
//! "<theme>"` inherits from another theme instead, and is what a user's
//! file uses to start from a built-in.
//!
//! A field with nothing to inherit from is an **error that names the
//! field**, never a default. A theme is not a surface that can be
//! half-painted: a missing `fg` is invisible text, and silence there is
//! the defect.

use crate::Palette;

/// One resolved palette plus every name that reaches it.
///
/// `canonical` is what a picker shows and what `Palette::name` holds;
/// `aliases` are the extra spellings [`crate::by_name`] accepts.
#[derive(Debug, Clone)]
pub struct Entry {
    /// The user-visible name, and the value of `palette.name`.
    pub canonical: String,
    /// Additional names that resolve to this palette.
    pub aliases: Vec<String>,
    /// The fully resolved palette.
    pub palette: Palette,
}

/// Why a theme file could not be turned into palettes.
///
/// Every variant names the thing that is wrong — the field, the base, the
/// value. A theme that fails to load is reported, never swapped silently
/// for a default: a user who typo'd one hex gets told which one, not a
/// different theme than the one they asked for.
#[derive(Debug, thiserror::Error)]
pub enum ThemeError {
    /// The file could not be read at all.
    ///
    /// Its own variant rather than an empty source fed through the parser:
    /// that reported a permissions error as "has no `[dark]` or `[light]`
    /// section", sending the user to look for a typo in bytes nobody could
    /// read.
    #[error("theme `{family}` could not be read: {source}")]
    Unreadable {
        family: String,
        #[source]
        source: std::io::Error,
    },

    /// The file is not valid TOML.
    #[error("theme `{family}` is not valid TOML: {source}")]
    Toml {
        family: String,
        #[source]
        source: toml::de::Error,
    },

    /// No `[dark]` and no `[light]`, so there is nothing to resolve.
    #[error("theme `{family}` has no [dark] or [light] section")]
    NoSides { family: String },

    /// A top-level key that is not a side.
    #[error(
        "theme `{family}` has an unexpected top-level key `{key}`; expected [dark] or [light]"
    )]
    UnknownSide { family: String, key: String },

    /// A section wrote `name`, which is derived from the filename.
    #[error(
        "theme `{family}` writes `name` in its [{side}] section; \
         the name is derived from the file name (`{family}.toml`) and the section"
    )]
    NameIsDerived { family: String, side: String },

    /// `extends` pointed at a theme that does not exist (or is not loaded
    /// yet, which for a cycle amounts to the same thing).
    #[error("theme `{theme}` extends `{base}`, which does not exist")]
    UnknownBase { theme: String, base: String },

    /// A field is missing, or spelled in a way `Palette` does not know.
    #[error("theme `{theme}`: {message}")]
    Field { theme: String, message: String },

    /// A value that is not `#rrggbb` / `#rrggbbaa`.
    #[error("theme `{theme}` field `{field}` is not a hex colour: `{value}`")]
    NotHex {
        theme: String,
        field: String,
        value: String,
    },
}

/// The two sides, in resolution order: `[light]` may inherit `[dark]`, so
/// `[dark]` resolves first.
const SIDES: [&str; 2] = ["dark", "light"];

/// Parse one theme file into its resolved entries.
///
/// `family` is the filename without `.toml` and is the only source of the
/// theme's name. `lookup` resolves an `extends` target against whatever is
/// already loaded; [`crate::registry`] passes the accumulated built-ins
/// (and then the user's own) so a later theme can extend an earlier one.
pub fn parse_family(
    family: &str,
    src: &str,
    lookup: &dyn Fn(&str) -> Option<Palette>,
) -> Result<Vec<Entry>, ThemeError> {
    let doc: toml::Table = toml::from_str(src).map_err(|source| ThemeError::Toml {
        family: family.to_string(),
        source,
    })?;

    for key in doc.keys() {
        if !SIDES.contains(&key.as_str()) {
            return Err(ThemeError::UnknownSide {
                family: family.to_string(),
                key: key.clone(),
            });
        }
    }

    let has_dark = doc.contains_key("dark");
    if !has_dark && !doc.contains_key("light") {
        return Err(ThemeError::NoSides {
            family: family.to_string(),
        });
    }

    let mut entries: Vec<Entry> = Vec::with_capacity(2);
    for side in SIDES {
        let Some(value) = doc.get(side) else { continue };
        let section = value.as_table().ok_or_else(|| ThemeError::Field {
            theme: family.to_string(),
            message: format!("[{side}] is not a table"),
        })?;

        let (canonical, aliases) = names_for(family, side, has_dark);
        // `[dark]` resolves first, so anything already here is the sibling.
        let sibling_dark = entries.first().map(|e: &Entry| e.palette.clone());
        let palette = resolve_side(family, side, &canonical, section, sibling_dark, lookup)?;

        entries.push(Entry {
            canonical,
            aliases,
            palette,
        });
    }

    Ok(entries)
}

/// What one side is called: its canonical name plus every alias.
///
/// The dark side of a family takes the bare family name, which is what
/// every shipped theme already does (`outl` is the dark side of the pair,
/// `outl-light` the light one), and `<family>-dark` is always derived.
/// A family with no dark side gives its light side the suffix and the bare
/// name as an alias, so `logseq` and `logseq-light` both reach it.
fn names_for(family: &str, side: &str, has_dark: bool) -> (String, Vec<String>) {
    if side == "dark" {
        (family.to_string(), vec![format!("{family}-dark")])
    } else if has_dark {
        (format!("{family}-light"), Vec::new())
    } else {
        (format!("{family}-light"), vec![family.to_string()])
    }
}

/// Merge a section over its base and validate the result.
fn resolve_side(
    family: &str,
    side: &str,
    canonical: &str,
    section: &toml::Table,
    sibling_dark: Option<Palette>,
    lookup: &dyn Fn(&str) -> Option<Palette>,
) -> Result<Palette, ThemeError> {
    if section.contains_key("name") {
        return Err(ThemeError::NameIsDerived {
            family: family.to_string(),
            side: side.to_string(),
        });
    }

    let mut overrides = section.clone();
    let extends = match overrides.remove("extends") {
        Some(toml::Value::String(s)) => Some(s),
        Some(other) => {
            return Err(ThemeError::Field {
                theme: canonical.to_string(),
                message: format!("`extends` must be a theme name, got {other}"),
            })
        }
        None => None,
    };

    // An explicit `extends` replaces the sibling default: a light side
    // that names a foreign base meant that base, not its own dark side.
    let base = match extends {
        Some(name) => Some(lookup(&name).ok_or_else(|| ThemeError::UnknownBase {
            theme: canonical.to_string(),
            base: name,
        })?),
        None => sibling_dark,
    };

    let mut merged = match base {
        Some(p) => palette_table(&p, canonical)?,
        None => toml::Table::new(),
    };
    for (k, v) in overrides {
        merged.insert(k, v);
    }
    // Derived, so it is written last and cannot be overridden.
    merged.insert("name".into(), toml::Value::String(canonical.to_string()));

    let palette: Palette = merged
        .try_into()
        .map_err(|e: toml::de::Error| ThemeError::Field {
            theme: canonical.to_string(),
            message: e.message().to_string(),
        })?;

    for (field, value) in palette.fields() {
        if crate::palette::parse_hex(value).is_none() {
            return Err(ThemeError::NotHex {
                theme: canonical.to_string(),
                field: field.to_string(),
                value: value.to_string(),
            });
        }
    }

    Ok(palette)
}

/// A resolved palette as a TOML table, so a section can be merged over it
/// with the same code that merges one section over another.
fn palette_table(p: &Palette, theme: &str) -> Result<toml::Table, ThemeError> {
    match toml::Value::try_from(p) {
        Ok(toml::Value::Table(t)) => Ok(t),
        _ => Err(ThemeError::Field {
            theme: theme.to_string(),
            message: "base palette did not serialise to a table".into(),
        }),
    }
}
