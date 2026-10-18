import { createSignal } from "solid-js";

import { coerceDiagramsMode, type DiagramsMode } from "./policy";

const [mode, setMode] = createSignal<DiagramsMode>("pele");

/** The one `editor.diagrams` value both fence renderers read. */
export function diagramsMode(): DiagramsMode {
  return mode();
}

/** Unknown values stay on the default. Only `"off"` turns diagrams off. */
export function setDiagramsMode(value: unknown): void {
  setMode(coerceDiagramsMode(value));
}
