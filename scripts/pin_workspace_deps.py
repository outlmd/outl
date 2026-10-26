#!/usr/bin/env python3
"""Pin workspace-internal path dependencies to an exact version.

`cargo publish` turns

    outl-md = { path = "crates/outl-md", version = "0.13.0-beta.N" }

into a caret requirement. Cargo's caret on a prerelease also matches the
stable release of that number, and the stable sorts higher, so the verify
build compiles against the crates.io release instead of the crate this job
just published.

https://doc.rust-lang.org/cargo/reference/specifying-dependencies.html
calls out the fully specified `"=1.2.3-alpha.3"` form for this case.

The package's own `version = "..."` line is not a path dependency and is
left alone. `=0.13.0` is not a valid package version.

Usage: python3 scripts/pin_workspace_deps.py <version> [Cargo.toml]
"""

from __future__ import annotations

import sys
from pathlib import Path


class PinError(Exception):
    """The manifest cannot be pinned without guessing."""


def pin_manifest(text: str, version: str) -> str:
    """Return `text` with every `path = "crates/..."` dep required at `=version`.

    A path dep whose version is already `=version` is left as it is, so a
    second run changes nothing. Any other version on such a line is an
    error: the release sed was supposed to have rewritten it first, and
    silently leaving a caret requirement is how the verify build picks the
    stable crate.
    """
    if not version or any(c in version for c in '"\n\r'):
        raise PinError(f"refusing to pin with version {version!r}")

    bare = f'version = "{version}"'
    exact = f'version = "={version}"'
    pinned = 0
    already = 0
    out: list[str] = []

    for line in text.splitlines(True):
        if 'path = "crates/' not in line:
            out.append(line)
            continue
        if exact in line:
            already += 1
            out.append(line)
            continue
        if bare not in line:
            raise PinError(
                "workspace-internal dep is not at "
                f"{version!r} and not already exact: {line.rstrip()}"
            )
        pinned += 1
        out.append(line.replace(bare, exact, 1))

    if pinned + already == 0:
        raise PinError('no workspace-internal path dependencies (path = "crates/...)')

    return "".join(out)


def main(argv: list[str]) -> int:
    if len(argv) not in (2, 3):
        print(
            "usage: pin_workspace_deps.py <version> [Cargo.toml]",
            file=sys.stderr,
        )
        return 2
    version = argv[1]
    manifest = Path(argv[2] if len(argv) == 3 else "Cargo.toml")
    try:
        original = manifest.read_text()
        updated = pin_manifest(original, version)
    except PinError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1
    except OSError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1
    if updated != original:
        manifest.write_text(updated)
    internal = sum(1 for line in updated.splitlines() if 'path = "crates/' in line)
    print(f"pinned {internal} workspace-internal deps to ={version}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
