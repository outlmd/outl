import { describe, expect, it, vi } from "vitest";

import { handleReminderAction, type ReminderActionDeps } from "./reminder-actions";

/**
 * iOS's identifier for "the user tapped the banner itself" rather than
 * one of its buttons. Spelled by Apple.
 *
 * It lives in the test, not in the module: the dispatcher never names
 * it — anything that is not a known button id is a tap — so exporting
 * it from production code would be a constant nothing reads.
 */
const IOS_DEFAULT_ACTION = "com.apple.UNNotificationDefaultActionIdentifier";

/**
 * A dispatcher wired to spies, with the ids the Rust catalog ships
 * today. Tests pass ids explicitly rather than importing a constant,
 * so a rename in `reminder_action_catalog` shows up as a failing
 * assertion here instead of a test that silently follows it.
 */
function deps(over: Partial<ReminderActionDeps> = {}): ReminderActionDeps {
  return {
    buttons: [
      { id: "snooze-1h", kind: "snooze" },
      { id: "done", kind: "done" },
    ],
    snoozeReminder: vi.fn().mockResolvedValue(undefined),
    openPageBySlug: vi.fn().mockResolvedValue({ page: { id: "page-1" } }),
    markBlockDone: vi.fn().mockResolvedValue(undefined),
    navigateToBlock: vi.fn().mockResolvedValue(undefined),
    onError: vi.fn(),
    ...over,
  };
}

const subject = { extra: { blockId: "blk-1", pageSlug: "journal/2026-09-05" } };

describe("handleReminderAction", () => {
  it("snoozes an hour when the snooze button is pressed", async () => {
    const d = deps();
    const out = await handleReminderAction({ ...subject, actionId: "snooze-1h" }, d);

    expect(out).toBe("snoozed");
    expect(d.snoozeReminder).toHaveBeenCalledWith("blk-1", "1h");
    // Snooze converges on its own through Op::SnoozeRemind — resolving
    // the page would be a round trip for nothing.
    expect(d.openPageBySlug).not.toHaveBeenCalled();
    expect(d.navigateToBlock).not.toHaveBeenCalled();
  });

  it("resolves the block's own page before marking it done", async () => {
    const d = deps();
    const out = await handleReminderAction({ ...subject, actionId: "done" }, d);

    expect(out).toBe("done");
    // The banner can arrive while the user is looking at any page, so
    // the page id has to come from the reminder, never from the view.
    expect(d.openPageBySlug).toHaveBeenCalledWith("journal/2026-09-05");
    expect(d.markBlockDone).toHaveBeenCalledWith("page-1", "blk-1");
  });

  it("navigates on a plain tap, on both platforms", async () => {
    // iOS names its default action; Android sends a tap with no id.
    for (const actionId of [IOS_DEFAULT_ACTION, "", undefined]) {
      const d = deps();
      const out = await handleReminderAction({ ...subject, actionId }, d);

      expect(out).toBe("navigated");
      expect(d.navigateToBlock).toHaveBeenCalledWith("journal/2026-09-05", "blk-1");
      expect(d.markBlockDone).not.toHaveBeenCalled();
      expect(d.snoozeReminder).not.toHaveBeenCalled();
    }
  });

  it("ignores a banner that carries no subject", async () => {
    // A notification from a build before the extras existed, delivered
    // after an upgrade. Doing nothing beats acting on a guess.
    const d = deps();

    expect(await handleReminderAction({ actionId: "done" }, d)).toBe("ignored");
    expect(
      await handleReminderAction({ actionId: "done", extra: { blockId: "blk-1" } }, d),
    ).toBe("ignored");

    expect(d.markBlockDone).not.toHaveBeenCalled();
    expect(d.navigateToBlock).not.toHaveBeenCalled();
    // Not an error the user should see — there is nothing they can do.
    expect(d.onError).not.toHaveBeenCalled();
  });

  it("reports a failure instead of throwing inside the OS callback", async () => {
    // This runs in an event handler, so an exception has nowhere to go
    // and the user would be left with a banner that did nothing.
    const d = deps({
      markBlockDone: vi.fn().mockRejectedValue(new Error("workspace is locked")),
    });

    const out = await handleReminderAction({ ...subject, actionId: "done" }, d);

    expect(out).toBe("ignored");
    expect(d.onError).toHaveBeenCalledWith("workspace is locked");
  });
});
