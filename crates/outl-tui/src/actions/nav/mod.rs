//! Navigation: between pages, between journals, between blocks, and
//! inside a block's text. Also `[[ref]]` / `#tag` / date-link
//! resolution.
//!
//! Almost nothing here persists — navigation mutates `self.view`,
//! `self.selected` and `self.cursor_col`, and the lifecycle module is
//! the one that touches disk. The two deliberate exceptions are
//! `open`, which seeds a `.md` when the user opens a page that does
//! not exist yet, and `backlinks`, which persists the list's sort
//! direction to the global config.
//!
//! ## Module layout
//!
//! | Submodule       | What's in it                                                   |
//! |-----------------|----------------------------------------------------------------|
//! | `mod.rs` (here) | Which view is open: its path, its title, its slug              |
//! | `open`          | Everything that *changes* the open view (journals, refs, pages) |
//! | `selection`     | Moving the selection block to block, across the backlink zone   |
//! | `cursor`        | Moving the caret inside the selected block's text               |
//! | `backlinks`     | The backlink index: accessors, background build, sort order     |
//! | `search`        | `*` / `#` — search the word under the cursor                    |

use crate::state::{App, Mode, View};
use std::path::PathBuf;

mod backlinks;
mod cursor;
mod open;
mod search;
mod selection;

impl App {
    pub(crate) fn current_path(&self) -> PathBuf {
        match &self.view {
            View::Journal(date) => self
                .workspace_root
                .join("journals")
                .join(format!("{}.md", date.format("%Y-%m-%d"))),
            View::Page(p) => p.clone(),
        }
    }

    #[allow(dead_code)] // header now uses chrome::breadcrumb; kept for future reuse
    pub(crate) fn current_title(&self) -> String {
        let mode_tag = match self.mode {
            Mode::Normal => "NORMAL",
            Mode::Insert { .. } => "INSERT",
            Mode::Visual { .. } => "VISUAL",
        };
        match &self.view {
            View::Journal(date) => {
                format!("Journal · {} · [{}]", date.format("%A, %Y-%m-%d"), mode_tag)
            }
            View::Page(p) => {
                let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("?");
                // Pull title + icon from the workspace index. Falls
                // back to the slug when the index doesn't know about
                // this page yet (just-created file, before the next
                // rebuild). Title is preferred over slug because it's
                // what the user wrote — `Page · CTO` reads better than
                // `Page · cto`.
                let entry = self.index.by_slug(stem);
                let display_name = entry
                    .map(|e| e.title.clone())
                    .unwrap_or_else(|| stem.to_string());
                let icon_prefix = entry
                    .and_then(|e| e.icon.as_deref())
                    .map(|i| format!("{i} "))
                    .unwrap_or_default();
                format!("Page · {icon_prefix}{display_name} · [{mode_tag}]")
            }
        }
    }

    /// Slug of the currently-opened view, used to look up backlinks.
    pub(crate) fn current_slug(&self) -> String {
        self.current_path()
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string()
    }
}
