import { afterEach, describe, expect, it } from "vitest";
import {
  HIDE_MESSAGE,
  buildShowMessage,
  getNativeSuggesterState,
  setNativeSuggesterState,
} from "./native-suggester";
import type { PageMeta } from "@outl/shared/api/types";

afterEach(() => {
  setNativeSuggesterState(null);
});

describe("buildShowMessage", () => {
  it("strips PageMeta down to the wire shape", () => {
    const pages: PageMeta[] = [
      { id: "a", slug: "avelino", title: "Avelino", kind: "page" },
      { id: "b", slug: "2026-05-28", title: "2026-05-28", kind: "journal" },
    ];
    expect(buildShowMessage(pages)).toEqual({
      action: "show",
      items: [
        // Page pick value is the title (matches desktop refReplacement).
        { slug: "Avelino", title: "Avelino", kind: "page" },
        // Journal pick value stays the ISO slug.
        { slug: "2026-05-28", title: "2026-05-28", kind: "journal" },
      ],
    });
  });

  it("inserts the title, not the slug, for a page whose slug differs (#88)", () => {
    // Regression: the chip showed "avelino/outl" but tapping wrote the
    // slug `avelino-outl`. The picked value (the `slug` wire field) must
    // be the human-readable title so the ref renders verbatim and still
    // resolves via the slugified-match arm of open_or_create_by_ref.
    const pages: PageMeta[] = [
      { id: "a", slug: "avelino-outl", title: "avelino/outl", kind: "page" },
    ];
    const item = buildShowMessage(pages).items[0];
    expect(item.slug).toBe("avelino/outl");
    expect(item.title).toBe("avelino/outl");
  });

  it("keeps inserting the journal's ISO slug, not its long title", () => {
    const pages: PageMeta[] = [
      {
        id: "j1",
        slug: "2026-05-28",
        title: "Thursday, May 28, 2026",
        kind: "journal",
      },
    ];
    const item = buildShowMessage(pages).items[0];
    expect(item.slug).toBe("2026-05-28");
    expect(item.title).toBe("2026-05-28");
  });

  it("inserts the person's title verbatim for an @ mention", () => {
    const pages: PageMeta[] = [
      { id: "p1", slug: "avelino", title: "Avelino", kind: "page" },
    ];
    // mention → pick value is the title (applySuggestion wraps it
    // as `[[@Avelino]]`).
    expect(buildShowMessage(pages, { kind: "mention" }).items[0].slug).toBe(
      "Avelino",
    );
  });

  it("inserts the slug, not the title, for a # tag (#15)", () => {
    // Regression. `#` has no closer, so the first space ends the tag:
    // handing back "Deep Work" writes `#Deep Work`, which tags `#Deep`
    // and leaves "Work" as loose prose. The desktop got this right
    // from the start because its accept passes `tag` to
    // `refReplacement`; this path passed `{ mention }` alone, so a
    // tag took the title branch. Nothing was red — no test reached
    // the mobile tag accept at all.
    const pages: PageMeta[] = [
      { id: "t1", slug: "deep-work", title: "Deep Work", kind: "page" },
    ];
    expect(buildShowMessage(pages, { kind: "tag" }).items[0].slug).toBe(
      "deep-work",
    );
    // The chip still *shows* the human title; only the pick differs.
    expect(buildShowMessage(pages, { kind: "tag" }).items[0].title).toBe(
      "Deep Work",
    );
  });

  it("keeps a journal's ISO slug for a tag too", () => {
    const pages: PageMeta[] = [
      {
        id: "j1",
        slug: "2026-11-11",
        title: "Wednesday, November 11, 2026",
        kind: "journal",
      },
    ];
    expect(buildShowMessage(pages, { kind: "tag" }).items[0].slug).toBe(
      "2026-11-11",
    );
  });

  it("replaces a journal's human title with its ISO slug", () => {
    const pages: PageMeta[] = [
      {
        id: "j1",
        slug: "2026-05-28",
        title: "Thursday, May 28, 2026",
        kind: "journal",
      },
    ];
    expect(buildShowMessage(pages).items[0].title).toBe("2026-05-28");
  });

  it("emits an empty items array when no pages match", () => {
    expect(buildShowMessage([])).toEqual({ action: "show", items: [] });
  });
});

describe("HIDE_MESSAGE", () => {
  it("is the literal hide payload", () => {
    expect(HIDE_MESSAGE).toEqual({ action: "hide" });
  });
});

describe("setNativeSuggesterState / getNativeSuggesterState", () => {
  it("starts null", () => {
    expect(getNativeSuggesterState()).toBeNull();
  });

  it("round-trips a show message through the global", () => {
    const msg = buildShowMessage([
      { id: "a", slug: "avelino", title: "Avelino", kind: "page" },
    ]);
    setNativeSuggesterState(msg);
    expect(getNativeSuggesterState()).toEqual(msg);
  });

  it("clears with HIDE_MESSAGE", () => {
    setNativeSuggesterState(buildShowMessage([]));
    setNativeSuggesterState(HIDE_MESSAGE);
    expect(getNativeSuggesterState()).toEqual({ action: "hide" });
  });

  it("clears entirely with null", () => {
    setNativeSuggesterState(buildShowMessage([]));
    setNativeSuggesterState(null);
    expect(getNativeSuggesterState()).toBeNull();
  });
});
