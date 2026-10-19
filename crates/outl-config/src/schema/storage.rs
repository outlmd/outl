//! The workspace's bytes on this device: where they are, how many of
//! them stay resident, and what protects them.
//!
//! Every section here is read by the persistence layer, never by a
//! renderer — the directory the client opens, the op-log cache cap, when
//! a boot snapshot lands, the ceiling on one uploaded file, and whether a
//! local git backup is taken.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Workspace section — primarily where the desktop remembers the
/// last opened directory so the next launch skips the picker.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WorkspaceCfg {
    /// Absolute path to the last workspace the user opened. The
    /// desktop writes this on every `set_workspace` call; the TUI
    /// can read it as a fallback when no `--path` flag was given.
    pub last: Option<PathBuf>,
}

/// Storage section — controls `JsonlStorage`'s in-memory footprint
/// (RFC #137). The op-log cache is a bounded LRU; ops evicted from
/// RAM are addressable through the per-actor offset index. This keeps
/// RSS roughly constant regardless of how much history the workspace
/// has accumulated.
///
/// Defaults are conservative: 20k ops ≈ 4 MB of cache on the desktop,
/// enough for the home page plus its backlinks; mobile pins to 5k
/// (≈ 1 MB) at boot to stay well under iOS jetsam.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StorageCfg {
    /// Maximum number of ops held in RAM per `JsonlStorage` instance.
    /// `0` is treated as "unbounded" (legacy behaviour — every op
    /// stays resident). Anything `> 0` enforces the LRU cap.
    pub lru_cap: usize,
}

impl Default for StorageCfg {
    fn default() -> Self {
        Self { lru_cap: 20_000 }
    }
}

/// Snapshot section — controls when long-lived clients persist a
/// materialized-state snapshot of the workspace to disk.
///
/// Snapshots are a boot cache: `Workspace::open_with_storage` loads
/// one (if present and its `content_hash` matches) and replays only
/// the ops posted after its cutoff, instead of walking the entire
/// per-actor op log. This is the fix for issue #109 (CLI pays full
/// replay on every invocation).
///
/// The CLI never writes snapshots — it's ephemeral — but reads any
/// produced by a long-lived client (TUI / desktop / mobile) so it
/// still gets the speedup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SnapshotCfg {
    /// Master switch. `false` makes `Workspace::apply` skip the
    /// snapshot-trigger check entirely. The CLI sets this to `false`
    /// in-memory regardless of the TOML.
    pub enabled: bool,

    /// How many ops a long-lived client applies between snapshot
    /// writes. Lower values shrink the post-snapshot delta (faster
    /// boot) at the cost of more writes; higher values write less
    /// often but leave more ops to replay. The default of `10_000`
    /// is roughly "once per long editing session" for an individual
    /// user — rare enough not to stutter, frequent enough that the
    /// boot delta stays in the hundreds of milliseconds.
    pub op_threshold: u32,
}

impl Default for SnapshotCfg {
    fn default() -> Self {
        Self {
            enabled: true,
            op_threshold: 10_000,
        }
    }
}

/// Assets section — policy for uploaded files (`assets/<hash>.<ext>`).
///
/// The directory itself is fixed at `<workspace>/assets/` (part of the
/// workspace layout, not configurable), so the only policy here is the
/// upper bound on a single upload. A file over the cap is rejected
/// before it is copied, so a fat-fingered drag of a multi-GB file can't
/// balloon the workspace (and, on the P2P transport, every paired
/// device).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AssetsCfg {
    /// Maximum size, in bytes, of a single uploaded file. `0` means
    /// "unbounded". Default is 100 MiB.
    pub max_bytes: u64,
}

impl Default for AssetsCfg {
    fn default() -> Self {
        Self {
            max_bytes: 100 * 1024 * 1024,
        }
    }
}

/// Automatic local backups of the workspace (`outl_actions::backup`).
///
/// **Device-local, like every other preference here.** A backup is a
/// property of *this* machine's disk, not of the workspace: two paired
/// devices each keep their own history, and neither one's git repo
/// travels through the op log or the sync transport — the repository
/// itself lives outside the workspace, under `outl_core::device_dir()`,
/// precisely so no file transport can replicate it.
///
/// Read today by the **TUI** only (`outl_actions::backup::spawn_auto_pass`
/// at startup); the desktop and mobile clients preserve the section
/// verbatim but do not yet run the pass. See
/// [`docs/config.md`](https://github.com/outlmd/outl/blob/main/docs/config.md)
/// → `[backup]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BackupCfg {
    /// Take automatic snapshots. **On by default.**
    ///
    /// The failure this guards against — a projection bug, a mis-aimed
    /// import, a page deleted and the app closed before undo could help
    /// — is exactly the kind a user only discovers later, when the
    /// window to enable a safety net has already closed. Defaulting off
    /// means the feature is present for everyone who didn't need it and
    /// absent for everyone who did.
    ///
    /// Costs nothing on a workspace that never changes (an unchanged
    /// tree produces no commit) and degrades to a `warn!` when there is
    /// no `git` on `PATH`.
    pub enabled: bool,

    /// Minimum minutes between automatic snapshots. Default 30.
    ///
    /// A floor, not a schedule: the background pass wakes on this
    /// cadence and takes a snapshot only when at least this much time
    /// has passed since the newest commit, so a burst of edits never
    /// turns into a burst of commits. The elapsed time is read back out
    /// of the git history, not from a state file, so it survives a
    /// restart.
    pub interval_minutes: u64,
}

impl Default for BackupCfg {
    fn default() -> Self {
        Self {
            enabled: true,
            interval_minutes: 30,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::Config;

    /// The partial-TOML path required when adding a section: a config
    /// that predates `[backup]` must still get the defaults.
    #[test]
    fn backup_section_defaults_when_absent() {
        let c: Config = toml::from_str("[theme]\npreset = \"outl\"\n").unwrap();
        assert!(c.backup.enabled);
        assert_eq!(c.backup.interval_minutes, 30);
    }

    #[test]
    fn backup_section_parses() {
        let c: Config =
            toml::from_str("[backup]\nenabled = false\ninterval_minutes = 120\n").unwrap();
        assert!(!c.backup.enabled);
        assert_eq!(c.backup.interval_minutes, 120);
    }

    #[test]
    fn missing_assets_section_defaults_to_100_mib() {
        let c: Config = toml::from_str("[theme]\npreset = \"nord\"\n").unwrap();
        assert_eq!(c.assets.max_bytes, 100 * 1024 * 1024);
    }

    #[test]
    fn missing_snapshot_section_uses_defaults() {
        // No [snapshot] → enabled=true, op_threshold=10_000 (the
        // long-lived clients' default). A user who never heard of
        // snapshots still gets the boot speedup.
        let c: Config = toml::from_str("").unwrap();
        assert!(c.snapshot.enabled);
        assert_eq!(c.snapshot.op_threshold, 10_000);
    }

    #[test]
    fn snapshot_section_can_disable_writes() {
        // The CLI overrides this in-memory regardless, but a user can
        // also opt out globally by writing `enabled = false`.
        let c: Config = toml::from_str("[snapshot]\nenabled = false\n").unwrap();
        assert!(!c.snapshot.enabled);
        // op_threshold keeps its default.
        assert_eq!(c.snapshot.op_threshold, 10_000);
    }

    #[test]
    fn snapshot_section_can_tune_threshold() {
        let c: Config = toml::from_str("[snapshot]\nop_threshold = 1000\n").unwrap();
        assert_eq!(c.snapshot.op_threshold, 1000);
        // enabled keeps its default.
        assert!(c.snapshot.enabled);
    }
}
