import { describe, expect, it } from "vitest";

import {
  chooseFence,
  diagramFailureView,
  formatDiagramError,
  linkDisposition,
  overDiagramLimit,
} from "./policy";

describe("chooseFence", () => {
  it("lets a plugin transformer win over the built-in diagram", () => {
    expect(
      chooseFence({
        hasTransformer: true,
        language: "mermaid",
        diagrams: "pele",
      }),
    ).toBe("plugin");
  });

  it("picks the built-in diagram for a mermaid fence when diagrams are on", () => {
    expect(
      chooseFence({
        hasTransformer: false,
        language: "mermaid",
        diagrams: "pele",
      }),
    ).toBe("diagram");
  });

  it("treats the mermaid info string case-insensitively", () => {
    expect(
      chooseFence({
        hasTransformer: false,
        language: " Mermaid ",
        diagrams: "pele",
      }),
    ).toBe("diagram");
  });

  it("falls back to highlighted code when diagrams are off", () => {
    expect(
      chooseFence({
        hasTransformer: false,
        language: "mermaid",
        diagrams: "off",
      }),
    ).toBe("code");
  });

  it("falls back to highlighted code for a language Pele does not own", () => {
    expect(
      chooseFence({
        hasTransformer: false,
        language: "zenuml",
        diagrams: "pele",
      }),
    ).toBe("code");
  });

  it("falls back to highlighted code for an ordinary fence", () => {
    expect(
      chooseFence({
        hasTransformer: false,
        language: "rust",
        diagrams: "pele",
      }),
    ).toBe("code");
  });
});

describe("diagram errors", () => {
  it("formats a Pele failure as line and column", () => {
    expect(formatDiagramError(3, 1, "expected a node")).toBe(
      "line 3, col 1: expected a node",
    );
  });

  it("shows source plus an error line for a syntax failure", () => {
    expect(diagramFailureView("syntax")).toBe("error");
    expect(diagramFailureView("semantic")).toBe("error");
  });

  it("drops an unsupported diagram back to source with no error line", () => {
    expect(diagramFailureView("unsupported-diagram")).toBe("code");
  });

  it("drops a source over the limit back to source with no error line", () => {
    expect(diagramFailureView("limit")).toBe("code");
    expect(overDiagramLimit("x".repeat(50_000))).toBe(false);
    expect(overDiagramLimit("x".repeat(50_001))).toBe(true);
  });
});

describe("linkDisposition", () => {
  it("sends a link Pele marks internal to page navigation", () => {
    expect(linkDisposition({ href: "My Page", internal: true })).toBe("page");
  });

  it("keeps http, https and mailto as external links", () => {
    expect(linkDisposition({ href: "https://example.com", internal: false })).toBe(
      "external",
    );
    expect(linkDisposition({ href: "http://example.com/a", internal: false })).toBe(
      "external",
    );
    expect(linkDisposition({ href: "mailto:a@example.com", internal: false })).toBe(
      "external",
    );
  });

  it("drops javascript, tel and a bare note name", () => {
    expect(linkDisposition({ href: "javascript:alert(1)", internal: false })).toBe(
      "drop",
    );
    expect(linkDisposition({ href: "tel:+15551212", internal: false })).toBe("drop");
    expect(linkDisposition({ href: "My Page", internal: false })).toBe("drop");
  });
});
