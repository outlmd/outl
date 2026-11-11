/**
 * Escaping text that lands in the SVG or the HTML around it.
 *
 * One function, because there were two: an attribute-safe one next to the
 * fill patterns and a text-only one next to the shape renderers, which left
 * `html.ts` importing its escaper from `render/shapes` and every call site
 * picking between them by eye. The attribute version is a superset — escaping
 * `"` inside text content is harmless — so the narrower one bought nothing.
 */

/** Escape a value for an XML/HTML attribute or text node. */
export function escape(value: string): string {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

/**
 * A value out of a scene file as a trimmed string, or `""` when it is not one.
 *
 * Every field here is typed from JSON somebody else wrote, so `strokeColor`
 * can arrive as `1`, `true` or `{}`. `(value || "").trim()` throws on all
 * three, and a throw in a renderer costs the whole descriptor — the fence
 * draws nothing and says nothing.
 */
export function asString(value: unknown): string {
  return typeof value === "string" ? value.trim() : "";
}
