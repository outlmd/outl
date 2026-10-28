#!/usr/bin/env python3
"""Tests for `scripts/check-release-assets.py`.

Two things are being pinned, and they fail in opposite directions.

The `--manifest` mode decides whether four files agree about the platforms a
release ships. Every way they can disagree is silent at release time, so a
check that reports "PASS" because its parser stopped understanding the
workflow is worse than no check: each case below mutates one file the way a
careless change would and asserts the specific complaint.

The `--draft` mode decides whether a release publishes. A false failure
blocks a good release; a false pass publishes one that claims platforms it
does not carry, or bumps a package manager onto a digest that is not there.

Run: python3 scripts/tests/test_check_release_assets.py
"""

import contextlib
import importlib.util
import io
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, "..", ".."))
SCRIPT = os.path.join(ROOT, "scripts", "check-release-assets.py")

spec = importlib.util.spec_from_file_location("check_release_assets", SCRIPT)
check = importlib.util.module_from_spec(spec)
spec.loader.exec_module(check)
table = check.table

VERSION = "9.9.9-beta.1"

TRACKED = (
    ".github/workflows/release.yml",
    "Formula/outl-beta.rb",
    "Casks/outl-desktop-beta.rb",
    "bucket/outl-beta.json",
)


class ManifestMode(unittest.TestCase):
    """`--manifest`: the four files against `release_assets.py`."""

    def setUp(self):
        self.root = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, self.root)
        for rel in TRACKED:
            dst = os.path.join(self.root, rel)
            os.makedirs(os.path.dirname(dst), exist_ok=True)
            shutil.copy(os.path.join(ROOT, rel), dst)

    def path(self, rel):
        return os.path.join(self.root, rel)

    def edit(self, rel, old, new, count=1):
        """Replace `old` in a fixture copy. `count=None` means "every
        occurrence, and there is at least one" — for a name that appears
        across a job's steps, where pinning the number would make the test
        fail on an unrelated edit to the workflow."""
        with open(self.path(rel)) as handle:
            text = handle.read()
        if count is None:
            self.assertIn(old, text, f"{rel}: fixture no longer contains {old!r}")
        else:
            self.assertEqual(text.count(old), count, f"{rel}: fixture no longer contains {old!r}")
        with open(self.path(rel), "w") as handle:
            handle.write(text.replace(old, new))

    def problems(self):
        return check.check_manifest(check.Path(self.root))

    def assertComplains(self, needle):
        found = self.problems()
        self.assertTrue(found, "expected a complaint, got a clean pass")
        self.assertTrue(
            any(needle in problem for problem in found),
            f"no complaint mentioned {needle!r}; got: {found}",
        )

    def test_the_repo_as_it_stands_passes(self):
        self.assertEqual(self.problems(), [])

    def test_a_matrix_leg_missing_from_the_table(self):
        self.edit(
            ".github/workflows/release.yml",
            "          - os: blacksmith-4vcpu-windows-2025\n            target: x86_64-pc-windows-msvc",
            "          - os: blacksmith-4vcpu-windows-2025\n            target: x86_64-pc-windows-gnu",
        )
        self.assertComplains("x86_64-pc-windows-gnu")

    def test_a_table_leg_missing_from_the_matrix(self):
        self.edit(
            ".github/workflows/release.yml",
            "            target: aarch64-unknown-linux-gnu\n            artifact: outl-linux-arm64",
            "            target: riscv64gc-unknown-linux-gnu\n            artifact: outl-linux-arm64",
        )
        self.assertComplains("aarch64-unknown-linux-gnu")

    def test_an_optional_leg_that_the_workflow_treats_as_required(self):
        self.edit(
            ".github/workflows/release.yml",
            "            apprun_sha256: 072f17c0895a85c490282fe5395c5007e5fc75da727e553b3b8fb680feb11578\n            optional: true",
            "            apprun_sha256: 072f17c0895a85c490282fe5395c5007e5fc75da727e553b3b8fb680feb11578",
        )
        self.assertComplains("disagrees with `release_assets.py` about being required")

    def test_continue_on_error_removed_from_a_job(self):
        """Without it, `optional` in the table means nothing: the leg still
        fails its job, and the job still holds back every other platform."""
        self.edit(
            ".github/workflows/release.yml",
            "    continue-on-error: ${{ matrix.optional || false }}\n    strategy:\n      fail-fast: false\n      matrix:\n        include:\n          # Both legs are pinned to 22.04 on purpose:",
            "    strategy:\n      fail-fast: false\n      matrix:\n        include:\n          # Both legs are pinned to 22.04 on purpose:",
        )
        self.assertComplains("continue-on-error")

    def test_an_asset_renamed_in_the_workflow(self):
        """Caught here rather than at release time, where it would surface as
        a missing *required* asset and block the publish over a slip."""
        self.edit(
            ".github/workflows/release.yml",
            "outl-desktop-macos.dmg",
            "outl-desktop-mac.dmg",
            count=None,
        )
        self.assertComplains("never mentions `outl-desktop-macos.dmg`")

    def test_an_asset_whose_arch_placeholder_changed(self):
        """The desktop jobs build the name from the matrix, so the check knows
        the `${ARCH}` and `$env:ARCH` spellings. Any third spelling is a name
        the table can no longer confirm."""
        self.edit(
            ".github/workflows/release.yml",
            "outl-desktop-linux-${ARCH}",
            "outl-desktop-linux-${A}",
            count=None,
        )
        self.assertComplains("never mentions `outl-desktop-linux-x86_64.AppImage`")

    def test_an_asset_uploaded_but_not_in_the_table(self):
        """The other direction: nothing gates an asset the table does not
        know, so a release missing it would publish in silence."""
        self.edit(
            ".github/workflows/release.yml",
            '            "outl-desktop-linux-${ARCH}.rpm" \\\n',
            '            "outl-desktop-linux-${ARCH}.rpm" \\\n            "outl-desktop-linux-${ARCH}.tar.zst" \\\n',
        )
        self.assertComplains("uploads `outl-desktop-linux-x86_64.tar.zst`")

    def test_the_upload_scan_ignores_bundler_intermediates(self):
        """`outl_<version>_universal.dmg` is Tauri's own output name and sits
        in the same job as the upload. Reading filenames out of the whole job
        instead of out of the upload command would flag it."""
        self.assertNotIn(
            "universal.dmg",
            " ".join(self.problems()),
        )

    def test_a_platform_dropped_from_the_tap_bump_loop(self):
        self.edit(
            ".github/workflows/release.yml",
            "for arch in macos-arm64 macos-x64 linux-x64 linux-arm64; do",
            "for arch in macos-arm64 macos-x64 linux-x64; do",
        )
        self.assertComplains("does not list `linux-arm64`")

    def test_a_platform_dropped_from_the_bottle_loop(self):
        self.edit(
            ".github/workflows/release.yml",
            ' "linux-arm64:arm64_linux"',
            "",
        )
        self.assertComplains('"linux-arm64:arm64_linux"')

    def test_a_formula_download_anchor_deleted(self):
        self.edit("Formula/outl-beta.rb", "# anchor: linux-arm64", "# anchor: linux-armv7")
        self.assertComplains("has no `# anchor: linux-arm64` line")

    def test_a_formula_bottle_tag_renamed_under_its_anchor(self):
        self.edit("Formula/outl-beta.rb", "arm64_linux:   ", "aarch64_linux:")
        self.assertComplains("does not carry the `arm64_linux:` tag")

    def test_a_cask_anchor_deleted(self):
        self.edit("Casks/outl-desktop-beta.rb", "# anchor: macos", "# anchor: darwin")
        self.assertComplains("has no `# anchor: macos` line")

    def scoop(self, mutate):
        path = self.path("bucket/outl-beta.json")
        with open(path) as handle:
            payload = json.load(handle)
        mutate(payload)
        with open(path, "w") as handle:
            json.dump(payload, handle)

    def test_a_scoop_architecture_dropped(self):
        self.scoop(lambda p: p["architecture"].pop("arm64"))
        self.assertComplains("has no `architecture.arm64` block")

    def test_a_scoop_url_pointing_at_the_wrong_asset(self):
        self.scoop(
            lambda p: p["architecture"]["arm64"].update(url=p["architecture"]["64bit"]["url"])
        )
        self.assertComplains("does not point at outl-windows-arm64.zip")

    def test_a_scoop_architecture_nothing_builds(self):
        self.scoop(
            lambda p: p["architecture"].update(
                {"32bit": {"url": "https://example.invalid/x.zip", "hash": "0" * 64}}
            )
        )
        self.assertComplains("serves `architecture.32bit`")


class DraftMode(unittest.TestCase):
    """`--draft`: the assets on the draft against what a release must carry."""

    def all_assets(self):
        return set(table.expected(VERSION))

    @staticmethod
    def draft(assets, tap=None):
        """`check_draft`, with its workflow annotations swallowed: they are
        the point in CI and noise in a test run."""
        with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            return check.check_draft(assets, VERSION, tap)

    def test_a_complete_release_passes(self):
        code, note = self.draft(self.all_assets())
        self.assertEqual(code, 0)
        self.assertEqual(note, "")

    def test_a_missing_required_asset_fails(self):
        assets = self.all_assets() - {"outl-linux-x64.tar.gz"}
        code, _ = self.draft(assets)
        self.assertEqual(code, 1, "a release missing a required asset must not publish")

    def test_a_missing_sidecar_fails_too(self):
        """The digest file is what the package managers read, so it is an
        asset in its own right and not a detail of one."""
        assets = self.all_assets() - {"outl-macos-arm64.tar.gz.sha256"}
        code, _ = self.draft(assets)
        self.assertEqual(code, 1)

    def test_a_missing_optional_asset_publishes_with_a_note(self):
        gone = {name for name in self.all_assets() if "linux-aarch64" in name}
        self.assertTrue(gone, "fixture: no optional Linux aarch64 assets in the table")
        code, note = self.draft(self.all_assets() - gone)
        self.assertEqual(code, 0, "an optional gap must not hold back the rest of the release")
        self.assertIn("missing some optional artifacts", note)
        self.assertIn("build_desktop_linux", note)

    def test_the_note_names_one_line_per_leg_not_per_file(self):
        gone = {name for name in self.all_assets() if "linux-aarch64" in name}
        _, note = self.draft(self.all_assets() - gone)
        self.assertEqual(
            sum(1 for line in note.splitlines() if line.startswith("- ")),
            1,
            f"six files from one leg is one fact about one architecture: {note}",
        )

    def test_an_empty_draft_fails(self):
        code, _ = self.draft(set())
        self.assertEqual(code, 1, "the v0.12.0 failure mode: a draft with zero assets")

    def test_a_tap_advances_only_when_all_of_its_assets_are_there(self):
        for tap in ("brew", "cask", "scoop"):
            with self.subTest(tap=tap):
                wanted = table.tap_assets(VERSION, tap)
                self.assertTrue(wanted, f"{tap} points at nothing")
                code, _ = self.draft(self.all_assets(), tap)
                self.assertEqual(code, 0)
                one_gone = self.all_assets() - {sorted(wanted)[0]}
                code, _ = self.draft(one_gone, tap)
                self.assertEqual(
                    code, 1, f"{tap} must not be bumped onto a digest that is not on the release"
                )

    def test_an_optional_gap_still_blocks_the_tap_that_points_at_it(self):
        """`scoop` serves Windows arm64, which is optional. The release
        publishes without it; the manifest does not move. A bumped version
        with a stale digest is a hash mismatch on install."""
        assets = self.all_assets() - {"outl-windows-arm64.zip", "outl-windows-arm64.zip.sha256"}
        code, _ = self.draft(assets)
        self.assertEqual(code, 0)
        code, _ = self.draft(assets, "scoop")
        self.assertEqual(code, 1)

    def test_an_unrelated_gap_does_not_block_a_tap(self):
        assets = self.all_assets() - {"outl-android-arm64.apk", "outl-android-arm64.apk.sha256"}
        for tap in ("brew", "cask", "scoop"):
            code, _ = self.draft(assets, tap)
            self.assertEqual(code, 0, f"{tap} does not point at the APK")


class Cli(unittest.TestCase):
    def test_manifest_mode_exits_zero_on_the_repo(self):
        proc = subprocess.run(
            [sys.executable, SCRIPT, "--manifest"], cwd=ROOT, capture_output=True, text=True
        )
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertIn("PASS", proc.stdout)

    def test_draft_mode_reads_the_gh_json_shape(self):
        """`gh release view --json assets` nests the names; the script also
        takes a bare list, which is what `update_tap` hands it."""
        with tempfile.TemporaryDirectory() as tmp:
            payload = os.path.join(tmp, "assets.json")
            with open(payload, "w") as handle:
                json.dump({"assets": [{"name": n} for n in table.expected(VERSION)]}, handle)
            proc = subprocess.run(
                [sys.executable, SCRIPT, "--draft", payload, "--version", VERSION],
                cwd=ROOT,
                capture_output=True,
                text=True,
            )
            self.assertEqual(proc.returncode, 0, proc.stderr)


if __name__ == "__main__":
    unittest.main(verbosity=2)
