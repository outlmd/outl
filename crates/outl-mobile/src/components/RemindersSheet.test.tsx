/**
 * A reminder whose block has no text of its own (`remind:: 3pm` on an
 * otherwise empty bullet) is still a button, labelled with
 * `EMPTY_REMINDER_NOTICE` in place of a title (#364).
 *
 * The reason it is not an inert explanation, because that is what stops
 * it being reverted: opening is the one genuinely useful thing such a
 * row can do. The desktop's `jumpTo` also sets `selectedBlockId`, so the
 * click puts the cursor on the blank bullet — exactly where the user has
 * to type for the row to ever say anything else. This client opens the
 * hosting page instead (block selection is the zoom root here, see
 * `jumpTo`), which is the same destination one gesture short. So the row
 * stays tappable on both, and keeps its urgency colour with it.
 */
import { render } from "solid-js/web";
import { afterEach, describe, expect, it, vi } from "vitest";

import { EMPTY_REMINDER_NOTICE } from "@outl/shared/reminders";
import { listReminders, openPageBySlug } from "@outl/shared/api/commands";
import type { Reminder } from "@outl/shared/api/types";

vi.mock("@outl/shared/api/commands", () => ({
  listReminders: vi.fn(),
  reminderSettings: vi.fn(async () => ({ enabled: true, quiet_hours: "" })),
  snoozePresets: vi.fn(async () => [{ id: "1h", label: "1h" }]),
  snoozeReminder: vi.fn(async () => undefined),
  clearReminderSnooze: vi.fn(),
  setReminderSettings: vi.fn(),
  markBlockDone: vi.fn(),
  openPageBySlug: vi.fn(),
  formatNextFire: () => "soon",
  groupReminders: (items: unknown[]) => [{ label: "Today", items }],
}));

import { RemindersSheet } from "./RemindersSheet";

let dispose: (() => void) | undefined;
afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = "";
  vi.clearAllMocks();
});

function reminder(id: string, text: string): Reminder {
  return {
    block_id: id,
    page_slug: "p",
    page_title: "P",
    text,
    rule: "3pm",
    anchor_date: "2026-10-23",
    done: false,
    next_fire: "2026-10-23T15:00:00",
    snoozed_until: null,
    urgency: "soon",
  };
}

describe("RemindersSheet empty block", () => {
  it("a_reminder_whose_block_has_no_text_is_a_button_labelled_with_the_notice", async () => {
    vi.mocked(listReminders).mockResolvedValue([
      { ...reminder("a", "  "), urgency: "overdue" },
      reminder("b", "call mum"),
    ]);
    vi.mocked(openPageBySlug).mockResolvedValue({} as never);
    const onView = vi.fn();
    const host = document.createElement("div");
    document.body.appendChild(host);
    dispose = render(
      () => (
        <RemindersSheet open onClose={() => {}} onMessage={() => {}} onView={onView} currentSlug={null} />
      ),
      host,
    );
    await vi.waitFor(() => expect(host.textContent).toContain("call mum"));

    expect(host.textContent).not.toContain("(empty block)");
    const jumpButtons = [...host.querySelectorAll("button")].filter((b) =>
      b.textContent?.includes("soon"),
    );
    expect(jumpButtons).toHaveLength(2);
    expect(jumpButtons[0].textContent).toContain(EMPTY_REMINDER_NOTICE);
    // Urgency colouring is not lost on the empty row, and it is the
    // palette's token — the same one the desktop row paints with
    // (root CLAUDE.md invariant 13), never Tailwind's own red.
    expect(
      jumpButtons[0].querySelector('[class*="text-(--color-outl-destructive)"]'),
    ).not.toBeNull();

    jumpButtons[0].click();
    await vi.waitFor(() => expect(onView).toHaveBeenCalled());
    expect(openPageBySlug).toHaveBeenCalledWith("p");
  });
});
