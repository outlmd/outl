#!/usr/bin/env python3
"""Hold the release pipeline against `scripts/release_assets.py`.

Two modes, because the list of assets answers two different questions:

  `--manifest`   Offline, runs in `hygiene.yml`. Do the build matrices, the
                 `update_tap` loops and the four package-manager manifests all
                 name the platforms the table names? Every disagreement
                 between them is silent at release time (see that module's
                 docstring), so it fails here instead.

  `--draft FILE` Runs in `release.yml`, on `gh release view --json assets`.
                 Is every *required* asset in the draft? A missing required
                 asset fails, so a release never publishes without the
                 platforms it claims. A missing optional one prints a note for
                 the release body and exits 0 — the gap reaches the user
                 instead of either blocking every other platform or
                 disappearing.

  `--draft FILE --tap NAME`
                 May this manifest advance? Only when every asset it points at
                 is present. A manifest with a new version and the previous
                 release's digest is a hash mismatch on install, which is
                 worse than a manifest that still points at an older release.

Usage:
  scripts/check-release-assets.py --manifest [--root .]
  scripts/check-release-assets.py --draft assets.json --version 1.2.3-beta.4
                                  [--tap brew|cask|scoop] [--note-file out.md]
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import release_assets as table  # noqa: E402

WORKFLOW = Path(".github/workflows/release.yml")
FORMULA = Path("Formula/outl-beta.rb")
CASK = Path("Casks/outl-desktop-beta.rb")
SCOOP = Path("bucket/outl-beta.json")

# Which matrix key identifies a leg, per job. `build` legs are one Rust
# target; the desktop legs are one architecture, and build for the host.
MATRIX_KEY = {
    "build": "target",
    "build_desktop_linux": "arch",
    "build_desktop_windows": "arch",
}

# `anchor -> Scoop architecture key`. Scoop's own spelling, which is not ours.
SCOOP_ARCH = {"windows-x64": "64bit", "windows-arm64": "arm64"}


def job_block(text: str, name: str) -> str:
    """The lines of one top-level job. Jobs sit at two-space indentation, so
    the block ends at the next line that starts a sibling key."""
    start = re.search(rf"^  {re.escape(name)}:$", text, re.M)
    if not start:
        sys.exit(f"FAIL: {WORKFLOW} has no `{name}:` job — this script is reading a file it does not recognise")
    rest = text[start.end() :]
    end = re.search(r"^  \S", rest, re.M)
    return rest[: end.start()] if end else rest


def matrix_legs(block: str, key: str) -> dict[str, bool]:
    """`matrix key -> is it marked optional` for every leg in one job.

    The matrix is read up to `steps:` first: a leg's own lines are indented
    further than the list marker, so splitting the whole job block on the
    marker would hand the last leg every step in the job.
    """
    matrix = block.split("\n    steps:")[0]
    legs: dict[str, bool] = {}
    for leg in re.split(r"^          - ", matrix, flags=re.M)[1:]:
        found = re.search(rf"^ *{key}: (\S+)", leg, re.M)
        if found:
            legs[found.group(1)] = bool(re.search(r"^ *optional: true", leg, re.M))
    return legs


def uploaded_assets(block: str, archs: set[str]) -> set[str]:
    """Asset names a job hands to `gh release upload`.

    Read from the upload command itself rather than by grepping the job for
    anything that looks like a filename: the bundlers' own intermediate names
    (`outl_<version>_universal.dmg`) look exactly like asset names and are
    not. The command spans lines, continued with a trailing backslash.

    The names are templated on the architecture, so each `${ARCH}` /
    `$env:ARCH` is expanded with every arch this job's matrix builds — a
    superset when a job has more than one leg, which is the safe direction
    here: this answers "is every name the workflow uploads one the table
    knows", and the per-leg direction is checked separately.
    """
    found: set[str] = set()
    for match in re.finditer(r"gh release upload(.*?)(?:\n\s*(?:--repo|&&|\n)|\Z)", block, re.S):
        command = match.group(1)
        # Stop at the first line that does not continue the command.
        lines = []
        for line in command.splitlines():
            lines.append(line)
            if not line.rstrip().endswith("\\"):
                break
        for token in re.findall(r"[\w$.{}:/-]*outl[\w$.{}:-]*", "\n".join(lines)):
            token = token.strip("\"'")
            if token.endswith(".sha256"):
                continue
            if "ARCH" in token:
                for arch in archs:
                    found.add(token.replace("${ARCH}", arch).replace("$env:ARCH", arch))
            else:
                found.add(token)
    return found


def check_manifest(root: Path) -> list[str]:
    workflow = (root / WORKFLOW).read_text()
    formula = (root / FORMULA).read_text()
    cask = (root / CASK).read_text()
    scoop = json.loads((root / SCOOP).read_text())
    problems: list[str] = []

    # 1. The matrices carry exactly the legs the table describes, and the
    #    optional ones are the ones marked optional. A leg whose
    #    `continue-on-error` is missing can still hold back the release, which
    #    is the thing this whole shape exists to prevent.
    for job, key in MATRIX_KEY.items():
        block = job_block(workflow, job)
        if "continue-on-error: ${{ matrix.optional" not in block:
            problems.append(
                f"`{job}` has no `continue-on-error: ${{{{ matrix.optional || false }}}}` — "
                f"an optional leg there would still fail the job and hold back every other platform"
            )
        in_workflow = matrix_legs(block, key)
        in_table = {leg.key: not leg.required for leg in table.LEGS if leg.job == job}
        for extra in sorted(set(in_workflow) - set(in_table)):
            problems.append(f"`{job}` builds `{extra}`, which `release_assets.py` does not list — its assets are gated by nothing")
        for missing in sorted(set(in_table) - set(in_workflow)):
            problems.append(f"`release_assets.py` lists `{job}` leg `{missing}`, which that matrix does not build")
        for shared in sorted(set(in_workflow) & set(in_table)):
            if in_workflow[shared] != in_table[shared]:
                expected = "optional: true" if in_table[shared] else "no `optional:`"
                problems.append(
                    f"`{job}` leg `{shared}` disagrees with `release_assets.py` about being required: "
                    f"the table says {expected}"
                )

    # 2. Every asset the table names is a string the job that uploads it can
    #    actually produce. Without this, renaming an asset in the workflow and
    #    forgetting the table is caught by the publish gate instead — at
    #    release time, as a missing *required* asset, which blocks the release
    #    over a bookkeeping slip. Here it is a one-second failure on the PR.
    for leg in table.LEGS:
        block = job_block(workflow, leg.job)
        for asset in leg.assets:
            stem = re.sub(r"\.(tar\.gz|zip)$", "", asset)
            # The shells build these names from the matrix, so the arch is a
            # variable: bash `${ARCH}`, PowerShell `$env:ARCH`. The CLI job
            # names the stem once, as `artifact:`.
            variants = {
                asset,
                stem,
                asset.replace(leg.key, "${ARCH}"),
                asset.replace(leg.key, "$env:ARCH"),
            }
            if not any(variant in block for variant in variants):
                problems.append(
                    f"`{leg.job}` never mentions `{asset}` (nor its `${{ARCH}}` form), "
                    f"but `release_assets.py` says that job uploads it"
                )

    # 3. And the other direction: every name a job uploads is one the table
    #    knows about. Without this, an asset added to a job and not to the
    #    table is gated by nothing — the release publishes without it and
    #    says nothing, which is the failure this whole table exists to end.
    for job in {leg.job for leg in table.LEGS}:
        block = job_block(workflow, job)
        archs = {leg.key for leg in table.LEGS if leg.job == job}
        known = {asset for leg in table.LEGS if leg.job == job for asset in leg.assets}
        for uploaded in sorted(uploaded_assets(block, archs) - known):
            problems.append(
                f"`{job}` uploads `{uploaded}`, which `release_assets.py` does not list — "
                f"nothing gates it, so a release missing it would publish in silence"
            )

    # 4. `update_tap` reads one `.sha256` per name in two loops, and `awk` on a
    #    file the release never uploaded fails the bump.
    tap = job_block(workflow, "update_tap")
    url_loop = re.search(r"for arch in ([^;\n]+); do", tap)
    bottle_loop = re.search(r"for pair in ([^;\n]+); do", tap)
    if not url_loop or not bottle_loop:
        problems.append(f"{WORKFLOW}: `update_tap` no longer has the two `for` loops this script checks")
    else:
        looped = set(url_loop.group(1).split())
        paired = set(re.findall(r'"([^"]+)"', bottle_loop.group(1)))
        brewed = {leg.anchor: leg for leg in table.LEGS if "brew" in leg.taps}
        for anchor, leg in sorted(brewed.items()):
            if anchor not in looped:
                problems.append(f"`update_tap`'s `for arch in …` does not list `{anchor}`, so its download digest is never bumped")
            if leg.bottle_tag and f"{anchor}:{leg.bottle_tag}" not in paired:
                problems.append(f'`update_tap`\'s `for pair in …` does not list `"{anchor}:{leg.bottle_tag}"`')
        for orphan in sorted(looped - set(brewed)):
            problems.append(f"`update_tap` bumps `{orphan}`, which no leg in `release_assets.py` uploads")

    # 5. Every manifest has a line per platform it serves, with the anchor the
    #    `sed` matches on (Scoop is JSON and is rewritten with `jq`, so it is
    #    checked by structure instead).
    for leg in table.LEGS:
        if "brew" in leg.taps:
            if f"# anchor: {leg.anchor}" not in formula:
                problems.append(f"{FORMULA} has no `# anchor: {leg.anchor}` line for {leg.assets[0]}")
            line = re.search(rf"^.*# anchor: bottle-{re.escape(leg.anchor or '')}$", formula, re.M)
            if not line:
                problems.append(f"{FORMULA} has no `# anchor: bottle-{leg.anchor}` line")
            elif f"{leg.bottle_tag}:" not in line.group(0):
                problems.append(
                    f"{FORMULA}'s `# anchor: bottle-{leg.anchor}` line does not carry the `{leg.bottle_tag}:` tag "
                    f"the matrix builds — `update_tap`'s `sed` matches on both"
                )
        if "cask" in leg.taps and f"# anchor: {leg.anchor}" not in cask:
            problems.append(f"{CASK} has no `# anchor: {leg.anchor}` line for {leg.assets[0]}")
        if "scoop" in leg.taps:
            arch = SCOOP_ARCH.get(leg.anchor or "")
            spec = scoop.get("architecture", {}).get(arch or "")
            if not spec:
                problems.append(f"{SCOOP} has no `architecture.{arch}` block for {leg.assets[0]}")
            elif leg.assets[0] not in spec.get("url", ""):
                problems.append(f"{SCOOP}'s `architecture.{arch}.url` does not point at {leg.assets[0]}")

    for anchor in sorted(set(re.findall(r"# anchor: (?!bottle-root-url)(?:bottle-)?(\S+)", formula))):
        if anchor not in {leg.anchor for leg in table.LEGS if "brew" in leg.taps}:
            problems.append(f"{FORMULA} carries `# anchor: {anchor}`, which no leg in `release_assets.py` produces")
    for arch in sorted(scoop.get("architecture", {})):
        if arch not in {SCOOP_ARCH.get(leg.anchor or "") for leg in table.LEGS if "scoop" in leg.taps}:
            problems.append(f"{SCOOP} serves `architecture.{arch}`, which no leg in `release_assets.py` produces")

    return problems


def check_draft(assets: set[str], version: str, tap: str | None) -> tuple[int, str]:
    """Returns (exit code, note). The note is markdown for the release body."""
    if tap:
        wanted = table.tap_assets(version, tap)
        missing = sorted(name for name in wanted if name not in assets)
        if missing:
            print(
                f"::warning::the {tap} manifest is not being bumped: {len(missing)} asset(s) it points at "
                f"are not on this release ({', '.join(missing)}). It keeps pointing at the previous release, "
                f"which is consistent; a bumped version with a stale digest would not be.",
            )
            return 1, ""
        print(f"PASS: every asset the {tap} manifest points at is on the release")
        return 0, ""

    wanted = table.expected(version)
    missing_required = sorted(n for n, leg in wanted.items() if leg.required and n not in assets)
    missing_optional = sorted(n for n, leg in wanted.items() if not leg.required and n not in assets)

    for name in missing_required:
        print(f"::error::required release asset missing: {name}", file=sys.stderr)
    if missing_required:
        print(
            f"FAIL: {len(missing_required)} required asset(s) missing — refusing to publish a release "
            f"that claims platforms it does not carry",
            file=sys.stderr,
        )
        return 1, ""

    if not missing_optional:
        print(f"PASS: all {len(wanted)} expected assets are on the release")
        return 0, ""

    # One line per leg, not per file: six missing files from one bundler leg is
    # one fact about one architecture.
    legs = {}
    for name in missing_optional:
        leg = wanted[name]
        legs.setdefault((leg.job, leg.key), []).append(name)
    lines = ["", "---", "", "**This build is missing some optional artifacts.**", ""]
    for (job, key), names in sorted(legs.items()):
        lines.append(f"- `{job}` / `{key}`: {len(names)} asset(s) did not land, including `{names[0]}`.")
    lines.append("")
    lines.append(
        "Those platforms are still in bring-up, so their build does not hold back the rest of the "
        "release. The run log says why they failed."
    )
    # Trailing newline: the caller appends this to the release body with
    # `cat`, and a body that ends mid-line renders the next thing glued to it.
    note = "\n".join(lines) + "\n"
    print(f"::warning::publishing without {len(missing_optional)} optional asset(s) from {len(legs)} leg(s); the release notes say so")
    print(note)
    return 0, note


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--manifest", action="store_true")
    ap.add_argument("--draft", type=Path)
    ap.add_argument("--version")
    ap.add_argument("--tap", choices=("brew", "cask", "scoop"))
    ap.add_argument("--note-file", type=Path)
    ap.add_argument("--root", type=Path, default=Path("."))
    args = ap.parse_args()

    if args.manifest:
        problems = check_manifest(args.root)
        if problems:
            for problem in problems:
                print(f"FAIL: {problem}", file=sys.stderr)
            return 1
        required = sum(1 for leg in table.LEGS if leg.required)
        print(
            f"PASS: {len(table.LEGS)} build leg(s) ({required} required) agree across the matrices, "
            f"the tap bump and the four manifests"
        )
        return 0

    if not args.draft or not args.version:
        ap.error("--draft needs --version")

    payload = json.loads(args.draft.read_text())
    # `gh release view --json assets` shape, or a bare list of names.
    if isinstance(payload, dict):
        payload = payload.get("assets", [])
    names = {a["name"] if isinstance(a, dict) else a for a in payload}
    code, note = check_draft(names, args.version, args.tap)
    if args.note_file:
        args.note_file.write_text(note)
    return code


if __name__ == "__main__":
    sys.exit(main())
