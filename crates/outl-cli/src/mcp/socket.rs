//! Unix-socket transport for the MCP server embedded in `outl serve`.
//!
//! The socket is the whole network surface: no TCP, nothing that listens on
//! an address. It lives in a per-user directory under the device store
//! (`<device dir>/run/`, mode 0700) and is itself 0600, so only the owning
//! user can connect. The device store is where `identity.key` and the actor
//! bindings already live, which also means a test that isolates its device
//! directory (`OUTL_DEVICE_DIR`, `XDG_CONFIG_HOME`) gets an isolated socket.
//!
//! One socket per workspace: the file name carries a hash of the canonical
//! workspace root, so two workspaces served on one machine do not collide and
//! `outl mcp serve` can find the right one from the path alone.

use std::path::{Path, PathBuf};

/// Longest socket path we bind. `sun_path` is 104 bytes on macOS and 108 on
/// Linux, and the bind fails with an unhelpful error past that.
const MAX_SOCKET_PATH: usize = 100;

/// FNV-1a. Hand-rolled because the name has to be identical in every `outl`
/// build that may talk to this one, and `DefaultHasher` promises nothing
/// across releases.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for b in bytes {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

/// Socket path for `workspace` under `device_dir`.
pub(crate) fn socket_path_in(device_dir: &Path, workspace: &Path) -> PathBuf {
    let root = std::fs::canonicalize(workspace).unwrap_or_else(|_| workspace.to_path_buf());
    let hash = fnv1a(root.as_os_str().as_encoded_bytes());
    device_dir.join("run").join(format!("mcp-{hash:016x}.sock"))
}

/// Socket path for `workspace` under this device's store.
pub(crate) fn socket_path(workspace: &Path) -> PathBuf {
    let store = outl_core::device::DeviceStore::open_default();
    socket_path_in(store.dir(), workspace)
}

#[cfg(unix)]
mod imp {
    use super::MAX_SOCKET_PATH;
    use std::io;
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::{Path, PathBuf};

    /// Outcome of trying to claim the socket path.
    pub(crate) enum Claim {
        /// We own the socket now.
        Bound(McpListener),
        /// A live `outl serve` already answers on it.
        AlreadyServed,
        /// Could not be set up here; the string says why.
        Unavailable(String),
    }

    /// A bound socket. Dropping it removes the file, so a clean shutdown
    /// leaves nothing for the next start to probe.
    pub(crate) struct McpListener {
        pub(crate) listener: UnixListener,
        path: PathBuf,
    }

    impl Drop for McpListener {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    /// Connect to a live server, if there is one.
    pub(crate) fn connect(path: &Path) -> io::Result<UnixStream> {
        UnixStream::connect(path)
    }

    fn private_dir(dir: &Path) -> io::Result<()> {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
        // An existing directory keeps whatever mode it had.
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
    }

    /// Claim `path`: refuse when a live server holds it, replace a stale file,
    /// bind otherwise. Never fails startup; the caller logs the verdict.
    pub(crate) fn claim(path: &Path) -> Claim {
        if path.as_os_str().len() > MAX_SOCKET_PATH {
            return Claim::Unavailable(format!(
                "socket path {} is longer than {MAX_SOCKET_PATH} bytes",
                path.display()
            ));
        }
        let Some(dir) = path.parent() else {
            return Claim::Unavailable("socket path has no parent directory".to_string());
        };
        if let Err(e) = private_dir(dir) {
            return Claim::Unavailable(format!("cannot prepare {}: {e}", dir.display()));
        }
        if path.exists() {
            match UnixStream::connect(path) {
                Ok(_) => return Claim::AlreadyServed,
                // Nobody is listening: a previous server died without
                // cleaning up.
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::ConnectionRefused | io::ErrorKind::NotFound
                    ) =>
                {
                    if let Err(e) = std::fs::remove_file(path) {
                        return Claim::Unavailable(format!(
                            "cannot remove stale socket {}: {e}",
                            path.display()
                        ));
                    }
                }
                Err(e) => {
                    return Claim::Unavailable(format!(
                        "cannot probe existing socket {}: {e}",
                        path.display()
                    ))
                }
            }
        }
        match UnixListener::bind(path) {
            Ok(listener) => {
                let private = std::fs::Permissions::from_mode(0o600);
                if let Err(e) = std::fs::set_permissions(path, private) {
                    let _ = std::fs::remove_file(path);
                    return Claim::Unavailable(format!("cannot restrict {}: {e}", path.display()));
                }
                Claim::Bound(McpListener {
                    listener,
                    path: path.to_path_buf(),
                })
            }
            // Lost a race with another `outl serve` between the probe and the bind.
            Err(e) if e.kind() == io::ErrorKind::AddrInUse => Claim::AlreadyServed,
            Err(e) => Claim::Unavailable(format!("cannot bind {}: {e}", path.display())),
        }
    }
}

#[cfg(unix)]
pub(crate) use imp::{claim, connect, Claim, McpListener};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_path_is_stable_short_and_per_workspace() {
        let dev = Path::new("/tmp/dev");
        let a = socket_path_in(dev, Path::new("/nonexistent/a"));
        let b = socket_path_in(dev, Path::new("/nonexistent/b"));
        assert_eq!(a, socket_path_in(dev, Path::new("/nonexistent/a")));
        assert_ne!(a, b);
        assert!(a.starts_with("/tmp/dev/run"));
        assert!(a.as_os_str().len() < MAX_SOCKET_PATH);
    }
}
