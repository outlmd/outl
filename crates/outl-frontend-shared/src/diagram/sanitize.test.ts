import { describe, expect, it } from "vitest";

import { adoptDiagramSvg } from "./sanitize";

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
});
