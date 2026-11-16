import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  applySafeAreaInsets,
  installSafeArea,
  parseSafeAreaInsets,
  type SafeAreaGlobals,
} from "./safe-area";

const css = (side: string) =>
  document.documentElement.style.getPropertyValue(`--safe-area-inset-${side}`);

beforeEach(() => {
  document.documentElement.removeAttribute("style");
});

describe("parseSafeAreaInsets", () => {
  it("accepts an object and a JSON string", () => {
    const insets = { top: 59, right: 0, bottom: 34, left: 0 };
    expect(parseSafeAreaInsets(insets)).toEqual(insets);
    expect(parseSafeAreaInsets(JSON.stringify(insets))).toEqual(insets);
  });

  it("drops sides that are negative, non-finite or not numbers", () => {
    expect(
      parseSafeAreaInsets({ top: -1, right: Number.NaN, bottom: "9", left: 12 }),
    ).toEqual({ left: 12 });
  });

  it("returns nothing for payloads that are not insets", () => {
    expect(parseSafeAreaInsets(null)).toEqual({});
    expect(parseSafeAreaInsets("not json")).toEqual({});
    expect(parseSafeAreaInsets(42)).toEqual({});
  });

  it("keeps fractional CSS pixels, rounded to two places", () => {
    expect(parseSafeAreaInsets({ top: 24.123456 })).toEqual({ top: 24.12 });
  });
});

describe("applySafeAreaInsets", () => {
  it("writes each side as a px custom property on :root", () => {
    applySafeAreaInsets({ top: 59, right: 0, bottom: 34, left: 44 });
    expect(css("top")).toBe("59px");
    expect(css("right")).toBe("0px");
    expect(css("bottom")).toBe("34px");
    expect(css("left")).toBe("44px");
  });

  it("clears a side that is no longer published so env() applies again", () => {
    applySafeAreaInsets({ top: 59, bottom: 34 });
    applySafeAreaInsets({ top: 47 });
    expect(css("top")).toBe("47px");
    expect(css("bottom")).toBe("");
  });
});

describe("installSafeArea", () => {
  const win = () => window as Window & SafeAreaGlobals;

  beforeEach(() => {
    delete win().__outlSafeArea;
    delete win().__outlApplySafeArea;
    delete win().OutlSafeArea;
    delete win().webkit;
  });

  it("applies a value native pushed before the app booted", () => {
    win().__outlSafeArea = { top: 52, right: 0, bottom: 34, left: 0 };
    installSafeArea();
    expect(css("top")).toBe("52px");
    expect(css("bottom")).toBe("34px");
  });

  it("reads the Android interface when nothing was pushed", () => {
    win().OutlSafeArea = {
      get: () => JSON.stringify({ top: 28, right: 0, bottom: 48, left: 0 }),
    };
    installSafeArea();
    expect(css("top")).toBe("28px");
    expect(css("bottom")).toBe("48px");
  });

  it("asks iOS to push, and applies what it pushes back", () => {
    const postMessage = vi.fn(() => {
      win().__outlApplySafeArea?.({ top: 59, right: 0, bottom: 34, left: 0 });
    });
    win().webkit = { messageHandlers: { outlSafeArea: { postMessage } } };
    installSafeArea();
    expect(postMessage).toHaveBeenCalledOnce();
    expect(css("top")).toBe("59px");
  });

  it("follows later pushes, such as a rotation", () => {
    installSafeArea();
    win().__outlApplySafeArea?.({ top: 0, right: 47, bottom: 21, left: 47 });
    expect(css("top")).toBe("0px");
    expect(css("left")).toBe("47px");
    expect(win().__outlSafeArea).toEqual({ top: 0, right: 47, bottom: 21, left: 47 });
  });

  it("leaves :root alone when there is no native side", () => {
    installSafeArea();
    expect(document.documentElement.getAttribute("style") ?? "").toBe("");
  });
});
