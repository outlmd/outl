---
description: Drive the real desktop app in a container and assert the layout holds. Use after touching the outline rows, an overlay, or desktop styles.
allowed-tools: Bash(.claude/skills/run-desktop/battery/run.sh:*), Bash(docker:*)
---

Run the layout battery:

```bash
.claude/skills/run-desktop/battery/run.sh
```

First run builds the lab image and the Linux binary (a few minutes each,
cached in named volumes afterwards). Later runs refresh only the frontend
and take seconds.

It boots the real app against a fixed fixture workspace, then points at
every content row in turn and measures whether anything moves. A control
revealed on hover must not take a line of its own: that shipped once as
`PropertyEditor`'s `+ prop`, which pushed every row below the pointer
down by 18px and flickered at row boundaries.

Report the verdict as printed. On a failure, name the row and the pixel
shift, and look for a `hidden` -> `flex` (or any display toggle) on an
element that sits in the block's flow. The fix is to anchor it instead —
`BlockRow` is already `relative`.

**What this does not cover.** The battery only asks the hover question.
Keyboard flows work in the lab and are worth driving by hand — see
`.claude/skills/run-desktop/SKILL.md` — but macOS window behaviour
(WKWebView first responder, which decides whether a modal takes the
keyboard at all) only reproduces on a real macOS window.

If `docker` is missing, say so plainly rather than reporting a pass.
