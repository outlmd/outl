import { describe, expect, it } from "vitest";

import { adoptDiagramSvg } from "./sanitize";

const XLINK = "http://www.w3.org/1999/xlink";

function adopted(inner: string): Element {
  const node = adoptDiagramSvg(
    `<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="${XLINK}">${inner}</svg>`,
  );
  expect(node).toBeInstanceOf(Element);
  return node as Element;
}

function hrefOf(el: Element): string | null {
  return el.getAttribute("href") ?? el.getAttribute("xlink:href") ?? el.getAttributeNS(XLINK, "href");
}

describe("adoptDiagramSvg", () => {
  it("moves style attributes onto the CSSOM and drops active content", () => {
    const node = adoptDiagramSvg(
      [
        '<svg xmlns="http://www.w3.org/2000/svg" style="--_fg:var(--pele-fg,#1f1f1f)">',
        "<script>alert(1)</script>",
        "<foreignObject><p>x</p></foreignObject>",
        '<g onclick="alert(1)" style="stroke:black"></g>',
        "</svg>",
      ].join(""),
    );
    expect(node).toBeInstanceOf(Element);
    const svg = node as SVGElement;
    document.body.appendChild(svg);
    expect(svg.style.getPropertyValue("--_fg")).toContain("--pele-fg");
    expect(svg.querySelector("script, foreignObject")).toBeNull();
    expect(svg.querySelector("g")?.getAttribute("onclick")).toBeNull();
    expect(svg.querySelector("g")?.style.getPropertyValue("stroke")).toBe("black");
  });

  it("returns plain text when the svg does not parse", () => {
    const node = adoptDiagramSvg("<<<");
    expect(node.nodeType).toBe(Node.TEXT_NODE);
    expect(node.textContent).toBe("The diagram could not be drawn.");
  });

  it.each(["style", "animate", "set", "animateMotion", "animateTransform"])(
    "drops a <%s> element",
    (tag) => {
      const svg = adopted(`<${tag} attributeName="fill" to="red"></${tag}><rect />`);
      expect(svg.querySelector(tag)).toBeNull();
      expect(svg.querySelector("rect")).not.toBeNull();
    },
  );

  it.each([
    ["http://example.com/a", "http://example.com/a"],
    ["https://example.com/a", "https://example.com/a"],
    ["HTTPS://example.com/a", "HTTPS://example.com/a"],
    ["mailto:a@example.com", "mailto:a@example.com"],
    ["MAILTO:a@example.com", "MAILTO:a@example.com"],
    ["#node", "#node"],
    ["  #node  ", "  #node  "],
  ])("keeps href %j", (href) => {
    const svg = adopted(`<a href="${href}">x</a>`);
    expect(hrefOf(svg.querySelector("a")!)).toBe(href);
  });

  it.each([
    "javascript:alert(1)",
    "JAVASCRIPT:alert(1)",
    "java\nscript:alert(1)",
    "java\tscript:alert(1)",
    "java\u0000script:alert(1)",
    "  javascript:alert(1)",
    "data:text/html,hi",
    "tel:+15551212",
    "pages/journal",
    "./notes",
    "/notes",
    "//evil.example/x",
    "",
  ])("removes href %j", (href) => {
    const svg = adopted(`<a href="${href}">x</a>`);
    const anchor = svg.querySelector("a")!;
    expect(anchor).not.toBeNull();
    expect(hrefOf(anchor)).toBeNull();
  });

  it("removes an xlink:href whose scheme is not allowed and keeps one that is", () => {
    const svg = adopted(
      [
        '<a id="bad" xlink:href="javascript:alert(1)">x</a>',
        '<a id="good" xlink:href="https://example.com/a">y</a>',
      ].join(""),
    );
    expect(hrefOf(svg.querySelector("#bad")!)).toBeNull();
    expect(hrefOf(svg.querySelector("#good")!)).toBe("https://example.com/a");
  });
});
