//! Device-wide election for the one iroh endpoint a device may bind.
//!
//! iroh's relay routes **one endpoint per `node_id`**, and every outl process
//! on a machine shares the same device identity (`~/.outl/identity.key` — one
//! node id per machine, see [`crate::IrohIdentity`]). A second endpoint
//! registering that key *replaces* the active client in the relay's routing
//! table: the demoted endpoint goes inactive, receives nothing, and its
//! outbound catch-up stalls too for any peer that is only reachable through the
//! relay. That is not a benign hijack — it breaks the first endpoint's sync in
//! **both** directions.
//!
//! The previous answer was a policy: only the GUI binds an endpoint, the MCP
//! server and the ephemeral CLI stay passive writers and let a co-resident GUI
//! push their ops out. It kept two endpoints from colliding, but it assumed the
//! GUI exists. On a headless machine (`outl mcp serve` driving an agent, no GUI
//! anywhere) nothing ever binds, so the device is unreachable: its ops never
//! leave and no peer's ops ever arrive. Issue #220.
//!
//! The constraint was never "only the GUI" — it is "one live endpoint per
//! identity". So this module replaces the policy with a mechanism: whoever
//! takes the lease binds the endpoint, everyone else degrades to
//! [`outl_actions::FileSyncTransport`] and converges through the shared `ops/`
//! dir. First process in wins; the lease is released when it exits.
//!
//! The lock file is a **sibling of the identity key**, so the scope follows the
//! node id automatically: the desktop / TUI / CLI / MCP share `~/.outl/` and
//! therefore arbitrate against each other, while mobile (whose identity lives in
//! the app sandbox) has its own lease and never contends.
//!
//! Not a workspace file. The lease is device-local state about a device-local
//! resource, so it must never ride the sync surface — see root `CLAUDE.md`
//! invariant 9.

use std::fmt;
use std::fs::File;
use std::path::{Path, PathBuf};

use fs2::FileExt;
use tracing::warn;

/// File name of the lease, created next to `identity.key`.
const LEASE_FILE: &str = "endpoint.lock";

/// Why [`EndpointLease::try_acquire`] refused to grant the endpoint.
///
/// Both answers keep the caller off the wire, so the *behaviour* is the same
/// either way — but they are not the same sentence to a user, and collapsing
/// them into one is how a device with an unwritable `~/.outl` gets told to go
/// find an `outl mcp serve` that isn't running. Every caller that words this
/// for a human reads the variant; a caller that only degrades can ignore it.
#[derive(Debug)]
pub enum LeaseDenied {
    /// Another outl process on this device already holds the lease. The
    /// ordinary outcome of the election, and a working state: the holder
    /// pushes this process's ops out on its own catch-up pass.
    HeldByAnotherProcess,
    /// The lease file itself could not be opened (permission denied,
    /// read-only mount, the parent is not a directory), so nothing on this
    /// device can arbitrate — see the fail-closed rule on
    /// [`EndpointLease::try_acquire`]. Nobody holds the endpoint here; the
    /// device directory is broken.
    LeaseFileUnusable {
        /// The lease file we could not open.
        path: PathBuf,
        /// What the open failed with.
        error: std::io::Error,
    },
}

impl fmt::Display for LeaseDenied {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HeldByAnotherProcess => {
                write!(f, "another outl process on this device holds the endpoint")
            }
            Self::LeaseFileUnusable { path, error } => write!(
                f,
                "the endpoint lease file {} cannot be opened ({error})",
                path.display()
            ),
        }
    }
}

/// Exclusive, advisory claim on this device's iroh endpoint.
///
/// Held for as long as the value lives; `flock(2)` is released by the kernel
/// when the process exits, so a crashed holder never leaves a stale lease
/// behind and there is no TTL to tune.
///
/// Obtained through [`EndpointLease::try_acquire`], and normally kept alive by
/// the [`crate::IrohSyncTransport`] it authorises (see
/// [`crate::build_transport`]) rather than by the caller.
#[must_use = "the lease is released when the value is dropped; keep it alive for as long as the endpoint runs"]
#[derive(Debug)]
pub struct EndpointLease {
    /// `None` when the lease file opened but the filesystem refused to lock it
    /// for a reason other than "taken" (see [`EndpointLease::try_acquire`]).
    /// The lease was still granted; there is simply no lock to release on drop.
    file: Option<File>,
    path: PathBuf,
}

impl EndpointLease {
    /// Try to claim this device's endpoint, keyed off `identity_path`.
    ///
    /// - `Ok(lease)` — the caller owns the endpoint and may bind it.
    /// - `Err(denied)` — the caller must **not** bind an endpoint; it should
    ///   fall back to [`outl_actions::FileSyncTransport`], which converges
    ///   through the shared `ops/` dir with no wire presence of its own.
    ///   [`LeaseDenied`] says which of the two refusals below happened, so a
    ///   caller reporting it to a user doesn't have to guess.
    ///
    /// The two failure modes are deliberately opposite, because they say
    /// different things about the arbiter:
    ///
    /// - **Cannot open the file** (permission denied, read-only mount, the
    ///   parent is not a directory) → **fail-closed**. There is no arbiter and
    ///   no way to tell whether someone else is already bound, so granting
    ///   would grant to *every* process that asks, and a device where the
    ///   directory is unwritable is broken in ways an endpoint won't fix.
    /// - **Opened, but `try_lock_exclusive` failed with something other than
    ///   `WouldBlock`** (`ENOLCK`, an exotic mount that has no locking) →
    ///   **fail-open**: the lease is granted, unarbitrated, with a warning.
    ///   Here the file exists and is ours to write; only the *locking* is
    ///   missing. Refusing everyone would leave the device with no endpoint at
    ///   all, which is the exact failure this module exists to remove, and the
    ///   contention it re-admits is only possible between processes on one
    ///   machine. This mirrors `outl_core::lock`'s rule that only `WouldBlock`
    ///   means "someone else has it".
    pub fn try_acquire(identity_path: &Path) -> Result<Self, LeaseDenied> {
        let path = lease_path(identity_path);
        if let Some(parent) = path.parent() {
            // A failure here needs no branch of its own: the open below fails
            // for the same reason and reports it.
            let _ = std::fs::create_dir_all(parent);
        }
        let file = match File::options()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&path)
        {
            Ok(f) => f,
            Err(error) => {
                warn!(
                    "endpoint lease: cannot open {} ({error}); staying off the wire",
                    path.display()
                );
                return Err(LeaseDenied::LeaseFileUnusable { path, error });
            }
        };
        match file.try_lock_exclusive() {
            Ok(()) => Ok(Self {
                file: Some(file),
                path,
            }),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                Err(LeaseDenied::HeldByAnotherProcess)
            }
            Err(e) => {
                warn!(
                    "endpoint lease: {} cannot be locked ({e}); proceeding unarbitrated",
                    path.display()
                );
                Ok(Self { file: None, path })
            }
        }
    }

    /// Path of the lease file (diagnostics).
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for EndpointLease {
    fn drop(&mut self) {
        if let Some(file) = &self.file {
            let _ = FileExt::unlock(file);
        }
    }
}

/// Lease file for a given identity key: a sibling named [`LEASE_FILE`].
fn lease_path(identity_path: &Path) -> PathBuf {
    match identity_path.parent() {
        Some(dir) => dir.join(LEASE_FILE),
        None => PathBuf::from(LEASE_FILE),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_second_process_on_a_device_is_told_the_endpoint_is_taken() {
        let dir = tempfile::tempdir().expect("tempdir");
        let identity = dir.path().join("identity.key");

        let first = EndpointLease::try_acquire(&identity).expect("first claim wins");
        assert!(
            matches!(
                EndpointLease::try_acquire(&identity),
                Err(LeaseDenied::HeldByAnotherProcess)
            ),
            "a second claim on the same identity must not be granted, and must \
             say a process holds it rather than leaving the caller to guess"
        );

        drop(first);
        assert!(
            EndpointLease::try_acquire(&identity).is_ok(),
            "releasing the lease must hand the endpoint to the next process"
        );
    }

    #[test]
    fn two_identities_on_one_machine_do_not_contend() {
        let dir = tempfile::tempdir().expect("tempdir");
        let desktop = dir.path().join("desktop/identity.key");
        let mobile = dir.path().join("mobile/identity.key");

        let _a = EndpointLease::try_acquire(&desktop).expect("desktop claim");
        assert!(
            EndpointLease::try_acquire(&mobile).is_ok(),
            "a different identity is a different node id, so it is a different endpoint"
        );
    }

    #[test]
    fn a_lease_file_that_cannot_be_opened_denies_the_endpoint_instead_of_granting_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        // The lease's parent is a regular FILE, so both `create_dir_all` and the
        // open fail. Stands in for the real cases — permission denied, a
        // read-only mount — that used to take the fail-open branch meant for a
        // filesystem that merely cannot *lock*. Without an arbiter there is no
        // way to know whether someone else is already bound, so granting here
        // grants to every process that asks.
        let blocked = dir.path().join("not-a-dir");
        std::fs::write(&blocked, b"").expect("write blocker");

        assert!(
            matches!(
                EndpointLease::try_acquire(&blocked.join("identity.key")),
                Err(LeaseDenied::LeaseFileUnusable { .. })
            ),
            "an unopenable lease file must keep the caller off the wire, not \
             wave every process through unarbitrated — and it must not be \
             reported as another process holding the endpoint, because there \
             is no such process to wait for"
        );
    }

    #[test]
    fn the_lease_lives_next_to_the_identity_it_gates() {
        let dir = tempfile::tempdir().expect("tempdir");
        let identity = dir.path().join("identity.key");
        let lease = EndpointLease::try_acquire(&identity).expect("claim");
        assert_eq!(lease.path(), dir.path().join(LEASE_FILE));
    }
}
