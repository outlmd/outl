//! The page action waiting for a `y`, and what that `y` performs.
//!
//! Three gestures share one `y/n` prompt on the status line: the page
//! delete (sidebar `d`, the `g d` chord), `/merge <page>`, and the name
//! collision `/rename <new name>` can run into. They share it because
//! the intercept that owns the keystream while a prompt is up lives in
//! one place (`input::normal`), and three pending slots on `App` would
//! be three ways to get that intercept's priority wrong.
//!
//! It lives in its own module rather than in `state.rs` because that
//! file is at its size ceiling — the same reason the merge was made to
//! fit inside the delete's struct instead of growing `App`. Widening
//! that `merge_into: Option<String>` again is what this type replaces:
//! a rename collision carries the name the user typed, which an
//! `Option<String>` meaning "target" cannot say.
//!
//! The slug and the title are resolved **at arm time**, so a page list
//! that changes between the prompt and the `y` (a peer's delete, a
//! rename on another device) cannot make the confirmation act on a
//! different page than the one the prompt named.

/// A page action awaiting `y` confirmation.
#[derive(Debug, Clone)]
pub(crate) struct PendingPageAction {
    /// Slug of the page the prompt is about.
    pub(crate) slug: String,
    /// Its display title when the prompt was armed, for the message.
    pub(crate) title: String,
    /// What `y` runs.
    pub(crate) action: PageAction,
}

/// What a confirmed [`PendingPageAction`] does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PageAction {
    /// `outl_actions::delete_page` on the pending slug.
    Delete,
    /// `outl_actions::merge_pages(slug, target)` — `/merge <page>`.
    MergeInto {
        /// Slug of the page the open one folds into.
        target: String,
    },
    /// `/rename` was refused with `ActionError::PageExists`, and the
    /// user is being offered the merge instead.
    ///
    /// Resolved by `outl_actions::rename_or_merge` with
    /// `OnExisting::MergeInto` rather than by `merge_pages` directly:
    /// the page that owned the name can be gone by the time the `y`
    /// arrives (a peer's delete, another device's rename), and then the
    /// rename the user actually asked for is what should happen.
    RenameCollision {
        /// Slug `new_title` resolved to, already validated.
        new_slug: String,
        /// The name the user typed, trimmed.
        new_title: String,
    },
}
