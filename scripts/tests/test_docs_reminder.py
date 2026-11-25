#!/usr/bin/env python3
"""Tests for `scripts/docs_reminder.py`.

The reminder runs on every PR, so the two ways it can go wrong are the two
pinned here: nagging on a change no user can see (people learn to ignore it),
and staying quiet on one they can (the whole point). Every `RULES` page also
has to exist, or the comment sends the author to a 404.

Run: python3 scripts/tests/test_docs_reminder.py
"""

import importlib.util
import pathlib
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("docs_reminder", ROOT / "scripts" / "docs_reminder.py")
assert spec and spec.loader
dr = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dr)


def decide(files, title="feat: x", labels=(), actor="avelino"):
    return dr.decide(files, title, set(labels), actor)


class Decide(unittest.TestCase):
    def test_user_visible_without_docs_comments(self):
        action, _, areas = decide(["crates/outl-tui/src/input/normal.rs"])
        self.assertEqual(action, "comment")
        self.assertEqual([a.area for a in areas], ["TUI"])

    def test_docs_touched_resolves(self):
        for doc in ("docs/tui.md", "README.md", "CHANGELOG.md"):
            action, _, _ = decide(["crates/outl-tui/src/a.rs", doc])
            self.assertEqual(action, "resolved", doc)

    def test_mcp_registry_beats_cli(self):
        _, _, areas = decide(["crates/outl-cli/src/mcp/tools/registry.rs"])
        self.assertEqual([a.area for a in areas], ["MCP tools"])

    def test_tests_and_internal_crates_are_quiet(self):
        for f in (
            "crates/outl-tui/src/input/tests.rs",
            "crates/outl-tui/tests/x.rs",
            "crates/outl-desktop/src/lib/a.test.ts",
            "crates/outl-core/src/op.rs",
            "crates/outl-tui/CLAUDE.md",
        ):
            self.assertEqual(decide([f])[0], "skip", f)

    def test_quiet_titles_skip_unless_forced(self):
        f = ["crates/outl-cli/src/cmd/page.rs"]
        for t in ("chore(deps): bump x", "ci: y", "test: z", "refactor(cli): w"):
            self.assertEqual(decide(f, title=t)[0], "skip", t)
            self.assertEqual(decide(f, title=t, labels=["needs-docs"])[0], "comment", t)
        self.assertEqual(decide(f, title="fix: w")[0], "comment")

    def test_labels_and_bots(self):
        f = ["crates/outl-cli/src/cmd/page.rs"]
        self.assertEqual(decide(f, labels=["no-docs-needed"])[0], "skip")
        self.assertEqual(decide(f, actor="dependabot[bot]")[0], "skip")

    def test_themes_dir_counts(self):
        self.assertEqual(decide(["crates/outl-theme/themes/nord.toml"])[0], "comment")


class Render(unittest.TestCase):
    def test_comment_has_marker_and_checklist(self):
        _, _, areas = decide(["crates/outl-shortcuts/src/defaults.rs"])
        body = dr.render("comment", areas)
        self.assertTrue(body.startswith(dr.MARKER))
        self.assertEqual(body.count("- [ ]"), 4)
        self.assertIn("docs/shortcuts.md", body)
        self.assertNotIn("—", body)

    def test_resolved_keeps_marker(self):
        self.assertTrue(dr.render("resolved", []).startswith(dr.MARKER))


class Map(unittest.TestCase):
    def test_every_mapped_page_exists(self):
        for rule in dr.RULES:
            for page in rule.pages:
                self.assertTrue((ROOT / page).is_file(), f"{rule.area}: {page} missing")


if __name__ == "__main__":
    unittest.main()
