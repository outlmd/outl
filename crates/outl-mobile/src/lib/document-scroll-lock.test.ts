/**
 * The decision this file pins: with the keyboard down, the document sits
 * at the top; with a text field focused, the scroll belongs to WebKit
 * (that is what keeps the caret above the keyboard) and we leave it.
 */
import { readFileSync } from "node:fs";
import { join } from "node:path";

import { afterEach, describe, expect, it, vi } from "vitest";

import {
  installDocumentScrollLock,
  isEditing,
  isScrolled,
  resetDocumentScroll,
} from "./document-scroll-lock";

function scrollTo(top: number) {
  const se = document.scrollingElement as HTMLElement;
  se.scrollTop = top;
}

afterEach(() => {
  document.body.innerHTML = "";
  scrollTo(0);
  vi.useRealTimers();
});

describe("isEditing", () => {
  it("is false with nothing focused", () => {
    expect(isEditing(document)).toBe(false);
  });

  it("is true for a focused textarea — the block editor", () => {
    const el = document.createElement("textarea");
    document.body.appendChild(el);
    el.focus();
    expect(isEditing(document)).toBe(true);
  });

  it("is true for a focused input — every sheet's search box", () => {
    const el = document.createElement("input");
    document.body.appendChild(el);
    el.focus();
    expect(isEditing(document)).toBe(true);
  });

  it("is false for a focused button — a tap on the chrome keeps the lock on", () => {
    const el = document.createElement("button");
    document.body.appendChild(el);
    el.focus();
    expect(isEditing(document)).toBe(false);
  });
});

describe("resetDocumentScroll", () => {
  it("puts a scrolled document back when nothing is being typed into", () => {
    scrollTo(240);
    expect(isScrolled(document)).toBe(true);

    expect(resetDocumentScroll(document)).toBe(true);
    expect(document.scrollingElement?.scrollTop).toBe(0);
  });

  it("leaves the scroll alone while a text field is focused", () => {
    const el = document.createElement("textarea");
    document.body.appendChild(el);
    el.focus();
    scrollTo(240);

    expect(resetDocumentScroll(document)).toBe(false);
    expect(document.scrollingElement?.scrollTop).toBe(240);
  });

  it("does nothing when the document is already at the top", () => {
    expect(resetDocumentScroll(document)).toBe(false);
  });
});

describe("the stylesheet's half of the same rule", () => {
  const css = readFileSync(join(__dirname, "..", "styles.css"), "utf8");

  it("declares the shell non-scrollable, so content can never push the chrome off", () => {
    // The JS lock answers WebKit scrolling the document on its own; this
    // answers a box growing past the viewport. Neither covers the other.
    expect(css.replace(/\s+/g, " ")).toContain("html, body { overflow: hidden; }");
  });
});

describe("installDocumentScrollLock", () => {
  it("resets on the document's own scroll event", () => {
    const stop = installDocumentScrollLock(window);
    scrollTo(180);
    window.dispatchEvent(new Event("scroll"));
    expect(document.scrollingElement?.scrollTop).toBe(0);
    stop();
  });

  it("resets after the keyboard animates away, not only on the blur frame", () => {
    vi.useFakeTimers();
    const stop = installDocumentScrollLock(window);
    const el = document.createElement("textarea");
    document.body.appendChild(el);
    el.focus();
    scrollTo(300);

    // The blur fires while WebKit is still animating the keyboard out, so
    // the immediate pass can land before the scroll settles.
    el.blur();
    document.dispatchEvent(new Event("focusout"));
    vi.advanceTimersByTime(400);

    expect(document.scrollingElement?.scrollTop).toBe(0);
    stop();
  });

  it("stops listening once torn down", () => {
    const stop = installDocumentScrollLock(window);
    stop();
    scrollTo(120);
    window.dispatchEvent(new Event("scroll"));
    expect(document.scrollingElement?.scrollTop).toBe(120);
  });
});
