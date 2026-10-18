/**
 * Which view a fenced block gets, and how a diagram link or failure
 * is read. Both GUI clients call this so the priority cannot drift:
 * a plugin transformer for the language, then the built-in diagram
 * for a mermaid fence, then highlighted source.
 */

export type DiagramsMode = "pele" | "off";

export type FenceChoice = "plugin" | "diagram" | "code";

/** Pele's own default source cap. A longer fence never reaches the renderer. */
export const DIAGRAM_SOURCE_LIMIT = 50_000;

export function coerceDiagramsMode(value: unknown): DiagramsMode {
  return value === "off" ? "off" : "pele";
}

export function isMermaidFence(language: string): boolean {
  return language.trim().toLowerCase() === "mermaid";
}

export function chooseFence(input: {
  hasTransformer: boolean;
  language: string;
  diagrams: DiagramsMode;
}): FenceChoice {
  if (input.hasTransformer) return "plugin";
  if (input.diagrams === "pele" && isMermaidFence(input.language)) return "diagram";
  return "code";
}

export function overDiagramLimit(source: string): boolean {
  return source.length > DIAGRAM_SOURCE_LIMIT;
}

/**
 * User-facing line. Callers assign this with `textContent`, never `innerHTML`.
 *
 * Jison's dump is a source excerpt, a caret line and "Parse error on
 * line N". The line and column are already in the prefix, so the
 * caret and that sentence are dropped and the rest is one line.
 */
export function formatDiagramError(line: number, column: number, message: string): string {
  return `line ${line}, col ${column}: ${diagramErrorDetail(message)}`;
}

function diagramErrorDetail(message: string): string {
  const lines = message
    .split(/\r?\n/)
    .map((part) => part.trim())
    .filter((part) => part.length > 0)
    .filter((part) => !isCaretLine(part))
    .filter((part) => !/^parse error on line \d+:?$/i.test(part));
  const joined = (lines.length > 0 ? lines : [message]).join(" ");
  return joined.replace(/^parse error on line \d+:\s*/i, "").replace(/\s+/g, " ").trim();
}

function isCaretLine(line: string): boolean {
  return /^[-^|.\s]+$/.test(line);
}

/**
 * `limit` and `unsupported-diagram` have no position worth showing.
 * The fence goes back to highlighted source. A syntax or semantic
 * failure keeps the source and adds the line/column message.
 */
export function diagramFailureView(code: string): "code" | "error" {
  if (code === "limit" || code === "unsupported-diagram") return "code";
  return "error";
}

export type LinkDisposition = "page" | "external" | "drop";

const EXTERNAL_SCHEMES = new Set(["http", "https", "mailto"]);

/**
 * An internal link's `href` is a note name, not a URL.
 * Check that flag before parsing, or a title with a space is dropped.
 */
export function linkDisposition(link: { href: string; internal: boolean }): LinkDisposition {
  if (link.internal) return "page";
  let url: URL;
  try {
    url = new URL(link.href);
  } catch {
    return "drop";
  }
  const scheme = url.protocol.slice(0, -1);
  return EXTERNAL_SCHEMES.has(scheme) ? "external" : "drop";
}
