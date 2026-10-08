//! External clipboard paste — convert markdown into a tree of blocks.
//!
//! Wired to `Event::Paste` from crossterm's bracketed-paste support.
//! Delegates the work to [`outl_actions::paste_markdown`] so the
//! semantics stay identical between the TUI and the mobile client.
//!
//! ## v0 anchor policy
//!
//! For simplicity, the TUI always uses [`outl_actions::PasteAnchor::AfterBlock`]
//! against the currently selected block. The mobile client uses
//! `AtCaret` when the paste happens inside a textarea — we deliberately
//! do not match that here in v0 because the TUI runs an AST-first edit
//! pipeline (the buffer is the source while in Insert; the workspace
//! is the source while in Normal). Reusing `AtCaret` from inside Insert
//! would require swapping the workspace state mid-edit, which the
//! peer-sync code path explicitly avoids (see `poll_jsonl_updates`).
//!
//! What we do instead in Insert mode: commit the in-flight buffer
//! first, then paste, then reload the workspace from disk so the new
//! tree shows up.

use std::path::{Path, PathBuf};

use outl_actions::{
    children_of, find_by_slug, import_asset, looks_structured, paste_markdown, paste_plain,
    PasteAnchor, PasteOutcome,
};
use outl_core::id::NodeId;
use outl_core::workspace::Workspace;

use crate::state::{App, EditTarget, Mode};

/// Read the OS clipboard, best-effort. `None` on a headless / no-display
/// environment (the same degradation as the copy side's `arboard`).
fn read_os_clipboard() -> Option<String> {
    arboard::Clipboard::new()
        .and_then(|mut c| c.get_text())
        .ok()
}

impl App {
    /// Apply a bracketed-paste payload to the workspace.
    ///
    /// `text` is the verbatim clipboard contents reported by crossterm.
    /// The function:
    ///
    /// 1. Commits any in-flight Insert buffer to disk first so the
    ///    workspace is in a clean state before we apply ops to it.
    /// 2. Resolves the currently selected block's `NodeId` via the
    ///    workspace index (sidecar-backed, O(1)).
    /// 3. Routes the text through `outl_actions::paste_markdown` with
    ///    [`PasteAnchor::AfterBlock`].
    /// 4. Reloads the materialised page from disk and updates the
    ///    status line with the block count the user just pasted.
    ///
    /// Empty paste, no selected block, or no index entry for the
    /// selected block are all soft failures that surface in the
    /// status line and leave the workspace untouched.
    pub(crate) fn paste_external(&mut self, text: String) {
        if text.is_empty() {
            return;
        }
        // Drag-and-drop upload. Terminals report a file dragged into the
        // window as its path pasted as text (bracketed paste). While
        // editing a block, if the payload is exactly the path(s) of
        // existing file(s), import them into the workspace and splice the
        // markdown link(s) at the caret instead of the raw path. Gated on
        // Insert mode + the file actually existing on disk so a legitimate
        // paste of a path string in the outline is never hijacked.
        if matches!(self.mode, Mode::Insert { .. }) {
            if let Some(paths) = looks_like_dropped_files(&text) {
                self.insert_dropped_assets(&paths);
                return;
            }
        }
        // Plain-text paste inside Insert mode is the common "drop a
        // URL / snippet into what I'm writing" workflow. Splicing the
        // raw text into the live buffer keeps the keyboard up and
        // the cursor where the user expects. Structured pastes — an
        // outline, a markdown table, a spreadsheet's tab-separated rows
        // — still go through the full pipeline below so they create
        // siblings (or one table block) as documented.
        if !looks_structured(&text) {
            if let Mode::Insert { buffer, .. } = &mut self.mode {
                buffer.insert_str(&text);
                self.status = "pasted text".into();
                return;
            }
        }
        self.graft_paste(text, false);
    }

    /// `p` — paste the OS clipboard **with formatting** after the
    /// selected block: outline syntax is converted and multi-paragraph
    /// text is split into one block per paragraph.
    pub(crate) fn paste_clipboard_formatted(&mut self) {
        match read_os_clipboard() {
            Some(text) => self.graft_paste(text, false),
            None => self.status = "clipboard unavailable".into(),
        }
    }

    /// `P` — paste the OS clipboard **without formatting** after the
    /// selected block: the raw text lands as a single block, no
    /// conversion or splitting.
    pub(crate) fn paste_clipboard_plain(&mut self) {
        match read_os_clipboard() {
            Some(text) => self.graft_paste(text, true),
            None => self.status = "clipboard unavailable".into(),
        }
    }

    /// Commit any in-flight edit, resolve the selected block, and graft
    /// `text` after it — through `paste_markdown` (formatted) when
    /// `plain` is false, or `paste_plain` (raw) when true. Reloads the
    /// workspace and repositions the cursor onto the new tail.
    fn graft_paste(&mut self, text: String, plain: bool) {
        if text.is_empty() {
            return;
        }
        // `commit_insert` writes the in-flight buffer back into the
        // AST and — when the buffer changed against the current page
        // — already calls `save()` (render → write → reconcile)
        // internally. Calling `save()` again afterwards would pay the
        // I/O + reconcile a second time for nothing. Track whether
        // the upcoming `commit_insert` will save the current page so
        // we skip the redundant call below.
        let commit_will_save_current = match &self.mode {
            Mode::Insert {
                target,
                buffer,
                original_text,
                ..
            } => matches!(target, EditTarget::CurrentPage) && buffer.as_string() != *original_text,
            _ => false,
        };
        if matches!(self.mode, Mode::Insert { .. }) {
            self.commit_insert();
        }
        // Force a save + reconcile *before* resolving the selected
        // block's NodeId so the workspace tree mirrors the in-memory
        // AST. Otherwise a freshly opened journal (or a `.md`
        // imported externally that the orphan scanner hasn't picked
        // up yet) leaves the tree with fewer children than the AST
        // shows, and the path walk dead-ends.
        if !commit_will_save_current {
            self.save();
        }

        let slug = self.current_slug();
        let Some(path) = outl_md::outline_ops::path_for_index(&self.page.blocks, self.selected)
        else {
            self.status = "paste: no selected block".into();
            return;
        };
        // Resolve the selected block's NodeId by walking the workspace
        // tree directly. We deliberately don't go through
        // `WorkspaceIndex::block_at_location` here: the index is
        // sidecar-backed and rebuilt off the critical path, so right
        // after a freshly opened journal (or a previous paste that
        // hasn't reprojected yet) the entry may not exist. The
        // workspace tree is always up to date.
        let Some(page_id) = find_by_slug(&self.workspace, &slug) else {
            self.status = "paste: current page not in workspace".into();
            return;
        };
        let Some(node_id) = resolve_node_id_at_path(&self.workspace, page_id, &path) else {
            self.status = "paste: could not resolve selected block in tree".into();
            return;
        };

        let anchor = PasteAnchor::AfterBlock(node_id);
        let result: Result<PasteOutcome, _> = if plain {
            paste_plain(&mut self.workspace, &self.hlc, anchor, &text)
        } else {
            paste_markdown(&mut self.workspace, &self.hlc, anchor, &text)
        };
        match result {
            Ok(out) => {
                // Full refresh: re-read everything from disk and
                // rebuild the workspace index. The lighter
                // `reload_workspace_from_disk` path leaves the page
                // list and index pointing at pre-paste state, which
                // showed up as ghost cells on the right edge of the
                // outline after the user moved the cursor.
                self.reload_workspace_from_disk();
                self.refresh_page_list();
                self.spawn_index_rebuild();
                // Land the selection on the bottom of what we just
                // pasted so the user sees the new tail without
                // scrolling — nicer than landing wherever the post-
                // reload flat index happens to fall.
                self.flat_len = outl_md::outline_ops::flat_count(&self.page.blocks);
                let landed = self.flat_index_for_node(node_id, out.new_blocks.last().copied());
                if let Some(idx) = landed {
                    self.selected = idx.min(self.flat_len.saturating_sub(1));
                }
                self.cursor_col = 0;
                // Any half-pressed Vim chord (`y`, `g`, `d`, `q`) from
                // before the paste must not survive — otherwise the
                // next keystroke fires the chord against a freshly
                // pasted block the user hadn't reviewed yet.
                self.pending_chord = None;
                self.status = if out.root_count > 0 {
                    format!(
                        "pasted {n} block{s}",
                        n = out.root_count,
                        s = if out.root_count == 1 { "" } else { "s" }
                    )
                } else {
                    "pasted text".into()
                };
            }
            Err(e) => {
                self.status = format!("paste failed: {e}");
            }
        }
    }

    /// Import the dragged file(s) into `<root>/assets/` and splice the
    /// resulting markdown link(s) at the Insert caret.
    ///
    /// The asset bytes land on disk immediately (`import_asset`); only
    /// the link enters the block text, and it does so through the same
    /// buffer splice the normal Insert-mode paste uses — so the edit
    /// commits on the next Esc like any other keystroke, keeping the
    /// AST-first persistence model intact.
    ///
    /// On an import error (file too large, IO) we surface a status-line
    /// message and insert **nothing** — pasting the raw path would leave
    /// junk in the block, which is worse than a no-op the user can retry.
    fn insert_dropped_assets(&mut self, paths: &[PathBuf]) {
        let max_bytes = outl_config::load().assets.max_bytes;
        let mut links: Vec<String> = Vec::with_capacity(paths.len());
        for path in paths {
            match import_asset(&self.workspace_root, path, max_bytes) {
                Ok(imported) => links.push(imported.markdown),
                Err(e) => {
                    self.status = format!("import failed: {e}");
                    return;
                }
            }
        }
        if let Mode::Insert { buffer, .. } = &mut self.mode {
            buffer.insert_str(&links.join(" "));
        }
        let n = links.len();
        self.status = format!("imported {n} file{s}", s = if n == 1 { "" } else { "s" });
    }

    /// Locate a freshly-pasted block in the current AST so the caller
    /// can move the selection cursor onto it. Tries the last pasted
    /// id first, falls back to the anchor block.
    fn flat_index_for_node(&self, anchor: NodeId, last_pasted: Option<NodeId>) -> Option<usize> {
        let target = last_pasted.unwrap_or(anchor);
        self.id_by_flat.iter().position(|id| *id == target)
    }
}

/// Resolve one trimmed token to an existing file path.
///
/// Honours the macOS drag convention of backslash-escaping spaces
/// (`/My\ Files/report.pdf`): the raw token is tried first, then — only
/// when it isn't already a file — the `\ ` → ` ` unescaped form. Returns
/// `None` when neither points at an existing file.
fn existing_file_from_token(token: &str) -> Option<PathBuf> {
    let direct = Path::new(token);
    if direct.is_file() {
        return Some(direct.to_path_buf());
    }
    if token.contains('\\') {
        let unescaped = token.replace("\\ ", " ");
        let candidate = Path::new(&unescaped);
        if candidate.is_file() {
            return Some(candidate.to_path_buf());
        }
    }
    None
}

/// Decide whether a bracketed-paste payload is a **single** dragged file.
///
/// Returns `Some(path)` only when the trimmed payload is one line that
/// names an existing file on disk; `None` otherwise. This is the pure
/// anti-hijack heuristic: pasting a real file's path into an outliner is
/// rare, so requiring a single line **and** `is_file()` keeps the common
/// case (dragging a file out of the terminal) from stealing legitimate
/// text pastes. Kept side-effect-free so it can be unit-tested apart from
/// the import.
pub(crate) fn looks_like_dropped_file_path(pasted: &str) -> Option<PathBuf> {
    let trimmed = pasted.trim();
    if trimmed.is_empty() || trimmed.contains('\n') {
        return None;
    }
    existing_file_from_token(trimmed)
}

/// Extend [`looks_like_dropped_file_path`] to the multi-file drop:
/// terminals separate several dragged files by newlines. Returns the
/// paths only when **every** non-empty line is an existing file, so a
/// single stray non-path line falls the whole payload back to the normal
/// paste flow. A single-file drop short-circuits through the same
/// per-token check, so the two paths can't disagree on what counts.
pub(crate) fn looks_like_dropped_files(pasted: &str) -> Option<Vec<PathBuf>> {
    if let Some(path) = looks_like_dropped_file_path(pasted) {
        return Some(vec![path]);
    }
    let lines: Vec<&str> = pasted
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    if lines.len() < 2 {
        return None;
    }
    let mut paths = Vec::with_capacity(lines.len());
    for line in lines {
        paths.push(existing_file_from_token(line)?);
    }
    Some(paths)
}

/// Walk the tree from `page_id` following the DFS path produced by
/// `outl_md::outline_ops::path_for_index`. Returns the `NodeId` at
/// `path` when every step lines up, `None` if any segment is out of
/// range (in practice only happens when the AST drifted from the
/// workspace state, e.g. a peer added blocks since the last reload).
pub(crate) fn resolve_node_id_at_path(
    workspace: &Workspace,
    page_id: NodeId,
    path: &[usize],
) -> Option<NodeId> {
    let mut current = page_id;
    for &idx in path {
        let kids = children_of(workspace, current);
        let (child, _) = kids.into_iter().nth(idx)?;
        current = child;
    }
    Some(current)
}

#[cfg(test)]
mod tests {
    use super::{looks_like_dropped_file_path, looks_like_dropped_files};
    use tempfile::tempdir;

    #[test]
    fn single_existing_file_is_detected() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("report.pdf");
        std::fs::write(&file, b"%PDF fake").unwrap();
        let pasted = file.to_string_lossy().to_string();

        assert_eq!(looks_like_dropped_file_path(&pasted), Some(file));
    }

    #[test]
    fn surrounding_whitespace_is_trimmed() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("note.txt");
        std::fs::write(&file, b"hi").unwrap();
        let pasted = format!("  {}\n", file.to_string_lossy());

        assert_eq!(looks_like_dropped_file_path(&pasted), Some(file));
    }

    #[test]
    fn nonexistent_path_is_ignored() {
        // A plausible path string that isn't a real file must not be
        // hijacked — this is the anti-hijack guard.
        assert_eq!(looks_like_dropped_file_path("/does/not/exist.pdf"), None);
    }

    #[test]
    fn multi_line_single_path_is_not_a_single_drop() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("a.txt");
        std::fs::write(&file, b"x").unwrap();
        // Two lines where only the first is a file → not a single drop.
        let pasted = format!("{}\njust some text", file.to_string_lossy());
        assert_eq!(looks_like_dropped_file_path(&pasted), None);
    }

    #[test]
    fn backslash_escaped_space_resolves() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("my report.pdf");
        std::fs::write(&file, b"x").unwrap();
        // macOS drags escape spaces: `/dir/my\ report.pdf`.
        let escaped = file.to_string_lossy().replace(' ', "\\ ");
        assert_eq!(looks_like_dropped_file_path(&escaped), Some(file));
    }

    #[test]
    fn multiple_files_by_newline_all_import() {
        let dir = tempdir().unwrap();
        let a = dir.path().join("a.pdf");
        let b = dir.path().join("b.png");
        std::fs::write(&a, b"x").unwrap();
        std::fs::write(&b, b"y").unwrap();
        let pasted = format!("{}\n{}", a.to_string_lossy(), b.to_string_lossy());

        assert_eq!(looks_like_dropped_files(&pasted), Some(vec![a, b]));
    }

    #[test]
    fn multi_drop_with_one_bogus_line_falls_back() {
        let dir = tempdir().unwrap();
        let a = dir.path().join("a.pdf");
        std::fs::write(&a, b"x").unwrap();
        let pasted = format!("{}\n/nope/missing.png", a.to_string_lossy());
        assert_eq!(looks_like_dropped_files(&pasted), None);
    }

    #[test]
    fn empty_paste_is_never_a_drop() {
        assert_eq!(looks_like_dropped_file_path(""), None);
        assert_eq!(looks_like_dropped_files("   \n  "), None);
    }
}
