import { render } from "solid-js/web";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const api = vi.hoisted(() => ({ getSettings: vi.fn(), updateSettings: vi.fn() }));
vi.mock("../lib/api", () => api);

const os = vi.hoisted(() => ({ platform: vi.fn() }));
vi.mock("@tauri-apps/plugin-os", () => os);

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("@outl/shared/api/commands", () => ({
  listThemes: vi.fn().mockResolvedValue([]),
}));
vi.mock("@outl/shared/api/telemetry", () => ({
  getTelemetryEnabled: vi.fn().mockResolvedValue(true),
  setTelemetryEnabled: vi.fn(),
}));
vi.mock("@outl/shared/theme", () => ({
  installTheme: vi.fn(),
  pickSide: () => "light",
}));
vi.mock("@outl/shared/diagram", () => ({ setDiagramsMode: vi.fn() }));
vi.mock("./SyncPanel", () => ({ SyncPanel: () => null }));

import { SettingsModal } from "./SettingsModal";
import { appState, setAppState } from "../lib/store";

const settings = (last_workspace: string) => ({
  vim_mode: false,
  theme: "a",
  theme_dark: "b",
  theme_mode: "auto",
  diagrams: "pele",
  font_size: 15,
  last_workspace,
  reminders_enabled: true,
  reminders_quiet_hours: "",
  sync_transport: "iroh",
  managed: false,
});

let dispose: (() => void) | undefined;

async function mount(onSwitchWorkspace: () => Promise<boolean>) {
  const host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(() => <SettingsModal onSwitchWorkspace={onSwitchWorkspace} />, host);
  setAppState("settingsOpen", true);
  await vi.waitFor(() => expect(buttonByText("Switch…")).toBeTruthy());
  return host;
}

function buttonByText(text: string): HTMLButtonElement | undefined {
  return [...document.querySelectorAll("button")].find(
    (b) => b.textContent?.trim() === text,
  ) as HTMLButtonElement | undefined;
}

beforeEach(() => {
  api.getSettings.mockReset().mockResolvedValue(settings("/ws/one"));
  os.platform.mockReset().mockReturnValue("linux");
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  setAppState("settingsOpen", false);
  document.body.innerHTML = "";
});

describe("SettingsModal workspace switch", () => {
  it("runs the switch flow from the Switch… button", async () => {
    const onSwitch = vi.fn().mockResolvedValue(false);
    await mount(onSwitch);
    buttonByText("Switch…")!.click();
    expect(onSwitch).toHaveBeenCalledOnce();
  });

  it("closes the modal once the workspace was switched", async () => {
    await mount(vi.fn().mockResolvedValue(true));
    buttonByText("Switch…")!.click();
    await vi.waitFor(() => expect(appState.settingsOpen).toBe(false));
  });

  it("stays open when the switch was cancelled or failed", async () => {
    const onSwitch = vi.fn().mockResolvedValue(false);
    await mount(onSwitch);
    buttonByText("Switch…")!.click();
    await vi.waitFor(() => expect(onSwitch).toHaveBeenCalled());
    await Promise.resolve();
    expect(appState.settingsOpen).toBe(true);
  });

  it("does not mention a File menu off macOS", async () => {
    const host = await mount(vi.fn());
    expect(host.textContent).toContain("Use the button to change.");
    expect(host.textContent).not.toContain("File →");
  });

  it("mentions File → Switch Workspace… on macOS", async () => {
    os.platform.mockReturnValue("macos");
    const host = await mount(vi.fn());
    expect(host.textContent).toContain(
      "Use File → Switch Workspace… or the button to change.",
    );
  });

  it("shows the current workspace after it changed behind the modal", async () => {
    const host = await mount(vi.fn());
    await vi.waitFor(() => expect(host.textContent).toContain("/ws/one"));
    setAppState("settingsOpen", false);
    api.getSettings.mockResolvedValue(settings("/ws/two"));
    setAppState("settingsOpen", true);
    await vi.waitFor(() => expect(host.textContent).toContain("/ws/two"));
  });
});
