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

const DROPPED = new Set(["script", "foreignobject"]);

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

/**
 * Re-apply every `style` attribute under `root` through the CSSOM.
 *
 * Pele's zoom controls and `.pele-error` call `setAttribute("style")`
 * after `sanitize` returns. The same assignment, done through
 * `cssText`, is what the content-security policy allows.
 */
let lifting = false;

export function liftStyleAttributes(root: Element): void {
  if (lifting) return;
  lifting = true;
  try {
    const elements: Element[] = [root];
    root.querySelectorAll("*").forEach((el) => elements.push(el));
    for (const el of elements) applyStyle(el);
  } finally {
    lifting = false;
  }
}

/**
 * Pele adds zoom controls after `sanitize` returns, and a resize can
 * add them again. Watch the host and send each new `style` attribute
 * through the CSSOM. `applyStyle` skips a value it already wrote, so
 * the observer does not loop.
 */
export function watchStyleAttributes(root: HTMLElement): () => void {
  liftStyleAttributes(root);
  const observer = new MutationObserver(() => liftStyleAttributes(root));
  observer.observe(root, {
    subtree: true,
    childList: true,
    attributes: true,
    attributeFilter: ["style"],
  });
  return () => observer.disconnect();
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
    if (name.toLowerCase().startsWith("on")) el.removeAttribute(name);
  }
  applyStyle(el);
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
