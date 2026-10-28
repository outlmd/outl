#!/usr/bin/env python3
"""Pins for `scripts/pin_workspace_deps.py`.

The release job rewrites `version = "0.13.0"` to the beta string and then
calls this script. A caret left on that beta matches the stable release,
which is what made `cargo publish` verify `outl-actions` against
`outl-md` 0.13.0. These cases fail if the pin misses a path dep, touches
the package version, or the publish job stops calling it.

Run: python3 scripts/tests/test_pin_workspace_deps.py
"""

import importlib.util
import os
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, "..", ".."))
SCRIPT = os.path.join(ROOT, "scripts", "pin_workspace_deps.py")

spec = importlib.util.spec_from_file_location("pin_workspace_deps", SCRIPT)
pin = importlib.util.module_from_spec(spec)
spec.loader.exec_module(pin)

VERSION = "0.13.0-beta.999"


FIXTURE = """\
[workspace.package]
version = "0.13.0-beta.999"
edition = "2021"

[workspace.dependencies]
outl-core = { path = "crates/outl-core", version = "0.13.0-beta.999" }
outl-exec = { path = "crates/outl-exec", version = "0.13.0-beta.999", default-features = false }
serde = { version = "1", features = ["derive"] }
"""


class PinWorkspaceDeps(unittest.TestCase):
    def test_pins_path_deps_and_leaves_the_package_version(self):
        updated = pin.pin_manifest(FIXTURE, VERSION)
        self.assertIn('version = "0.13.0-beta.999"\n', updated)
        self.assertIn(
            'outl-core = { path = "crates/outl-core", version = "=0.13.0-beta.999" }',
            updated,
        )
        self.assertIn(
            'version = "=0.13.0-beta.999", default-features = false',
            updated,
        )
        self.assertIn('serde = { version = "1", features = ["derive"] }', updated)
        # The package version is the bare string. Path deps are exact.
        for line in updated.splitlines():
            if 'path = "crates/' in line:
                self.assertIn('version = "=0.13.0-beta.999"', line)
                self.assertNotIn('version = "0.13.0-beta.999"', line)

    def test_second_run_is_a_no_op(self):
        once = pin.pin_manifest(FIXTURE, VERSION)
        twice = pin.pin_manifest(once, VERSION)
        self.assertEqual(once, twice)

    def test_a_path_dep_at_another_version_is_an_error(self):
        text = 'outl-md = { path = "crates/outl-md", version = "0.12.0" }\n'
        with self.assertRaises(pin.PinError):
            pin.pin_manifest(text, VERSION)

    def test_a_manifest_with_no_internal_dep_is_an_error(self):
        text = '[workspace.package]\nversion = "0.13.0-beta.999"\n'
        with self.assertRaises(pin.PinError):
            pin.pin_manifest(text, VERSION)

    def test_real_workspace_manifest(self):
        with open(os.path.join(ROOT, "Cargo.toml"), encoding="utf-8") as fh:
            original = fh.read()
        # Same replacement the publish job's sed does, against today's base.
        rewritten = original.replace(
            'version = "0.13.0"', 'version = "0.13.0-beta.999"'
        )
        self.assertNotEqual(rewritten, original)
        updated = pin.pin_manifest(rewritten, VERSION)
        package_version = [
            line
            for line in updated.splitlines()
            if line.startswith("version = ")
        ]
        self.assertEqual(package_version, ['version = "0.13.0-beta.999"'])
        internal = [
            line for line in updated.splitlines() if 'path = "crates/' in line
        ]
        self.assertGreaterEqual(len(internal), 5)
        for line in internal:
            self.assertIn('version = "=0.13.0-beta.999"', line)
            self.assertIn("default-features = false", updated)
        self.assertEqual(pin.pin_manifest(updated, VERSION), updated)

    def test_publish_job_pins_before_it_publishes(self):
        with open(
            os.path.join(ROOT, ".github", "workflows", "release.yml"),
            encoding="utf-8",
        ) as fh:
            workflow = fh.read()
        pin_at = workflow.index("scripts/pin_workspace_deps.py")
        loop_at = workflow.index(
            "for crate in outl-core outl-md outl-exec outl-actions outl-ws; do"
        )
        self.assertLess(pin_at, loop_at)
        # The version rewrite still happens first. The pin only matches
        # the string that sed just wrote.
        sed_at = workflow.rindex('s|version = \\"${BASE}\\"|version = \\"${VERSION}\\"|g', 0, pin_at)
        self.assertLess(sed_at, pin_at)


if __name__ == "__main__":
    unittest.main()
