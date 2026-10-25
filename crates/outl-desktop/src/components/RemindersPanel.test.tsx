import { render } from "solid-js/web";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { listReminders, openPageBySlug, snoozeReminder } from "@outl/shared/api/commands";
import { EMPTY_REMINDER_NOTICE } from "@outl/shared/reminders";
import type { Reminder } from "@outl/shared/api/types";

import { RemindersPanel } from "./RemindersPanel";
import { appState, setAppState } from "../lib/store";

vi.mock("@outl/shared/api/commands", () => ({
  listReminders: vi.fn(),
  reminderSettings: vi.fn(async () => ({ enabled: true })),
  snoozePresets: vi.fn(async () => [{ id: "1h", label: "1h" }]),
  snoozeReminder: vi.fn(async () => undefined),
  clearReminderSnooze: vi.fn(),
  markBlockDone: vi.fn(),
  openPageBySlug: vi.fn(),
  formatNextFire: () => "soon",
  groupReminders: (items: unknown[]) => [{ label: "Today", items }],
}));

let dispose: (() => void) | undefined;

function reminder(id: string): Reminder {
  return {
    block_id: id,
    page_slug: "p",
    page_title: "P",
    text: `block ${id}`,
    rule: "3pm",
    anchor_date: "2026-10-23",
    done: false,
    next_fire: "2026-10-23T15:00:00",
    snoozed_until: null,
    urgency: "soon",
  };
}

function rows(host: HTMLElement): HTMLElement[] {
  return [...host.querySelectorAll<HTMLElement>("[data-reminder-row]")];
}

async function open(): Promise<HTMLElement> {
  // A fresh array per call, like the real command: the same reference would
  // not notify the resource's readers, and the test would pass for free.
  vi.mocked(listReminders).mockImplementation(async () => [reminder("a"), reminder("b")]);
  const host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(() => <RemindersPanel applyView={vi.fn()} setError={vi.fn()} />, host);
  setAppState("remindersOpen", true);
  await vi.waitFor(() => expect(rows(host)).toHaveLength(2));
  return host;
}

beforeEach(() => setAppState("remindersOpen", false));

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = "";
  vi.clearAllMocks();
});

describe("RemindersPanel focus", () => {
  it("lands on the first jump row again after a close and reopen", async () => {
    const host = await open();
    await vi.waitFor(() => expect(document.activeElement).toBe(rows(host)[0]));
    setAppState("remindersOpen", false);
    setAppState("remindersOpen", true);
    await vi.waitFor(() => expect(listReminders).toHaveBeenCalledTimes(2));
    // Let the fresh rows land and settle: asserting earlier passes on the
    // stale rows that are about to be replaced.
    await new Promise((r) => setTimeout(r, 20));
    expect(document.activeElement).toBe(rows(host)[0]);
  });

  it("lands on the first jump row, not on a snooze control, when it opens", async () => {
    const host = await open();
    await vi.waitFor(() => expect(document.activeElement).toBe(rows(host)[0]));
  });

  it("does not pull focus back to the first row when a snooze refetches", async () => {
    const host = await open();
    await vi.waitFor(() => expect(document.activeElement).toBe(rows(host)[0]));
    rows(host)[1].focus();

    host.querySelectorAll<HTMLButtonElement>("button[title^='Snooze']")[1].click();
    await vi.waitFor(() => expect(snoozeReminder).toHaveBeenCalled());
    await vi.waitFor(() => expect(listReminders).toHaveBeenCalledTimes(2));
    await new Promise((r) => setTimeout(r, 0));

    // The refetch re-renders the rows, so focus may drop to the body;
    // what it must not do is jump back to the top of the list.
    expect(document.activeElement).not.toBe(rows(host)[0]);
  });
});

describe("RemindersPanel empty block", () => {
  async function openWith(items: Reminder[]): Promise<HTMLElement> {
    vi.mocked(listReminders).mockImplementation(async () => items);
    const host = document.createElement("div");
    document.body.appendChild(host);
    dispose = render(() => <RemindersPanel applyView={vi.fn()} setError={vi.fn()} />, host);
    setAppState("remindersOpen", true);
    await vi.waitFor(() => expect(rows(host)).toHaveLength(items.length));
    return host;
  }

  it("a_reminder_whose_block_has_no_text_is_a_jump_row_labelled_with_the_notice", async () => {
    const host = await openWith([{ ...reminder("a"), text: "   " }, reminder("b")]);

    expect(rows(host)[0].textContent).toBe(EMPTY_REMINDER_NOTICE);
    expect(host.textContent).not.toContain("(empty block)");
    // Focus-on-open lands on the first row, whatever it is.
    await vi.waitFor(() => expect(document.activeElement).toBe(rows(host)[0]));
  });

  it("takes focus when every reminder is empty", async () => {
    const host = await openWith([{ ...reminder("a"), text: "" }]);
    await vi.waitFor(() => expect(document.activeElement).toBe(rows(host)[0]));
  });

  it("opens the page and selects the blank block when the notice is clicked", async () => {
    const applyView = vi.fn();
    vi.mocked(openPageBySlug).mockResolvedValue({} as never);
    vi.mocked(listReminders).mockImplementation(async () => [{ ...reminder("a"), text: "" }]);
    const host = document.createElement("div");
    document.body.appendChild(host);
    dispose = render(() => <RemindersPanel applyView={applyView} setError={vi.fn()} />, host);
    setAppState("remindersOpen", true);
    await vi.waitFor(() => expect(rows(host)).toHaveLength(1));

    rows(host)[0].click();
    await vi.waitFor(() => expect(appState.selectedBlockId).toBe("a"));
    expect(openPageBySlug).toHaveBeenCalledWith("p");
  });
});
