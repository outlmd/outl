/**
 * Turn Pele's SVG string into a live node whose colours survive a
 * style-src nonce.
 *
 * Tauri hashes the inline boot style in the mobile shell and drops
 * `'unsafe-inline'`. A `style` attribute inserted with `innerHTML`
 * or `setAttribute` is then ignored, so the diagram loses its
 * strokes. Assigning the same text through `CSSStyleDeclaration`
 * is not gated by that directive.
 * https://pele.run/api (`sanitize`)
 * https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/Content-Security-Policy/style-src
 */

const XLINK = "http://www.w3.org/1999/xlink";

const DROPPED = new Set([
  "script",
  "foreignobject",
  "style",
  "animate",
  "set",
  "animatemotion",
  "animatetransform",
]);

const ALLOWED_SCHEMES = new Set(["http", "https", "mailto"]);

/** Parse `svg` and return a node safe to insert into the live document. */
export function adoptDiagramSvg(svg: string): Node {
  const parsed = new DOMParser().parseFromString(svg, "image/svg+xml");
  if (parsed.querySelector("parsererror")) {
    return document.createTextNode("The diagram could not be drawn.");
  }
  const imported = document.importNode(parsed.documentElement, true);
  if (!(imported instanceof Element)) {
    return document.createTextNode("The diagram could not be drawn.");
  }
  scrub(imported);
  return imported;
}

function scrub(el: Element): void {
  for (const child of [...el.children]) {
    if (DROPPED.has(child.localName.toLowerCase())) {
      child.remove();
      continue;
    }
    scrub(child);
  }
  for (const name of [...el.getAttributeNames()]) {
    const lower = name.toLowerCase();
    if (lower.startsWith("on")) el.removeAttribute(name);
    if (isHrefName(lower)) {
      const value = el.getAttribute(name);
      if (value === null || !hrefAllowed(value)) el.removeAttribute(name);
    }
  }
  const xlink = el.getAttributeNS(XLINK, "href");
  if (xlink !== null && !hrefAllowed(xlink)) el.removeAttributeNS(XLINK, "href");
  applyStyle(el);
}

function isHrefName(name: string): boolean {
  return name === "href" || name.endsWith(":href");
}

/**
 * `http`, `https`, `mailto`, or a `#fragment`.
 * The scheme is trimmed, lowercased, and stripped of whitespace and
 * control characters before that check, so `java\nscript:` does not
 * survive. Anything else, including a relative path, is removed.
 */
function hrefAllowed(value: string): boolean {
  const trimmed = value.trim();
  if (trimmed.startsWith("#")) return true;
  const colon = trimmed.indexOf(":");
  if (colon <= 0) return false;
  const scheme = trimmed
    .slice(0, colon)
    .toLowerCase()
    .replace(/[\s\u0000-\u001f\u007f-\u009f]/g, "");
  return ALLOWED_SCHEMES.has(scheme);
}

function applyStyle(el: Element): void {
  if (!(el instanceof HTMLElement) && !(el instanceof SVGElement)) return;
  const style = el.getAttribute("style");
  if (style === null || style === "") return;
  if (el.style.cssText === style) return;
  try {
    el.style.cssText = style;
  } catch {
    el.removeAttribute("style");
  }
}
