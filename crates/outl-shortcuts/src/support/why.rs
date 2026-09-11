//! The sentence a client shows when it cannot deliver an
//! [`Action`](crate::Action).
//!
//! Invariant 12: the reason text belongs to the catalog, never to
//! the client — a client that writes its own wording is a second
//! copy of the fact. Split out of `support.rs` only because that
//! file is at its size ceiling; this is still the same owner.
//!
//! A reason repeated across rows is named once here, so a re-wording
//! cannot land on some rows and miss others.

/// The desktop has no character cursor inside a block, so every
/// vim op that addresses a column has nowhere to land.
/// See RFC 0070 and `outl-desktop/CLAUDE.md` → "Vim parity".
pub const NO_CHAR_CURSOR: &str =
    "This vim op needs a character cursor inside the block, which only the TUI has. \
     Edit the block and use the arrow keys, or run it from the TUI.";

/// Mobile drives the outline by touch; there is no selection to
/// move and no modal Normal state to move it in.
pub const TOUCH_ONLY: &str =
    "Not on mobile — tap the block you want instead of moving a selection.";

/// Mobile has no keyboard chord surface for vim-modal state.
pub const NO_VIM_MODE: &str = "Mobile has no vim modes — it edits directly on tap.";

/// The TUI renders `**bold**` / `_italic_` / `` `code` `` /
/// `~~strike~~` (`outl-tui/src/view/inline.rs`) and has no chord
/// that *writes* the delimiters — `input/insert.rs` binds no
/// wrap key, and nothing under `outl-tui/src` mentions any of
/// the four wrap actions. The catalog said `Full` here on all
/// three clients from the day the rows landed.
pub const NO_TUI_INLINE_WRAP: &str =
    "The TUI has no chord for inline markdown — edit the block and type the markers \
     around the text yourself: `**bold**`, `_italic_`, `` `code` ``, `~~strike~~`.";

/// Same gap; a link needs its own repair text (two delimiters
/// with the caret between them, not one on each side).
pub const NO_TUI_LINK: &str =
    "The TUI has no link chord — edit the block and type `[label](url)`, or `[[page]]` \
     to link another page in the workspace.";

/// `ToolbarAction` in `@outl/shared` is the whole mobile button
/// set and carries `bold` / `italic` / `code` and nothing else
/// from this family; there is no second wrapping surface.
pub const NO_MOBILE_STRIKE: &str =
    "Strikethrough isn't on the mobile toolbar — type `~~` on each side of the text \
     instead (the bold, italic and code buttons are there).";

/// Same list, same absence. The two bracket buttons mobile does
/// ship (`[[`, `((`) insert refs, not a markdown link.
pub const NO_MOBILE_LINK: &str =
    "There's no link button on the mobile toolbar — type `[label](url)` by hand, or use \
     the `[[` button to link another page in the workspace.";
