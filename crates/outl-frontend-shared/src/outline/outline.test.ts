import { describe, expect, it } from "vitest";

import type { BlockNode, InlineToken } from "../api/types";

import {
  collectBlockRefHandles,
  countDescendants,
  embedOnlyHandle,
  findBlock,
  flattenAll,
  flattenNodes,
  flattenParents,
  flattenVisible,
  focusSubtree,
  isInVisualRange,
  nextVisibleId,
  previousVisibleId,
  rawTextWithTodo,
  sameCrumbTrail,
  visualRangeIds,
  visualRangeSet,
  cycleTodo,
} from "./index";

function block(
  id: string,
  opts: {
    text?: string;
    todo?: "TODO" | "DONE" | null;
    collapsed?: boolean;
    children?: BlockNode[];
  } = {},
): BlockNode {
  return {
    id,
    text: opts.text ?? id,
    todo: opts.todo ?? null,
    tokens: [],
    collapsed: opts.collapsed ?? false,
    properties: [],
    children: opts.children ?? [],
  };
}

describe("rawTextWithTodo", () => {
  it("returns text verbatim when there is no TODO state", () => {
    expect(rawTextWithTodo(block("a", { text: "ship it" }))).toBe("ship it");
  });

  it("reattaches TODO prefix", () => {
    expect(rawTextWithTodo(block("x", { text: "ship it", todo: "TODO" }))).toBe(
      "TODO ship it",
    );
  });

  it("rebuilds in the canonical word form, losing the checkbox spelling", () => {
    // The DTO carries `todo` separately from `text`, so a block the
    // user wrote as `[ ] ship it` arrives here indistinguishable from
    // one written `TODO ship it`. This function cannot tell them
    // apart and always rebuilds the word form.
    //
    // That is why a client seeding an editor draft from this MUST
    // compare against it before committing: writing the draft back
    // unconditionally rewrites the user's spelling on a focus/blur
    // with no keystroke. The mobile client shipped exactly that bug.
    expect(rawTextWithTodo(block("x", { text: "ship it", todo: "TODO" }))).toBe(
      "TODO ship it",
    );
    // Same input the backend produces for `[ ] ship it` — same output.
    // The guard is the caller's job, not this function's.
  });

  it("reattaches DONE prefix", () => {
    expect(rawTextWithTodo(block("x", { text: "ship it", todo: "DONE" }))).toBe(
      "DONE ship it",
    );
  });
});

describe("findBlock", () => {
  it("finds a top-level block", () => {
    const tree = [block("a"), block("b")];
    expect(findBlock(tree, "b")?.id).toBe("b");
  });

  it("descends into children recursively", () => {
    const tree = [
      block("a", { children: [block("a1", { children: [block("a1a")] })] }),
    ];
    expect(findBlock(tree, "a1a")?.id).toBe("a1a");
  });

  it("returns null when the id is not present", () => {
    expect(findBlock([block("a")], "missing")).toBeNull();
  });
});

describe("flattenNodes", () => {
  it("walks DFS preorder", () => {
    const tree = [
      block("a", { children: [block("a1"), block("a2")] }),
      block("b", { children: [block("b1")] }),
    ];
    expect(flattenNodes(tree).map((b) => b.id)).toEqual([
      "a",
      "a1",
      "a2",
      "b",
      "b1",
    ]);
  });

  it("returns an empty list for an empty tree", () => {
    expect(flattenNodes([])).toEqual([]);
  });

  it("includes children of collapsed nodes (fold state is ignored)", () => {
    const tree = [
      block("a", { collapsed: true, children: [block("a1")] }),
    ];
    expect(flattenNodes(tree).map((b) => b.id)).toEqual(["a", "a1"]);
  });
});

describe("countDescendants", () => {
  it("returns 0 for a leaf", () => {
    expect(countDescendants(block("a"))).toBe(0);
  });

  it("counts direct children", () => {
    const b = block("p", { children: [block("c1"), block("c2")] });
    expect(countDescendants(b)).toBe(2);
  });

  it("counts nested descendants", () => {
    const b = block("p", {
      children: [
        block("c1", { children: [block("c1a"), block("c1b")] }),
        block("c2"),
      ],
    });
    // c1 + c1a + c1b + c2 = 4
    expect(countDescendants(b)).toBe(4);
  });

  it("does not count the block itself", () => {
    expect(countDescendants(block("solo"))).toBe(0);
  });
});

describe("flattenVisible", () => {
  it("walks parents before children, siblings in order", () => {
    const tree: BlockNode[] = [
      block("a", { children: [block("a1"), block("a2")] }),
      block("b"),
    ];
    expect(flattenVisible(tree)).toEqual(["a", "a1", "a2", "b"]);
  });

  it("skips children of collapsed nodes", () => {
    const tree: BlockNode[] = [
      block("a", {
        collapsed: true,
        children: [block("a1"), block("a2")],
      }),
      block("b"),
    ];
    expect(flattenVisible(tree)).toEqual(["a", "b"]);
  });

  it("returns [] for an empty outline", () => {
    expect(flattenVisible([])).toEqual([]);
  });

  it("recurses through deeply nested visible subtrees", () => {
    const tree: BlockNode[] = [
      block("a", {
        children: [
          block("a1", {
            children: [block("a1a"), block("a1b")],
          }),
        ],
      }),
    ];
    expect(flattenVisible(tree)).toEqual(["a", "a1", "a1a", "a1b"]);
  });
});

describe("nextVisibleId", () => {
  const tree: BlockNode[] = [
    block("a"),
    block("b", { collapsed: true, children: [block("b1")] }),
    block("c"),
  ];

  it("returns the first id when current is null", () => {
    expect(nextVisibleId(null, tree)).toBe("a");
  });

  it("returns the first id when current is unknown to the outline", () => {
    expect(nextVisibleId("nonexistent", tree)).toBe("a");
  });

  it("steps over collapsed subtrees", () => {
    expect(nextVisibleId("b", tree)).toBe("c");
  });

  it("clamps at the bottom (no wrap)", () => {
    expect(nextVisibleId("c", tree)).toBe("c");
  });

  it("returns null on an empty outline", () => {
    expect(nextVisibleId("anything", [])).toBeNull();
    expect(nextVisibleId(null, [])).toBeNull();
  });
});

describe("previousVisibleId", () => {
  const tree: BlockNode[] = [
    block("a"),
    block("b", { collapsed: true, children: [block("b1")] }),
    block("c"),
  ];

  it("returns null at the top — never the current block (no wrap)", () => {
    // Must be null, not "a": returning the current (top) block left the
    // cursor on the very block a caller was about to delete, and the new
    // block then landed under the trash root (`o`-after-delete-all crash).
    expect(previousVisibleId("a", tree)).toBeNull();
  });

  it("skips children of the collapsed parent on the way up", () => {
    expect(previousVisibleId("c", tree)).toBe("b");
  });

  it("returns first visible when current is unknown", () => {
    expect(previousVisibleId("ghost", tree)).toBe("a");
  });

  it("returns null on empty outline", () => {
    expect(previousVisibleId(null, [])).toBeNull();
  });
});

describe("visualRangeIds / isInVisualRange", () => {
  const tree: BlockNode[] = [block("a"), block("b"), block("c"), block("d")];

  it("orders anchor + cursor regardless of direction", () => {
    expect(visualRangeIds("b", "d", tree)).toEqual({ lo: "b", hi: "d" });
    expect(visualRangeIds("d", "b", tree)).toEqual({ lo: "b", hi: "d" });
  });

  it("returns null when either endpoint is missing or invisible", () => {
    expect(visualRangeIds(null, "a", tree)).toBeNull();
    expect(visualRangeIds("a", null, tree)).toBeNull();
    expect(visualRangeIds("ghost", "a", tree)).toBeNull();
  });

  it("highlights every block in [lo, hi]", () => {
    expect(isInVisualRange("a", "b", "d", tree)).toBe(false);
    expect(isInVisualRange("b", "b", "d", tree)).toBe(true);
    expect(isInVisualRange("c", "b", "d", tree)).toBe(true);
    expect(isInVisualRange("d", "b", "d", tree)).toBe(true);
  });

  it("single-block range still includes the anchor", () => {
    expect(isInVisualRange("b", "b", "b", tree)).toBe(true);
    expect(isInVisualRange("a", "b", "b", tree)).toBe(false);
  });

  it("returns false when range is invalid", () => {
    expect(isInVisualRange("a", null, "b", tree)).toBe(false);
    expect(isInVisualRange("a", "ghost", "b", tree)).toBe(false);
  });
});

describe("flattenAll", () => {
  it("includes children of collapsed nodes (unlike flattenVisible)", () => {
    // The whole reason flattenAll exists: zR / cursor-pruning must see
    // blocks hidden under a folded parent, which flattenVisible skips.
    const tree: BlockNode[] = [
      block("a", { collapsed: true, children: [block("a1"), block("a2")] }),
      block("b"),
    ];
    expect(flattenVisible(tree)).toEqual(["a", "b"]);
    expect(flattenAll(tree)).toEqual(["a", "a1", "a2", "b"]);
  });

  it("is empty for an empty outline", () => {
    expect(flattenAll([])).toEqual([]);
  });

  it("walks the same DFS order as flattenNodes, ids instead of nodes", () => {
    const tree: BlockNode[] = [
      block("a", { children: [block("a1")] }),
      block("b"),
    ];
    expect(flattenAll(tree)).toEqual(flattenNodes(tree).map((b) => b.id));
  });
});

describe("flattenParents", () => {
  it("includes only nodes with children, skipping leaves", () => {
    // zM (fold-all) targets parents only — folding a leaf writes a
    // SetCollapsed op that would make future children appear collapsed.
    const tree: BlockNode[] = [
      block("a", { children: [block("a1", { children: [block("a11")] })] }),
      block("b"),
    ];
    // a and a1 are parents; a11 and b are leaves.
    expect(flattenParents(tree)).toEqual(["a", "a1"]);
  });

  it("descends into collapsed parents too", () => {
    const tree: BlockNode[] = [
      block("a", {
        collapsed: true,
        children: [block("a1", { children: [block("a11")] })],
      }),
    ];
    expect(flattenParents(tree)).toEqual(["a", "a1"]);
  });
});

describe("visualRangeSet", () => {
  const tree: BlockNode[] = [block("a"), block("b"), block("c"), block("d")];

  it("builds the inclusive set of ids between anchor and cursor", () => {
    expect(visualRangeSet("b", "d", tree)).toEqual(new Set(["b", "c", "d"]));
  });

  it("orders anchor + cursor regardless of direction", () => {
    expect(visualRangeSet("d", "b", tree)).toEqual(new Set(["b", "c", "d"]));
  });

  it("is null when either endpoint is unset or off-outline", () => {
    expect(visualRangeSet(null, "b", tree)).toBeNull();
    expect(visualRangeSet("b", null, tree)).toBeNull();
    expect(visualRangeSet("b", "ghost", tree)).toBeNull();
  });
});

describe("focusSubtree", () => {
  // a
  //   a1
  //     a1x
  //   a2
  // b
  const tree = [
    block("a", {
      children: [
        block("a1", { children: [block("a1x")] }),
        block("a2"),
      ],
    }),
    block("b"),
  ];

  it("returns the subtree and breadcrumb for a nested block", () => {
    const fv = focusSubtree(tree, "a1");
    expect(fv).not.toBeNull();
    expect(fv?.root.id).toBe("a1");
    // subtree carries its own children
    expect(fv?.root.children.map((c) => c.id)).toEqual(["a1x"]);
    // breadcrumb is page-top first, immediate parent last
    expect(fv?.breadcrumb.map((c) => c.id)).toEqual(["a"]);
  });

  it("gives an empty breadcrumb for a top-level block", () => {
    const fv = focusSubtree(tree, "b");
    expect(fv?.root.id).toBe("b");
    expect(fv?.breadcrumb).toEqual([]);
  });

  it("builds a top-down breadcrumb down a deep chain", () => {
    const fv = focusSubtree(tree, "a1x");
    expect(fv?.root.id).toBe("a1x");
    expect(fv?.breadcrumb.map((c) => c.id)).toEqual(["a", "a1"]);
  });

  it("returns null for an unknown id (stale zoom target)", () => {
    expect(focusSubtree(tree, "ghost")).toBeNull();
  });

  it("returns null for an empty outline", () => {
    expect(focusSubtree([], "a")).toBeNull();
  });
});

describe("sameCrumbTrail", () => {
  const crumb = (id: string) => ({ id, text: id });

  it("two empty trails are the same (both root-level)", () => {
    expect(sameCrumbTrail([], [])).toBe(true);
  });

  it("matches identical trails by id", () => {
    expect(sameCrumbTrail([crumb("a"), crumb("b")], [crumb("a"), crumb("b")])).toBe(true);
  });

  it("ignores text, compares by id", () => {
    const a = [{ id: "a", text: "Old" }];
    const b = [{ id: "a", text: "New" }];
    expect(sameCrumbTrail(a, b)).toBe(true);
  });

  it("different length is never the same", () => {
    expect(sameCrumbTrail([crumb("a")], [crumb("a"), crumb("b")])).toBe(false);
  });

  it("a shared prefix is not enough — the whole trail must match", () => {
    expect(sameCrumbTrail([crumb("a"), crumb("b")], [crumb("a"), crumb("c")])).toBe(false);
  });
});

describe("embedOnlyHandle", () => {
  it("returns the handle for a lone embed token", () => {
    const tokens: InlineToken[] = [{ kind: "embed", value: "blk-r6s4a1" }];
    expect(embedOnlyHandle(tokens)).toBe("blk-r6s4a1");
  });

  it("returns the handle ignoring surrounding whitespace-only plain tokens", () => {
    const tokens: InlineToken[] = [
      { kind: "plain", value: "  " },
      { kind: "embed", value: "blk-r6s4a1" },
      { kind: "plain", value: " " },
    ];
    expect(embedOnlyHandle(tokens)).toBe("blk-r6s4a1");
  });

  it("returns null when the embed is mixed with prose", () => {
    const tokens: InlineToken[] = [
      { kind: "plain", value: "see " },
      { kind: "embed", value: "blk-r6s4a1" },
      { kind: "plain", value: " context" },
    ];
    expect(embedOnlyHandle(tokens)).toBeNull();
  });

  it("returns null for a bare inline block-ref (no `!` embed)", () => {
    const tokens: InlineToken[] = [{ kind: "blockref", value: "blk-r6s4a1" }];
    expect(embedOnlyHandle(tokens)).toBeNull();
  });

  it("returns null when two embeds share one block", () => {
    const tokens: InlineToken[] = [
      { kind: "embed", value: "blk-aaaaaa" },
      { kind: "plain", value: " " },
      { kind: "embed", value: "blk-bbbbbb" },
    ];
    expect(embedOnlyHandle(tokens)).toBeNull();
  });
});

describe("collectBlockRefHandles", () => {
  function node(id: string, tokens: InlineToken[], children: BlockNode[] = []): BlockNode {
    return { id, text: "", todo: null, tokens, collapsed: false, properties: [], children };
  }

  it("collects both blockref and embed handles", () => {
    const outline = [
      node("a", [{ kind: "blockref", value: "blk-aaaaaa" }]),
      node("b", [{ kind: "embed", value: "blk-bbbbbb" }]),
    ];
    expect(collectBlockRefHandles(outline)).toEqual(["blk-aaaaaa", "blk-bbbbbb"]);
  });

  it("de-duplicates by first appearance, preserving order", () => {
    const outline = [
      node("a", [{ kind: "blockref", value: "blk-aaaaaa" }]),
      node("b", [{ kind: "embed", value: "blk-aaaaaa" }]),
      node("c", [{ kind: "blockref", value: "blk-cccccc" }]),
    ];
    expect(collectBlockRefHandles(outline)).toEqual(["blk-aaaaaa", "blk-cccccc"]);
  });

  it("recurses into children", () => {
    const outline = [
      node("a", [{ kind: "blockref", value: "blk-aaaaaa" }], [
        node("b", [{ kind: "embed", value: "blk-bbbbbb" }]),
      ]),
    ];
    expect(collectBlockRefHandles(outline)).toEqual(["blk-aaaaaa", "blk-bbbbbb"]);
  });

  it("descends the inner span of bold tokens", () => {
    const outline = [
      node("a", [
        { kind: "bold", inner: [{ kind: "blockref", value: "blk-inbold" }] },
      ]),
    ];
    expect(collectBlockRefHandles(outline)).toEqual(["blk-inbold"]);
  });

  it("returns an empty array when no handles are present", () => {
    const outline = [node("a", [{ kind: "plain", value: "just text" }])];
    expect(collectBlockRefHandles(outline)).toEqual([]);
  });
});

describe("cycleTodo", () => {
  it("walks none to TODO to DOING to DONE and back", () => {
    // Must match `outl_actions::todo::cycle_todo` stop for stop —
    // this function exists so a GUI draft doesn't wait on the
    // backend, and a different cycle here shows the user a state the
    // op log never gets.
    const s0 = "deploy frontend";
    const s1 = cycleTodo(s0);
    const s2 = cycleTodo(s1);
    const s3 = cycleTodo(s2);
    const s4 = cycleTodo(s3);
    expect(s1).toBe("TODO deploy frontend");
    expect(s2).toBe("DOING deploy frontend");
    expect(s3).toBe("DONE deploy frontend");
    expect(s4).toBe("deploy frontend");
  });

  it("keeps a quote marker in canonical order", () => {
    // Same order `outl_actions::cycle_todo` emits: state, then quote.
    expect(cycleTodo("> deploy")).toBe("TODO > deploy");
    expect(cycleTodo("TODO > deploy")).toBe("DOING > deploy");
    expect(cycleTodo("DOING > deploy")).toBe("DONE > deploy");
    expect(cycleTodo("DONE > deploy")).toBe("> deploy");
  });

  it("normalises a legacy TODO written after the quote", () => {
    // Naive concatenation would yield "TODO > TODO foo", which
    // `split_todo` then misreads.
    expect(cycleTodo("> TODO foo")).toBe("DOING > foo");
    expect(cycleTodo("> DOING foo")).toBe("DONE > foo");
    expect(cycleTodo("> DONE foo")).toBe("> foo");
  });

  it("does not treat a word starting with DOING as a marker", () => {
    expect(cycleTodo("DOINGs pile up")).toBe("TODO DOINGs pile up");
  });

  it("cycles a checkbox block onto the canonical word form", () => {
    // Issue #230: `- [ ] foo` is a task. Treating it as unmarked
    // would emit "TODO [ ] foo" — two markers, and the backend's
    // split_todo reads the wrong one.
    expect(cycleTodo("[ ] buy milk")).toBe("DOING buy milk");
    expect(cycleTodo("[/] buy milk")).toBe("DONE buy milk");
    expect(cycleTodo("[x] buy milk")).toBe("buy milk");
    expect(cycleTodo("[X] buy milk")).toBe("buy milk");
  });

  it("leaves a markdown link alone", () => {
    // `[x](url)` is a link whose anchor is "x". The trailing space in
    // the checkbox prefix is the only thing separating them.
    expect(cycleTodo("[x](https://example.com)")).toBe(
      "TODO [x](https://example.com)",
    );
  });

  it("handles an empty block", () => {
    expect(cycleTodo("")).toBe("TODO ");
  });

  it("does not treat a word starting with TODO as a marker", () => {
    // The marker is `TODO ` with its trailing space; `TODOs` is prose.
    expect(cycleTodo("TODOs are piling up")).toBe("TODO TODOs are piling up");
  });
});
