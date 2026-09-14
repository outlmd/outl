import { describe, expect, it } from "vitest";
import { createRoot, createSignal } from "solid-js";

import { backlinksKeyOf, createBacklinksKey, sameBacklinksKey } from "./backlinks-key";

describe("backlinksKeyOf", () => {
  it("is undefined while no page is open", () => {
    expect(backlinksKeyOf(null)).toBeUndefined();
    expect(backlinksKeyOf(undefined)).toBeUndefined();
  });

  it("carries the slug and the title", () => {
    expect(backlinksKeyOf({ slug: "os-linux", title: "os/linux" })).toEqual({
      slug: "os-linux",
      title: "os/linux",
    });
  });
});

describe("sameBacklinksKey", () => {
  it("compares by value", () => {
    expect(
      sameBacklinksKey({ slug: "a", title: "A" }, { slug: "a", title: "A" }),
    ).toBe(true);
    expect(sameBacklinksKey(undefined, undefined)).toBe(true);
  });

  it("treats a rename on the same slug as a different key", () => {
    expect(
      sameBacklinksKey(
        { slug: "os-linux", title: "os-linux" },
        { slug: "os-linux", title: "os/linux" },
      ),
    ).toBe(false);
    expect(sameBacklinksKey({ slug: "a", title: "A" }, undefined)).toBe(false);
  });
});

describe("createBacklinksKey", () => {
  it("keeps the same object across a block edit and changes on a rename", () => {
    createRoot((dispose) => {
      const [page, setPage] = createSignal<{ slug: string; title: string } | null>({
        slug: "os-linux",
        title: "os-linux",
      });
      const key = createBacklinksKey(page);
      const first = key();
      expect(first).toEqual({ slug: "os-linux", title: "os-linux" });

      // A new view object with the same slug and title: no refetch.
      setPage({ slug: "os-linux", title: "os-linux" });
      expect(key()).toBe(first);

      // The `title::` edit that moves the page into a namespace: refetch.
      setPage({ slug: "os-linux", title: "os/linux" });
      expect(key()).not.toBe(first);
      expect(key()).toEqual({ slug: "os-linux", title: "os/linux" });

      setPage(null);
      expect(key()).toBeUndefined();
      dispose();
    });
  });
});
