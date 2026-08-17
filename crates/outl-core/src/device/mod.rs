//! Device-local identity: which [`ActorId`] this machine writes under.
//!
//! The project invariant is "one `ops-<actor>.jsonl` per device, never
//! shared" — it is what makes last-write-wins-per-file harmless on every
//! file transport. Holding the actor id **inside** the workspace
//! (`<root>/.outl/config.toml`) breaks that invariant the moment the
//! workspace directory itself is replicated: Syncthing, Dropbox, NFS, a
//! shared network volume or a `git clone` all carry `.outl/` across, so
//! two devices read the same actor id, each acquires the *machine-local*
//! [`crate::ActorWriteLock`] successfully (`flock(2)` is advisory and
//! never crosses hosts), and both append to the same file. Last write
//! wins and ops are lost with no error anywhere.
//!
//! iCloud Documents happened to hide the bug — it drops dot-prefixed
//! paths, so `.outl/` never travelled — but that is an accident of one
//! transport, not a design.
//!
//! This module is the fix: the actor id lives in a **device store**,
//! a directory outside the workspace.
//!
//! ```text
//! ~/.config/outl/                 ← device_dir()
//! ├── config.toml                 ← user preferences (outl-config)
//! ├── machine-id                  ← this device's id + its host binding
//! ├── actor                       ← device-wide actor (desktop / mobile)
//! └── actors/
//!     ├── <workspace-id>          ← primary binding for that workspace
//!     └── <workspace-id>.<hash>   ← extra binding for a second *copy* of it
//! ```
//!
//! Two granularities, both device-local, both served from here:
//!
//! - [`DeviceStore::actor_for_instance`] keys the actor by
//!   [`WorkspaceId`] **and** the directory the workspace lives in. See
//!   "Why the workspace id alone is not the key" below.
//! - [`DeviceStore::device_actor`] is the single device-wide actor the
//!   Tauri clients have always used (`<dir>/actor`). Their
//!   `HlcGenerator` is built at app start, before a workspace is picked,
//!   so they cannot key on a workspace id.
//!
//! [`DeviceStore::machine_id`] is the third piece: a device fingerprint
//! that *can* be published into the shared workspace config, so a
//! pre-existing `actor_id` in `config.toml` is claimable by exactly one
//! device. See `outl_ws::actor` for that protocol.
//!
//! ## Why the workspace id alone is not the key
//!
//! [`WorkspaceId`] is persisted at `<root>/.outl/workspace-id`, i.e.
//! **inside** the copied bytes. `cp -R notes notes-backup` produces two
//! directories carrying one workspace id, and keying the actor on the id
//! alone hands both of them the same actor.
//!
//! That is worse than sharing a file. The P2P transport hashes the
//! workspace id into its gossip topic, so the two copies reconcile *as
//! one workspace*; op identity is `Hlc { physical, logical, actor }` and
//! dedup is by `ts`, so two independent `HlcGenerator`s running under one
//! actor mint colliding identities and genuinely distinct ops get dropped
//! as duplicates.
//!
//! So the binding records the workspace **root** next to the actor, and a
//! root that does not match forks a second actor — unless the recorded
//! root is *provably gone*, which is what a plain move or rename looks
//! like and must keep its actor. "Provably gone" means the path no longer
//! exists, or no longer holds this workspace id. Anything we cannot read
//! (unmounted volume, permission error) counts as still live: one extra
//! actor is a wasted file, one shared actor is lost data.
//!
//! ## Why the store itself is fingerprinted
//!
//! `~/.config/outl` is outside the *workspace*, but it is not outside
//! everything. Migration Assistant, a Time Machine restore, a VM or
//! container image, `chezmoi` / Mackup, and `$HOME` on NFS all replicate
//! it, and the clone carries the same `machine-id` — so the claim
//! protocol would report "this machine" on both, and nothing self-heals.
//!
//! [`DeviceStore::machine_id`] therefore binds the id to a host
//! fingerprint derived from an OS-provided identifier of the physical
//! machine (`/etc/machine-id`, `IOPlatformUUID`, `MachineGuid`). When the
//! stored binding and the current host disagree, the store was cloned:
//! the machine id is reminted, every actor binding stamped with the old
//! one reads as stale, and each workspace forks a fresh actor on its next
//! open. Where the platform exposes no such identifier (notably iOS,
//! where a restored device backup is exactly this hazard) the check is
//! inconclusive and deliberately does nothing — forking on every open
//! would be its own bug. That gap closes when the op log itself carries a
//! writer fingerprint; it is not closeable from here.

use std::fmt;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::id::ActorId;
use crate::workspace_id::WorkspaceId;

mod gc;
mod host;
mod record;
#[cfg(test)]
mod tests;

pub use gc::{ActorBinding, BindingVerdict, STALE_BINDING_TTL, STALE_SCRATCH_TTL};

use host::host_fingerprint;
use record::{create_new_record, read_record, write_record};

/// Failure reading or writing the device store.
#[derive(Debug, thiserror::Error)]
pub enum DeviceError {
    /// A device-store file could not be read or written.
    #[error("device store io at {path}: {source}")]
    Io {
        /// Path that failed.
        path: PathBuf,
        /// Underlying io error.
        source: std::io::Error,
    },
}

/// Stable fingerprint of one physical device.
///
/// A ULID generated once and persisted at `<device_dir>/machine-id`,
/// bound to a host fingerprint so a *cloned* store (restored backup,
/// copied VM image, `$HOME` on NFS) remints rather than impersonating the
/// machine it came from.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MachineId(String);

impl MachineId {
    /// The fingerprint as a string — the form written into the workspace
    /// config's claim marker.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for MachineId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Directory holding this device's identity files.
///
/// - `$OUTL_DEVICE_DIR` when set — see below.
/// - macOS / Linux: `$XDG_CONFIG_HOME/outl/`, else `~/.config/outl/`.
/// - Windows: `$XDG_CONFIG_HOME\outl\`, else `%APPDATA%\outl\`.
///
/// XDG-style on macOS is deliberate: outl is CLI-first, and a Mac user
/// dropping into a terminal sees the same path Linux uses. The
/// conventional `~/Library/Application Support/…` would split TUI +
/// desktop into two directories for no benefit. On Windows the
/// `~/.config` layout is not a convention, so the fallback routes
/// through `dirs::config_dir()` (Roaming) instead.
///
/// The base path deliberately matches `outl_config::config_dir` without
/// calling it: this answers "where is this device's *identity*", that
/// one answers "where are the user's *preferences*". `OUTL_DEVICE_DIR`
/// is what keeps them separable — the test suite and any container that
/// needs a throwaway actor rotates identity without also throwing away
/// the user's theme and timezone. If the base layout ever moves, move
/// both.
///
/// Total by design (a last-resort relative path rather than an
/// `Option`): every supported OS has a home directory, and forcing
/// every call site to handle a case that cannot happen buys nothing.
pub fn device_dir() -> PathBuf {
    if let Ok(custom) = std::env::var("OUTL_DEVICE_DIR") {
        if !custom.is_empty() {
            return PathBuf::from(custom);
        }
    }
    if let Ok(custom) = std::env::var("XDG_CONFIG_HOME") {
        if !custom.is_empty() {
            return PathBuf::from(custom).join("outl");
        }
    }
    #[cfg(windows)]
    {
        if let Some(roaming) = dirs::config_dir() {
            return roaming.join("outl");
        }
    }
    #[cfg(not(windows))]
    {
        if let Some(home) = dirs::home_dir() {
            return home.join(".config").join("outl");
        }
    }
    PathBuf::from(".config").join("outl")
}

/// Device-local mapping from a workspace *instance* to the actor this
/// machine writes under.
///
/// Rooted at an explicit directory so tests (and the mobile client,
/// whose sandbox dictates the path) can point it somewhere else;
/// [`DeviceStore::open_default`] uses [`device_dir`].
#[derive(Clone, Debug)]
pub struct DeviceStore {
    dir: PathBuf,
}

impl DeviceStore {
    /// Build a store rooted at `dir`. The directory is created lazily on
    /// the first write, so constructing one is infallible and cheap.
    pub fn at(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// Build a store rooted at [`device_dir`].
    pub fn open_default() -> Self {
        Self::at(device_dir())
    }

    /// Root directory of the store (for diagnostics).
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// This device's fingerprint, generated and persisted on first call.
    ///
    /// Reminted when the persisted host binding disagrees with the
    /// current machine — see the module doc, "Why the store itself is
    /// fingerprinted".
    ///
    /// Minting is compare-and-swap, for the same reason an actor binding is
    /// (see `bind`): two processes racing the *first* read of one store must
    /// converge on one id rather than each stamping its own.
    ///
    /// It matters more here than for a binding. A forked actor costs one
    /// extra ops file, but this id arbitrates every actor binding **and**
    /// every `actor_claimed_by` claim, so a loser that overwrote the winner
    /// makes all of the winner's already-written claims read as foreign —
    /// and a claim lives in the workspace's `config.toml`, so that
    /// workspace never adopts its own legacy ops file again.
    pub fn machine_id(&self) -> Result<MachineId, DeviceError> {
        let path = self.dir.join("machine-id");
        let host = host_fingerprint();
        let stored = read_record(&path)?;
        let existing = stored
            .as_ref()
            .and_then(|r| r.get("id"))
            .map(str::to_string);
        let bound = stored.as_ref().and_then(|r| r.get("host"));

        if let Some(id) = existing.as_deref() {
            match (bound, host) {
                (Some(was), Some(now)) if was != now => {
                    tracing::warn!(
                        "device store at {} was cloned from another machine; reminting the \
                         device id (every workspace forks a fresh actor)",
                        self.dir.display()
                    );
                }
                (None, Some(now)) => {
                    // Pre-binding store on this same machine: stamp it so
                    // the next clone is detectable.
                    write_record(&path, &[("id", id), ("host", now)])?;
                    return Ok(MachineId(id.to_string()));
                }
                _ => return Ok(MachineId(id.to_string())),
            }
        }
        let fresh = ulid::Ulid::new().to_string();
        let record: Vec<(&str, &str)> = match host {
            Some(now) => vec![("id", fresh.as_str()), ("host", now)],
            None => vec![("id", fresh.as_str())],
        };
        if existing.is_some() {
            // Reminting a clone: the file is there on purpose and this
            // process is the only one that decided to replace it.
            write_record(&path, &record)?;
            return Ok(MachineId(fresh));
        }
        // First open of this store: create-if-absent, so a concurrent first
        // open adopts whichever id landed instead of overwriting it.
        match create_new_record(&path, &record) {
            Ok(()) => Ok(MachineId(fresh)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                match read_record(&path)?.and_then(|r| r.get("id").map(str::to_string)) {
                    Some(id) => Ok(MachineId(id)),
                    // Present but unreadable as a record — a torn write, or
                    // the winner between its `create` and its `write`. Both
                    // leave nothing to adopt, so claim it.
                    None => {
                        write_record(&path, &record)?;
                        Ok(MachineId(fresh))
                    }
                }
            }
            Err(source) => Err(DeviceError::Io { path, source }),
        }
    }

    /// The actor this device writes under for the workspace identified by
    /// `workspace` and living at `root`, binding `fallback` when this
    /// device has no usable binding yet.
    ///
    /// The caller owns the *migration* decision — whether `fallback` is a
    /// legacy `actor_id` it is entitled to adopt or a brand-new
    /// [`ActorId`] — because that answer lives in the workspace config,
    /// which this crate does not parse. See `outl_ws::actor`.
    /// `fallback` is used only when this device has **no** binding for the
    /// workspace at all; a second live copy always forks a fresh actor,
    /// since both copies read `fallback` out of the same config bytes.
    ///
    /// Binding is compare-and-swap: two processes racing a first open
    /// converge on whichever wrote first, rather than each minting its
    /// own. The loser's per-actor `flock` then hands it an ephemeral
    /// actor through [`crate::resolve_write_actor`], which is the
    /// existing multi-process contract.
    pub fn actor_for_instance(
        &self,
        workspace: &WorkspaceId,
        root: &Path,
        fallback: ActorId,
    ) -> Result<ActorId, DeviceError> {
        let machine = self.machine_id()?;
        let root = canonical(root);
        let actors = self.dir.join("actors");
        let primary = actors.join(workspace_key(workspace));
        let instance = actors.join(instance_key(workspace, &root));

        // An instance file exists only for a second copy of one workspace
        // on this device, and is keyed by that copy's own path.
        if let Some((actor, _)) = read_binding(&instance, &machine)? {
            return Ok(actor);
        }

        // Absent, or stamped by a machine id this store no longer owns (a
        // clone that has since reminted): this device has no claim on it.
        let Some((actor, bound_root)) = read_binding(&primary, &machine)? else {
            return bind(&primary, actor_record(fallback, &root, &machine));
        };

        match bound_root {
            // Pre-`root` binding: adopt and upgrade the record in place.
            None => {
                write_record(&primary, &actor_record(actor, &root, &machine))?;
                Ok(actor)
            }
            Some(bound) if bound == root => Ok(actor),
            Some(bound) if prior_instance_is_gone(&bound, workspace) => {
                // Moved or renamed: the same workspace at a new path must
                // keep writing to the ops file it has always used.
                write_record(&primary, &actor_record(actor, &root, &machine))?;
                Ok(actor)
            }
            Some(bound) => {
                // A second live copy. `fallback` is not usable here: it is
                // whatever the *workspace config* offers, and both copies
                // carry the same config bytes — adopting it would hand
                // both instances one actor, which is the collision this
                // whole branch exists to prevent.
                tracing::warn!(
                    "workspace {workspace} is also present at {} on this device; {} writes \
                     under its own actor",
                    bound.display(),
                    root.display()
                );
                bind(&instance, actor_record(ActorId::new(), &root, &machine))
            }
        }
    }

    /// The device-wide actor at `<dir>/actor`, generated and persisted on
    /// first call.
    ///
    /// Used by the Tauri clients, whose `HlcGenerator` is bound at app
    /// start — before any workspace is chosen — so they cannot key on a
    /// [`WorkspaceId`]. Reminted when the store turns out to have been
    /// cloned onto another machine, for the same reason
    /// [`DeviceStore::actor_for_instance`] forks there.
    pub fn device_actor(&self) -> Result<ActorId, DeviceError> {
        let machine = self.machine_id()?;
        let path = self.dir.join("actor");
        if let Some(rec) = read_record(&path)? {
            if let Some(actor) = rec.get("actor").and_then(|raw| parse_actor(raw, &path)) {
                match rec.get("machine") {
                    Some(m) if m == machine.as_str() => return Ok(actor),
                    // Legacy file (the Tauri clients wrote a bare ULID):
                    // device-local by construction, so keep it, and stamp
                    // it so a later clone is detectable.
                    None => {
                        write_record(&path, &stamped(actor, &machine))?;
                        return Ok(actor);
                    }
                    Some(_) => {}
                }
            }
        }
        let fresh = ActorId::new();
        write_record(&path, &stamped(fresh, &machine))?;
        Ok(fresh)
    }
}

/// The three fields of an `actors/*` binding.
fn actor_record(actor: ActorId, root: &Path, machine: &MachineId) -> [(&'static str, String); 3] {
    [
        ("actor", actor.to_string()),
        ("root", root.display().to_string()),
        ("machine", machine.as_str().to_string()),
    ]
}

/// The two fields of the device-wide `actor` file.
fn stamped(actor: ActorId, machine: &MachineId) -> [(&'static str, String); 2] {
    [
        ("actor", actor.to_string()),
        ("machine", machine.as_str().to_string()),
    ]
}

/// Read a binding, treating one stamped by a foreign machine id as
/// absent. Returns the actor and the root it was bound to.
#[allow(clippy::type_complexity)]
fn read_binding(
    path: &Path,
    machine: &MachineId,
) -> Result<Option<(ActorId, Option<PathBuf>)>, DeviceError> {
    let Some(rec) = read_record(path)? else {
        return Ok(None);
    };
    if rec.get("machine").is_some_and(|m| m != machine.as_str()) {
        return Ok(None);
    }
    Ok(rec
        .get("actor")
        .and_then(|raw| parse_actor(raw, path))
        .map(|actor| (actor, rec.get("root").map(PathBuf::from))))
}

/// Create `path` with `record` if it does not exist, else adopt whatever
/// is already there. The compare-and-swap that keeps two processes racing
/// a first open from minting two actors for one workspace.
fn bind(path: &Path, record: [(&'static str, String); 3]) -> Result<ActorId, DeviceError> {
    let wanted = record[0].1.clone();
    let machine = record[2].1.clone();
    match create_new_record(path, &record) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            // Adopt only a record this device may actually use — a
            // concurrent first open on this machine. One stamped by
            // another machine (a cloned store that has since reminted) is
            // exactly what we are here to replace.
            if let Some(actor) = read_record(path)?
                .filter(|rec| rec.get("machine").is_none_or(|m| m == machine))
                .and_then(|rec| rec.get("actor").and_then(|raw| parse_actor(raw, path)))
            {
                return Ok(actor);
            }
            write_record(path, &record)?;
        }
        Err(source) => {
            return Err(DeviceError::Io {
                path: path.to_path_buf(),
                source,
            })
        }
    }
    parse_actor(&wanted, path).ok_or_else(|| DeviceError::Io {
        path: path.to_path_buf(),
        source: std::io::Error::new(std::io::ErrorKind::InvalidData, "unparseable actor id"),
    })
}

/// Whether the workspace that was bound at `path` is provably no longer
/// there. Conservative: anything unreadable counts as *still live*, so an
/// unmounted volume forks an extra actor instead of sharing one.
fn prior_instance_is_gone(path: &Path, workspace: &WorkspaceId) -> bool {
    match path.try_exists() {
        Ok(false) => true,
        Ok(true) => match std::fs::read_to_string(WorkspaceId::path_for(path)) {
            Ok(contents) => contents.trim() != workspace.as_str(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => true,
            Err(_) => false,
        },
        Err(_) => false,
    }
}

fn canonical(root: &Path) -> PathBuf {
    std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf())
}

/// Filename-safe key for a workspace id.
///
/// A [`WorkspaceId`] is normally a ULID, and using it verbatim keeps the
/// store readable. But [`WorkspaceId::from_raw`] accepts whatever a
/// pairing host sent over the wire, so an id containing `/` or `..`
/// would escape the store directory. Anything not plainly safe is
/// hashed instead.
fn workspace_key(workspace: &WorkspaceId) -> String {
    let raw = workspace.as_str();
    let plain = !raw.is_empty()
        && raw.len() <= 64
        && raw
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    if plain {
        return raw.to_string();
    }
    format!("h-{}", hash_hex(raw.as_bytes()))
}

/// Key for a *second* copy of one workspace on this device. The primary
/// key stays path-free so a plain move or rename keeps its actor; extra
/// copies get their own slot, keyed by the path that distinguishes them.
fn instance_key(workspace: &WorkspaceId, root: &Path) -> String {
    format!(
        "{}.{}",
        workspace_key(workspace),
        hash_hex(root.display().to_string().as_bytes())
    )
}

fn hash_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest[..16].iter().map(|b| format!("{b:02x}")).collect()
}

fn parse_actor(raw: &str, path: &Path) -> Option<ActorId> {
    match ulid::Ulid::from_string(raw.trim()) {
        Ok(u) => Some(ActorId(u)),
        Err(_) => {
            tracing::warn!("ignoring unparseable actor id in {}", path.display());
            None
        }
    }
}
