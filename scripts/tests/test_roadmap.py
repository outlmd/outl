#!/usr/bin/env python3
"""Tests for `scripts/roadmap.py`.

The generator writes a public page nobody reviews before it publishes: the
cron renders and pushes in one step. So the two decisions it makes without
asking are what get pinned here.

`headline` rewrites a title for display. The first attempt capitalised the
first character unconditionally and turned `search_blocks` into
`Search_blocks` and `iOS:` into `IOS:` — a roadmap naming an identifier that
does not exist. Every case below is a real title from the board.

`theme` decides which bullet lands under which heading, and the order is an
editorial call rather than the crate layout: `#15` is `area:actions` **and**
`area:mobile`, and it belongs under Mobile, because the person reading uses a
phone and does not know which crate the work lands in.

Run: python3 scripts/tests/test_roadmap.py
"""

import importlib.util
import pathlib
import sys
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location(
    "roadmap", ROOT / "scripts" / "roadmap.py"
)
assert spec and spec.loader
roadmap = importlib.util.module_from_spec(spec)
# Registered before `exec_module`, not after: on Python 3.12 `@dataclass`
# resolves a string annotation through `sys.modules[cls.__module__].__dict__`
# while checking for `KW_ONLY`, and an unregistered module makes that `None`.
# 3.13 stopped needing it, so skipping this passed locally and failed on the
# runner's 3.12 with an `AttributeError` inside `dataclasses`.
sys.modules["roadmap"] = roadmap
spec.loader.exec_module(roadmap)


def item(number: int, title: str = "t", labels: list[str] | None = None) -> object:
    return roadmap.Item(
        item_id=f"i{number}",
        status="🔜 Now",
        priority=None,
        kind="Issue",
        number=number,
        title=title,
        state="OPEN",
        url=f"https://github.com/outlmd/outl/issues/{number}",
        labels=labels or [],
    )


class Headline(unittest.TestCase):
    def test_strips_a_conventional_commit_prefix(self):
        self.assertEqual(
            roadmap.headline("feat(remind): deliver reminders when the app is closed"),
            "Deliver reminders when the app is closed",
        )
        self.assertEqual(
            roadmap.headline("perf(core): SIMD-backed search via memchr"),
            "SIMD-backed search via memchr",
        )
        self.assertEqual(
            roadmap.headline("fix!: stop losing content"), "Stop losing content"
        )

    def test_leaves_an_identifier_alone(self):
        """The regression: a snake_case or camelCase first word is code."""
        for title in (
            "search_blocks sits in outl-tauri-shared, so the TUI cannot share it",
            "commit_page is single-page, and cross-page moves are not",
            "iOS: Markdown export lands in the sandbox",
        ):
            self.assertEqual(roadmap.headline(title), title)

    def test_leaves_outl_lowercase(self):
        title = "outl doctor is blind to assets/"
        self.assertEqual(roadmap.headline(title), title)

    def test_capitalises_a_plain_first_word(self):
        self.assertEqual(
            roadmap.headline("page list is 2,576 names in alphabetical order"),
            "Page list is 2,576 names in alphabetical order",
        )

    def test_a_title_that_is_only_a_prefix_survives(self):
        self.assertEqual(roadmap.headline("chore:"), "chore:")

    def test_an_already_capitalised_title_is_untouched(self):
        title = "Trash list and restore are locked in outl-cli"
        self.assertEqual(roadmap.headline(title), title)


class Theme(unittest.TestCase):
    def test_the_audit_outranks_the_area(self):
        self.assertEqual(
            item(428, labels=["phase:4", "area:tui", "kind:bug"]).theme(),
            "The TUI audit",
        )

    def test_a_client_area_outranks_the_crate_it_lands_in(self):
        self.assertEqual(
            item(15, labels=["area:actions", "area:mobile"]).theme(), "Mobile"
        )

    def test_parity_outranks_a_single_client(self):
        self.assertEqual(
            item(305, labels=["area:desktop", "area:mobile", "parity"]).theme(),
            "Parity across clients",
        )

    def test_an_unlabelled_issue_still_lands_somewhere(self):
        self.assertEqual(item(549).theme(), "Everything else")

    def test_every_theme_label_is_reachable(self):
        """A typo in THEMES is a heading that silently never appears."""
        for label, name in roadmap.THEMES:
            if label is None:
                continue
            self.assertEqual(
                item(1, labels=[label]).theme(), name, f"{label} -> {name}"
            )


class PriorityLabel(unittest.TestCase):
    def test_reads_the_label_not_the_field(self):
        """230 of 233 issues had an empty field and a populated label."""
        self.assertEqual(item(15, labels=["priority:P1"]).priority_label, "P1")

    def test_no_label_is_none(self):
        self.assertIsNone(item(549).priority_label)


class Render(unittest.TestCase):
    def test_the_roadmap_issue_does_not_list_itself(self):
        items = [
            item(roadmap.ROADMAP_ISSUE, "Roadmap", ["roadmap"]),
            item(428, "The status line needs 128 columns", ["phase:4"]),
        ]
        body = roadmap.render(items)
        self.assertIn("#428", body)
        self.assertNotIn(f"#{roadmap.ROADMAP_ISSUE}", body)

    def test_a_closed_issue_is_not_rendered(self):
        closed = item(309, "The desktop editor is 3,623 lines")
        closed.state = "CLOSED"
        closed.status = "✅ Shipped"
        body = roadmap.render([closed, item(428, "x", ["phase:4"])])
        self.assertNotIn("#309", body)

    def test_an_empty_lane_prints_no_heading(self):
        body = roadmap.render([item(428, "x", ["phase:4"])])
        self.assertIn("## Now", body)
        self.assertNotIn("## Later", body)

    def test_a_statusless_issue_is_called_out(self):
        loose = item(123, "Something nobody triaged")
        loose.status = None
        self.assertIn("## Not triaged", roadmap.render([loose]))

    def test_the_count_matches_the_bullets(self):
        items = [item(n, f"issue {n}", ["phase:4"]) for n in (1, 2, 3)]
        self.assertIn("**3 open items on the board**", roadmap.render(items))


if __name__ == "__main__":
    unittest.main(verbosity=2, argv=[sys.argv[0]])
