//! Block-level mutations: Insert mode, create / indent / outdent /
//! delete / reorder blocks, TODO prefix cycle.
//!
//! All ops snapshot through [`crate::state::App::snapshot_for_undo`]
//! so the history stack can roll back any structural change. Saves
//! go through `App::save` in `lifecycle::persistence`.
//!
//! ## Module layout
//!
//! | Submodule       | What's in it                                                |
//! |-----------------|-------------------------------------------------------------|
//! | `insert`        | `enter_insert`, `commit_insert`, `abort_insert`             |
//! | `structural`    | create / indent / outdent / delete / move block             |
//! | `backlink_edit` | `apply_to_backlink_source`, `toggle_todo_backlink`          |
//! | `metadata`      | property writes, `toggle_pinned`, `toggle_todo`             |
//! | `mod.rs` (here) | TODO-prefix cycle helpers shared with `input::insert`       |

use crate::edit_buffer::EditBuffer;

mod backlink_edit;
pub(crate) mod insert;
mod metadata;
mod structural;
mod template;

pub(crate) use insert::InsertCursor;

/// Cycle a block's task prefix: none → `TODO ` → `DOING ` → `DONE `
/// → none.
///
/// Delegates to [`outl_actions::cycle_todo`] so the TUI and the
/// mobile client share the exact same rule for cycling state.
pub(crate) fn cycle_todo_state(text: &str) -> String {
    outl_actions::cycle_todo(text)
}

/// Cycle the task prefix directly on an [`EditBuffer`], preserving the
/// cursor's *visual* position relative to the user's text.
///
/// The cursor moves by the **difference between the two prefixes**,
/// never by a constant: `DOING ` is six characters and its neighbours
/// are five, so `TODO ` → `DOING ` shifts the caret right by one and
/// `DOING ` → `DONE ` shifts it back left by one. The earlier version
/// assumed every marker was five characters wide and would have left
/// the caret one column inside the word.
///
/// The whole cycle decision (including quote normalisation) is
/// [`outl_actions::cycle_todo`]'s — this only re-derives where the
/// caret should land.
pub(crate) fn cycle_todo_inline(buffer: &mut EditBuffer) {
    let current: String = buffer.chars.iter().collect();
    let next = outl_actions::cycle_todo(&current);

    // The shift is the change in total length, not the change in the
    // task prefix: `cycle_todo` also normalises a legacy `"> TODO x"`
    // into `"TODO > x"`, which moves text the prefix measurement
    // cannot see. Measuring the prefix alone read `"> DOING foo"` as
    // unmarked (the quote comes first), so the caret jumped to the end
    // of the line instead of following its character.
    //
    // Total length works for every case because the body is a suffix
    // of both strings — everything `cycle_todo` rewrites lives in
    // front of it.
    let before = current.chars().count();
    let after = next.chars().count();

    buffer.chars = next.chars().collect();
    if after >= before {
        buffer.cursor += after - before;
    } else {
        buffer.cursor = buffer.cursor.saturating_sub(before - after);
    }
    buffer.cursor = buffer.cursor.min(buffer.chars.len());
}
