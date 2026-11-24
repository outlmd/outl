//! The panel's lines, memoised.
//!
//! `render_app` runs on every keystroke and the panel used to be rebuilt
//! each time: on a hub page that is ~850 citing rows and ~3.5k subtree
//! nodes, each one tokenised and wrapped again, while the user typed in
//! the outline above it (root invariant 15; the incident is in
//! `docs/performance.md`, "A citing block's children").
//!
//! **What the lines read, and therefore what the key holds.**
//!
//! - the rows and each row's subtree: a hash over exactly the fields the
//!   renderer reads, plus the subtree's `Arc` address. The cache holds
//!   those `Arc`s, so an address cannot be reused while it is compared
//!   (no ABA);
//! - the width and the sort direction (the header's arrow);
//! - what is focused and, **only for a row being edited in the panel**,
//!   the live buffer. With the cursor in the outline the focus part is
//!   `None`, so typing above the panel never misses;
//! - [`PanelMemo::epoch`], for the two inputs with no cheap fingerprint:
//!   `App::index` (ref and embed chips resolve through it) and the
//!   theme. Their writers call [`App::bump_render_epoch`] (page patches go
//!   through [`App::patch_index_page`]).
//!
//! **What it does not do.** A key change rebuilds the whole panel, so
//! `j`/`k` through a hub's rows and typing *inside* a row are as costly
//! per keystroke as they were. The cheap case is the common one (idle,
//! and typing in the outline), and that is what is bounded.

use std::cell::{Cell, RefCell};
use std::hash::{Hash, Hasher};
use std::rc::Rc;
use std::sync::Arc;

use outl_actions::{Backlink, OutlineNode};
use outl_core::id::NodeId;
use ratatui::text::Line;

use crate::state::{App, EditTarget, Focus, Mode};

/// The panel as one frame reads it: shared lines and the focused row.
#[derive(Clone)]
pub(crate) struct Panel {
    pub(crate) lines: Rc<Vec<Line<'static>>>,
    pub(crate) selected: Option<usize>,
}

impl Panel {
    pub(super) fn empty() -> Self {
        Self::from_built((Vec::new(), None))
    }

    pub(super) fn from_built((lines, selected): (Vec<Line<'static>>, Option<usize>)) -> Self {
        Self {
            lines: Rc::new(lines),
            selected,
        }
    }
}

/// What the focused row's rendering reads beyond the row itself.
#[derive(Debug, Clone, PartialEq, Eq)]
enum FocusKey {
    /// The cursor is in the outline: no panel row reads the mode.
    Outline,
    /// The cursor is on a panel row.
    Row {
        idx: usize,
        node: Option<NodeId>,
        mode: ModeKey,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ModeKey {
    /// Block cursor on the focused block: reads `cursor_col`.
    Normal(usize),
    /// In-place edit buffer of one block.
    Edit(NodeId, String, usize),
    /// Any other mode renders the focused row as plain pretty text.
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Key {
    epoch: u64,
    width: u16,
    newest_first: bool,
    rows: u64,
    row_count: usize,
    focus: FocusKey,
}

struct Cached {
    key: Key,
    panel: Panel,
    /// Held so the subtree addresses hashed into `key.rows` stay unique.
    _subtrees: Vec<Arc<[OutlineNode]>>,
}

/// The memo and its two counters. One field on `App`.
#[derive(Default)]
pub(crate) struct PanelMemo {
    epoch: Cell<u64>,
    builds: Cell<u64>,
    cached: RefCell<Option<Cached>>,
}

impl PanelMemo {
    /// How many times the panel was built from scratch. A test seam:
    /// the structural net is "a redraw that changed nothing the panel
    /// reads builds nothing".
    #[cfg(test)]
    pub(crate) fn builds(&self) -> u64 {
        self.builds.get()
    }
}

impl App {
    /// `App::index.patch_page`, plus the epoch bump the panel's memo
    /// needs: the one way a page patch reaches the index.
    pub(crate) fn patch_index_page(
        &mut self,
        path: &std::path::Path,
        page: &outl_md::parse::ParsedPage,
    ) {
        self.index.patch_page(path, page);
        self.bump_render_epoch();
    }

    /// Mark `app.index` or `app.theme` as changed, so the panel's
    /// memoised lines are not served over a stale ref chip or palette.
    /// Every writer of those two calls this; a new one has to as well.
    pub(crate) fn bump_render_epoch(&self) {
        let memo = &self.panel_memo;
        memo.epoch.set(memo.epoch.get().wrapping_add(1));
    }
}

/// Fold the fields the renderer reads from one row into `h`.
fn hash_row(bl: &Backlink, subtree: Option<&Arc<[OutlineNode]>>, h: &mut impl Hasher) {
    bl.block_id.hash(h);
    if let Some(p) = &bl.source_page {
        (&p.id, &p.slug, &p.title, &p.icon).hash(h);
    }
    for c in &bl.ancestors {
        (&c.id, &c.text).hash(h);
    }
    let n = &bl.source_block;
    (&n.id, &n.text, n.todo.map(|t| t.as_str()), &n.properties).hash(h);
    subtree
        .map(|a| Arc::as_ptr(a).cast::<u8>() as usize)
        .hash(h);
}

fn focus_key(app: &App) -> FocusKey {
    let Focus::Backlink { idx, node } = &app.focus else {
        return FocusKey::Outline;
    };
    let mode = match &app.mode {
        Mode::Normal => ModeKey::Normal(app.cursor_col),
        Mode::Insert {
            target: EditTarget::SourcePage { block, .. },
            buffer,
            ..
        } => ModeKey::Edit(*block, buffer.as_string(), buffer.cursor),
        _ => ModeKey::Other,
    };
    FocusKey::Row {
        idx: *idx,
        node: *node,
        mode,
    }
}

/// Serve the memoised panel when its key still matches, else `build`.
pub(super) fn cached_or_build(
    app: &App,
    width: u16,
    backlinks: &[Backlink],
    build: impl FnOnce() -> (Vec<Line<'static>>, Option<usize>),
) -> Panel {
    let memo = &app.panel_memo;
    let subtrees: Vec<Option<Arc<[OutlineNode]>>> = backlinks
        .iter()
        .map(|bl| app.backlink_subtree(&bl.block_id))
        .collect();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for (bl, sub) in backlinks.iter().zip(&subtrees) {
        hash_row(bl, sub.as_ref(), &mut h);
    }
    let key = Key {
        epoch: memo.epoch.get(),
        width,
        newest_first: app.backlinks_newest_first,
        rows: h.finish(),
        row_count: backlinks.len(),
        focus: focus_key(app),
    };
    if let Some(hit) = memo.cached.borrow().as_ref().filter(|c| c.key == key) {
        return hit.panel.clone();
    }
    memo.builds.set(memo.builds.get() + 1);
    let panel = Panel::from_built(build());
    *memo.cached.borrow_mut() = Some(Cached {
        key,
        panel: panel.clone(),
        _subtrees: subtrees.into_iter().flatten().collect(),
    });
    panel
}
