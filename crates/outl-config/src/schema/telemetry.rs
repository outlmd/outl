//! The user's consent to crash reporting.

use serde::{Deserialize, Serialize};

/// Crash reporting (`outl-telemetry`).
///
/// Device-local like every other section here, and read at process start
/// — before a workspace exists, which is what rules the workspace out as
/// a home for it. This flag is one of four conditions; the other three
/// belong to the build and the environment. See
/// [RFC 0377](https://github.com/outlmd/outl/blob/main/docs/rfcs/0377-crash-reporting.md).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TelemetryCfg {
    /// Send crash reports. **On by default** — a reporter nobody turns
    /// on answers nothing, and the user it exists for is the one who
    /// crashes and never files an issue. Never carries note content.
    pub enabled: bool,
}

impl Default for TelemetryCfg {
    fn default() -> Self {
        Self { enabled: true }
    }
}

#[cfg(test)]
mod tests {
    use crate::Config;

    #[test]
    fn telemetry_is_on_by_default() {
        let c: Config = toml::from_str("").unwrap();
        assert!(c.telemetry.enabled);
    }

    /// The partial-TOML path required when adding a section: a config
    /// written before `[telemetry]` existed must still get the default.
    #[test]
    fn telemetry_section_defaults_when_absent() {
        let c: Config = toml::from_str("[theme]\npreset = \"outl\"\n").unwrap();
        assert!(c.telemetry.enabled);
    }

    #[test]
    fn telemetry_can_be_switched_off() {
        let c: Config = toml::from_str("[telemetry]\nenabled = false\n").unwrap();
        assert!(!c.telemetry.enabled);
    }
}
