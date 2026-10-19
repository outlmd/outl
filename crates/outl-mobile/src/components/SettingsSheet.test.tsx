/**
 * The header gear opened nothing at all.
 *
 * `SettingsSheet` drew its iOS toggle with a local helper component named
 * `Switch`, and `babel-preset-solid` auto-imports Solid's control-flow
 * `<Switch>` for that tag name regardless of what the module declares —
 * so `<Switch on={locked()} />` compiled to the built-in and the local
 * function was never called. A childless built-in `<Switch>` reads
 * `undefined.when` inside Solid's own memo and throws, which took down
 * the whole sheet render: tapping the gear flipped `settingsOpen` and
 * painted nothing.
 *
 * These pin that the sheet renders when `open` is set and that the toggle
 * in it is the one this file draws, so a helper named after a Solid
 * built-in fails here instead of on a device.
 */

import { render } from "solid-js/web";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("../lib/haptics", () => ({ haptic: () => {} }));

// Crash reporting is the one row here that talks to the backend, so the
// suite supplies the config instead of a Tauri bridge. `saved` is what
// the command persisted, which is also what the switch is allowed to
// move to — the row never trusts its own optimistic guess.
let savedReports = true;
let setReportsThrows = false;
let getReportsThrows = false;
const setReportsCalls: boolean[] = [];

vi.mock("@outl/shared/api/telemetry", () => ({
  getTelemetryEnabled: async () => {
    if (getReportsThrows) throw new Error("no config");
    return savedReports;
  },
  setTelemetryEnabled: async (enabled: boolean) => {
    setReportsCalls.push(enabled);
    if (setReportsThrows) throw new Error("read-only config");
    savedReports = enabled;
    return savedReports;
  },
}));

import { SettingsSheet } from "./SettingsSheet";

let dispose: (() => void) | undefined;

/** Let the sheet's `getTelemetryEnabled()` round-trip settle. */
function settle(): Promise<void> {
  return new Promise((r) => setTimeout(r, 0));
}

function mount() {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const onClose = vi.fn();
  const onOpenDevices = vi.fn();
  dispose = render(
    () => (
      <SettingsSheet open onClose={onClose} onOpenDevices={onOpenDevices} />
    ),
    host,
  );
  return { onClose, onOpenDevices };
}

/** The row that reaches pairing, found by its label rather than by
 *  position: it has to stay findable if the sheet grows. */
function devicesRow(): HTMLButtonElement | undefined {
  return [...document.body.querySelectorAll("button")].find((b) =>
    b.textContent?.includes("Devices"),
  ) as HTMLButtonElement | undefined;
}

/** A `role="switch"` row, found by its label.
 *
 *  **Not `querySelector('[role="switch"]')`.** That returned the first
 *  switch in the DOM, so the three lock tests below silently followed
 *  whatever row happened to be painted first — they stayed green while
 *  testing the crash-reports toggle the moment it landed above or below
 *  them. Same reason `devicesRow()` searches by text. */
function switchRow(label: string): HTMLButtonElement | undefined {
  return [...document.body.querySelectorAll('[role="switch"]')].find((b) =>
    b.textContent?.includes(label),
  ) as HTMLButtonElement | undefined;
}

function lockRow(): HTMLButtonElement | undefined {
  return switchRow("Lock button order");
}

function reportsRow(): HTMLButtonElement | undefined {
  return switchRow("Crash reports");
}

afterEach(() => {
  // happy-dom here exposes no `localStorage`; the toolbar store is
  // defensive about that, so the sheet renders either way.
  globalThis.localStorage?.clear();
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = "";
  savedReports = true;
  setReportsThrows = false;
  getReportsThrows = false;
  setReportsCalls.length = 0;
});

describe("SettingsSheet", () => {
  it("renders the toolbar lock row when open", () => {
    mount();
    const row = lockRow();
    expect(row).toBeDefined();
    expect(row!.textContent).toContain("Lock button order");
    expect(row!.getAttribute("aria-checked")).toBe("false");
  });

  it("draws its own toggle, not a Solid control-flow component", () => {
    mount();
    // The knob only exists if the local helper ran: a Solid built-in of
    // the same tag name renders its children instead (and throws when
    // there are none).
    const knob = lockRow()!.querySelector<HTMLElement>(
      '[aria-hidden="true"] > span',
    );
    expect(knob).not.toBeNull();
    expect(knob!.style.left).toBe("2px");
  });

  it("moves the knob and flips aria-checked on tap", () => {
    mount();
    lockRow()!.click();
    expect(lockRow()!.getAttribute("aria-checked")).toBe("true");
    const knob = lockRow()!.querySelector<HTMLElement>(
      '[aria-hidden="true"] > span',
    );
    expect(knob!.style.left).toBe("22px");
  });

  // P2P pairing was reachable only by pressing and holding the sync dot
  // (`64fe9c53`), a gesture nothing announces, and Settings never named
  // it — so on the phone the feature read as gone. The desktop has had
  // Settings → Sync the whole time.
  it("offers device sync, the way the desktop's Settings does", () => {
    const { onOpenDevices } = mount();
    const row = devicesRow();
    expect(row).toBeDefined();
    row!.click();
    expect(onOpenDevices).toHaveBeenCalledTimes(1);
  });

  // `DevicesSheet` is `z-50` and this sheet is `z-[55]`, so leaving this
  // one open would paint the devices sheet underneath it.
  it("closes itself when it hands over to the devices sheet", () => {
    const { onClose } = mount();
    devicesRow()!.click();
    expect(onClose).toHaveBeenCalled();
  });
});

// Crash reporting is on by default and there is no `outl` binary on a
// phone, so this row is the only opt-out a mobile user has (RFC 0377
// section 7). A switch that looks right and writes nothing is therefore
// not a UI nit.
describe("SettingsSheet — crash reports", () => {
  it("shows the shared copy and the state the config actually holds", async () => {
    savedReports = false;
    mount();
    await settle();
    const row = reportsRow();
    expect(row).toBeDefined();
    // The promise, not the sentence — the copy's one owner is
    // `@outl/shared/onboarding`, pinned by its own test.
    expect(row!.textContent?.toLowerCase()).toContain("never");
    expect(row!.getAttribute("aria-checked")).toBe("false");
  });

  it("reads the default as on rather than assuming off", async () => {
    mount();
    await settle();
    expect(reportsRow()!.getAttribute("aria-checked")).toBe("true");
  });

  it("writes the flip and moves only after the write lands", async () => {
    mount();
    await settle();
    reportsRow()!.click();
    await settle();
    expect(setReportsCalls).toEqual([false]);
    expect(reportsRow()!.getAttribute("aria-checked")).toBe("false");
    expect(savedReports).toBe(false);
  });

  // The failure this exists for: an optimistic flip leaves a switch
  // reading "off" over a reporter that is still on.
  it("leaves the switch where it was when the write fails", async () => {
    setReportsThrows = true;
    mount();
    await settle();
    reportsRow()!.click();
    await settle();
    expect(setReportsCalls).toEqual([false]);
    expect(reportsRow()!.getAttribute("aria-checked")).toBe("true");
    expect(reportsRow()!.textContent).toContain("could not read or save");
  });

  // Same rule for the read: the default is on, so falling back to it
  // would tell someone who opted out that they had not.
  it("refuses to guess, and to write, when it could not read the flag", async () => {
    getReportsThrows = true;
    mount();
    await settle();
    const row = reportsRow();
    expect(row!.getAttribute("aria-checked")).toBe("false");
    expect(row!.textContent).toContain("could not read or save");
    row!.click();
    await settle();
    expect(setReportsCalls).toEqual([]);
  });

  // The lock row sits above this one; both are `role="switch"`, so a
  // positional lookup would silently cross them.
  it("is a different row from the toolbar lock", async () => {
    mount();
    await settle();
    expect(reportsRow()).not.toBe(lockRow());
    reportsRow()!.click();
    await settle();
    expect(lockRow()!.getAttribute("aria-checked")).toBe("false");
  });
});
