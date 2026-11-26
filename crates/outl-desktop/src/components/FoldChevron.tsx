/**
 * The fold chevron: `▼` over an open parent, `▶` plus the child count
 * over a folded one, and an empty, disabled slot on a leaf so the bullet
 * column stays put.
 *
 * One component for the outline's `<BlockRow />` and the backlink row,
 * so the glyph, the count and the hit area cannot drift between the two
 * places a block is drawn. **What a click does is the caller's**: the
 * outline folds on the open page, a backlink row folds on the block's
 * own page. `class` carries placement and how the chevron is revealed,
 * which is the one part each host draws differently.
 */
export function FoldChevron(props: {
  childCount: number;
  collapsed: boolean;
  /** `false` keeps the slot but takes no click: a leaf, or a backlink
   *  row with no page to write the fold to. */
  enabled?: boolean;
  /** Called with the state the click asks for, never a toggle, so a
   *  host can send an explicit expand / collapse. */
  onToggle: (collapse: boolean) => void;
  class?: string;
}) {
  const live = () => props.childCount > 0 && (props.enabled ?? true);
  return (
    <button
      type="button"
      data-fold-chevron
      class={`select-none whitespace-nowrap text-left text-[9px] font-mono min-w-[16px] ${
        live() ? "cursor-pointer" : ""
      } disabled:cursor-default ${props.class ?? ""}`}
      disabled={!live()}
      onClick={(e) => {
        // The row behind it selects or starts an edit on click.
        e.stopPropagation();
        if (!live()) return;
        props.onToggle(!props.collapsed);
      }}
      aria-label={props.collapsed ? "Expand" : "Collapse"}
    >
      {props.childCount > 0 ? (
        props.collapsed ? (
          <>
            <span>▶</span>
            <span class="ml-1 text-(--color-outl-fg-dimmer)">
              {props.childCount}
            </span>
          </>
        ) : (
          "▼"
        )
      ) : (
        ""
      )}
    </button>
  );
}
