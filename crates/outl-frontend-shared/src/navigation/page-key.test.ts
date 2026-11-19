import { createEffect, createRoot, createSignal, on } from "solid-js";
import { createStore } from "solid-js/store";
import { afterEach, describe, expect, it } from "vitest";

import { createPageKey } from "./page-key";

type Page = { id: string; slug: string; title: string } | null;

function page(slug: string, title = slug, id = `id-${slug}`): Page {
  return { id, slug, title };
}

let dispose: () => void = () => {};

afterEach(() => {
  dispose();
  dispose = () => {};
});

/**
 * Effects run when the root body completes, so both the dispose and the
 * assertions stay outside it — asserting inside reads the arrays before
 * a single effect has run, and every one of these passes vacuously.
 */
function root<T>(body: () => T): T {
  let out!: T;
  createRoot((d) => {
    dispose = d;
    out = body();
  });
  return out;
}

describe("createPageKey", () => {
  it("is undefined while no page is open", () => {
    const key = root(() => {
      const [p] = createSignal<Page>(null);
      return createPageKey(p, (x) => x.slug);
    });

    expect(key()).toBeUndefined();
  });

  it("does not fire for a new object carrying the same value", () => {
    const runs: (string | undefined)[] = [];
    const setP = root(() => {
      const [p, set] = createSignal<Page>(page("today"));
      const slug = createPageKey(p, (x) => x.slug);
      createEffect(on(slug, (s) => runs.push(s)));
      return set;
    });

    // What a commit does: a fresh object, same slug, different id.
    setP(page("today", "today", "id-2"));
    setP(page("today", "today", "id-3"));

    expect(runs).toEqual(["today"]);
  });

  it("fires when the value changes, and when the page goes away", () => {
    const runs: (string | undefined)[] = [];
    const setP = root(() => {
      const [p, set] = createSignal<Page>(page("today"));
      const slug = createPageKey(p, (x) => x.slug);
      createEffect(on(slug, (s) => runs.push(s)));
      return set;
    });

    setP(page("other"));
    setP(null);
    setP(page("other"));

    expect(runs).toEqual(["today", "other", undefined, "other"]);
  });

  it("takes an equals for a key that is an object", () => {
    const runs: unknown[] = [];
    const setP = root(() => {
      const [p, set] = createSignal<Page>(page("os-linux", "os-linux"));
      const key = createPageKey(
        p,
        (x) => ({ slug: x.slug, title: x.title }),
        (a, b) => a?.slug === b?.slug && a?.title === b?.title,
      );
      createEffect(on(key, (k) => runs.push(k)));
      return set;
    });

    setP(page("os-linux", "os-linux", "id-2"));
    setP(page("os-linux", "os/linux"));

    expect(runs).toEqual([
      { slug: "os-linux", title: "os-linux" },
      { slug: "os-linux", title: "os/linux" },
    ]);
  });

  /**
   * The exact shape of the bug, against a real store: both clients
   * write the page as one object (`setState({ page, … })`), which
   * `setState` merges key by key — so `page` is *assigned* and its node
   * fires, while a path write (`setState("page", …)`) would merge into
   * the object already there and not fire at all.
   *
   * That difference is why this was never caught: a store test written
   * the other way passes against the broken code.
   */
  it("holds against the store write every client actually performs", () => {
    const bare: (string | undefined)[] = [];
    const keyed: (string | undefined)[] = [];
    const setState = root(() => {
      const [state, set] = createStore<{ page: Page; other: number }>({
        page: page("today"),
        other: 0,
      });

      createEffect(
        on(
          () => state.page?.slug,
          (s) => bare.push(s),
        ),
      );
      const slug = createPageKey(
        () => state.page,
        (x) => x.slug,
      );
      createEffect(on(slug, (s) => keyed.push(s)));
      return set;
    });

    setState({ page: page("today", "today", "id-2"), other: 1 });
    setState({ page: page("today", "today", "id-3"), other: 2 });

    // The shape every client used to write, and what it costs.
    expect(bare).toEqual(["today", "today", "today"]);
    expect(keyed).toEqual(["today"]);
  });
});
