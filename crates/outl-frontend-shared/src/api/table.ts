/**
 * The wire shapes for a markdown table, split out of `types.ts`.
 *
 * Same rule as every other DTO in `api/`: each shape mirrors a
 * `serde`-serialized Rust type, the mirror is hand-written on purpose,
 * and `outl-tauri-shared/tests/wire_types.rs` + `wire_enums.rs` are
 * what make that safe — this file is registered in the test's
 * `MIRROR_FILES`, so a field or a variant added in Rust and forgotten
 * here fails there rather than reaching a device as `undefined`.
 *
 * Its own file because the two shapes are one concept and `types.ts`
 * was past the file-size ratchet; `api/plugins.ts` is the same split.
 */

import type { InlineToken } from "./tokens";

/**
 * Per-column alignment, from the delimiter row's colons (`:---` /
 * `:---:` / `---:`). `"none"` is `---`: every client renders it
 * left-aligned, and the distinct value is what lets the on-disk
 * delimiter round-trip. Mirrors `outl_md::ColumnAlign`.
 */
export type ColumnAlign = "none" | "left" | "center" | "right";

/**
 * A markdown table with every cell pre-tokenized — same bargain as
 * `BlockNode.tokens`, one construct up: the backend runs the one
 * canonical tokenizer, so no client splits pipes for itself and renders
 * a cell's `[[ref]]` as literal text.
 *
 * **Rectangular** — `aligns`, `header` and every row carry one entry
 * per column, padded by the backend, so a renderer emits `<tr>`s
 * without counting. Mirrors `outl_md::TableView`.
 */
export interface TableView {
  aligns: ColumnAlign[];
  header: InlineToken[][];
  rows: InlineToken[][][];
}
