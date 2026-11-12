import { beforeEach, describe, expect, it, vi } from "vitest";

const dialog = vi.hoisted(() => ({ open: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => dialog);

const api = vi.hoisted(() => ({ setWorkspace: vi.fn() }));
vi.mock("./api", () => api);

const events = vi.hoisted(() => ({ listen: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => events);

import { onMenuSwitchWorkspace } from "./events";
import { appState, setAppState } from "./store";
import {
  SWITCH_WORKSPACE_DIALOG_TITLE,
  resetPageState,
  switchWorkspace,
} from "./switch-workspace";

const deps = () => ({
  setError: vi.fn(),
  reload: vi.fn().mockResolvedValue(undefined),
});

beforeEach(() => {
  dialog.open.mockReset();
  api.setWorkspace.mockReset().mockResolvedValue(undefined);
  events.listen.mockReset();
  resetPageState();
});

describe("switchWorkspace", () => {
  it("opens a folder dialog titled for the switch", async () => {
    dialog.open.mockResolvedValue(null);
    await switchWorkspace(deps());
    expect(dialog.open).toHaveBeenCalledWith({
      directory: true,
      multiple: false,
      title: SWITCH_WORKSPACE_DIALOG_TITLE,
    });
  });

  it("switches to the picked folder and reloads", async () => {
    dialog.open.mockResolvedValue("/ws/other");
    const d = deps();
    await switchWorkspace(d);
    expect(api.setWorkspace).toHaveBeenCalledWith("/ws/other");
    expect(d.reload).toHaveBeenCalledOnce();
    expect(d.setError).not.toHaveBeenCalled();
  });

  it("does nothing when the dialog is cancelled", async () => {
    dialog.open.mockResolvedValue(null);
    const d = deps();
    await switchWorkspace(d);
    expect(api.setWorkspace).not.toHaveBeenCalled();
    expect(d.reload).not.toHaveBeenCalled();
    expect(d.setError).not.toHaveBeenCalled();
  });

  it("surfaces a failed switch and keeps the current page", async () => {
    setAppState({ selectedBlockId: "b1" });
    dialog.open.mockResolvedValue("/ws/broken");
    api.setWorkspace.mockRejectedValue("open workspace at /ws/broken: locked");
    const d = deps();
    await switchWorkspace(d);
    expect(d.setError).toHaveBeenCalledWith("open workspace at /ws/broken: locked");
    expect(d.reload).not.toHaveBeenCalled();
    expect(appState.selectedBlockId).toBe("b1");
  });

  it("clears what pointed at the old workspace before reloading", async () => {
    setAppState({
      selectedBlockId: "b1",
      editingBlockId: "b1",
      mode: "edit",
      backlinks: [{ block_id: "x" } as never],
    });
    dialog.open.mockResolvedValue("/ws/other");
    const d = deps();
    d.reload.mockImplementation(async () => {
      expect(appState.selectedBlockId).toBeNull();
      expect(appState.editingBlockId).toBeNull();
      expect(appState.mode).toBe("normal");
      expect(appState.backlinks).toEqual([]);
      expect(appState.page).toBeNull();
    });
    await switchWorkspace(d);
    expect(d.reload).toHaveBeenCalledOnce();
  });

  it("ignores a second request while the dialog is open", async () => {
    let release: (v: string | null) => void = () => {};
    dialog.open.mockReturnValue(new Promise((r) => (release = r)));
    const first = switchWorkspace(deps());
    await switchWorkspace(deps());
    expect(dialog.open).toHaveBeenCalledTimes(1);
    release(null);
    await first;
  });
});

describe("onMenuSwitchWorkspace", () => {
  it("listens for the event the native menu emits", async () => {
    events.listen.mockResolvedValue(() => {});
    const handler = vi.fn();
    await onMenuSwitchWorkspace(handler);
    expect(events.listen.mock.calls[0][0]).toBe("menu://switch-workspace");
    events.listen.mock.calls[0][1]();
    expect(handler).toHaveBeenCalledOnce();
  });
});
