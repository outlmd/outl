//! Yank register + copy-to-OS-clipboard. Vim spelling: `yy` copies one
//! block, Visual `y` copies the range. Every yank writes clean canonical
//! outl markdown to the OS clipboard (arboard + OSC 52) and mirrors it in
//! the in-memory register. Pasting (`p` / `P`) reads the **OS clipboard**,
//! not this register — see `actions/paste.rs`.

use std::io::{IsTerminal, Write};

use base64::Engine as _;

use crate::outline_ops::{index_for_path, node_at_path, path_for_index};
use crate::state::{App, Mode, View};

/// Process-wide `arboard` handle used for every write.
///
/// On X11 there is no system clipboard buffer: the selection *owner*
/// serves the contents to whoever asks. `arboard` runs that server in a
/// background thread tied to the `Clipboard` instance, and dropping the
/// last instance hands the data to a clipboard manager only if one is
/// running — otherwise the contents are gone. A fresh `Clipboard` per
/// yank therefore loses the text immediately on a bare X11 session (no
/// desktop environment, no clipboard manager) even though `set_text`
/// returned `Ok`. Keeping one instance alive for the whole process keeps
/// the TUI the selection owner while it runs, or until something else
/// takes the selection over. The contents do not outlive the process on a
/// bare X11 session.
///
/// Reads (`actions/paste.rs`) don't need this: they only ask the current
/// owner.
static CLIPBOARD: std::sync::Mutex<Option<arboard::Clipboard>> = std::sync::Mutex::new(None);

/// Best-effort copy of `text` to the OS clipboard.
///
/// Returns `true` on success. Failures (no display server, missing
/// clipboard daemon, sandboxed terminal, headless CI) are swallowed
/// — the caller still has `last_yanked_ref` + status as the fallback
/// surface. We never panic over clipboard plumbing.
fn copy_to_os_clipboard(text: &str) -> bool {
    let Ok(mut guard) = CLIPBOARD.lock() else {
        return false;
    };
    if guard.is_none() {
        // Retried on every call while it fails, so a display that shows
        // up later (or a headless run) behaves exactly as before.
        *guard = arboard::Clipboard::new().ok();
    }
    let ok = guard
        .as_mut()
        .is_some_and(|c| c.set_text(text.to_string()).is_ok());
    if !ok {
        // A dead display connection (X server restart, dropped SSH
        // forwarding) would otherwise fail every later yank. Dropping the
        // handle makes the next call reconnect, as a fresh `Clipboard`
        // per yank used to.
        *guard = None;
    }
    ok
}

/// Emit an OSC 52 clipboard escape on stdout.
///
/// `arboard` needs a local display server, so it can't reach the user's
/// real clipboard over SSH, inside Crostini, or through tmux. OSC 52 is
/// the terminal's own copy channel — the emulator forwards the payload to
/// the outer clipboard. We only emit when stdout is a real terminal so a
/// headless / piped run never spews the escape into captured output.
///
/// Best-effort like [`copy_to_os_clipboard`]: returns `true` if the
/// sequence was written, but a `true` here means "the terminal got it",
/// not "the terminal honoured it" (some emulators ignore OSC 52, and tmux
/// needs `set-clipboard on`). We never panic over clipboard plumbing.
fn emit_osc52(text: &str) -> bool {
    let mut stdout = std::io::stdout();
    if !stdout.is_terminal() {
        return false;
    }
    let payload = base64::engine::general_purpose::STANDARD.encode(text.as_bytes());
    // OSC 52: ESC ] 52 ; c ; <base64> BEL — `c` selects the clipboard
    // (as opposed to a primary / cut-buffer selection).
    let seq = format!("\x1b]52;c;{payload}\x07");
    stdout
        .write_all(seq.as_bytes())
        .and_then(|()| stdout.flush())
        .is_ok()
}

/// Copy `markdown` to the OS clipboard through every channel available:
/// `arboard` (local display server) **and** an OSC 52 escape (the remote
/// path). Returns `true` if at least one channel accepted the write.
///
/// Both run unconditionally — a local terminal under a display server may
/// satisfy `arboard`, while an SSH / Crostini session only has OSC 52; we
/// don't know which the user is on, so we feed both.
fn copy_markdown_to_clipboard(markdown: &str) -> bool {
    let via_arboard = copy_to_os_clipboard(markdown);
    let via_osc52 = emit_osc52(markdown);
    via_arboard || via_osc52
}

/// Status line after a block yank: always reports the yank (the in-app
/// register is filled regardless), and appends the clipboard outcome so
/// the user knows whether an out-of-app paste will work.
fn block_yank_status(count: usize, copied: bool) -> String {
    let plural = if count == 1 { "" } else { "s" };
    if copied {
        format!("yanked {count} block{plural} → clipboard")
    } else {
        format!("yanked {count} block{plural} (clipboard unavailable)")
    }
}

/// Build the status-line message after a yank attempt.
///
/// `kind` is the human label (`"ref"`, `"embed"`); `token` is the
/// thing that landed on the clipboard. `copied` flips the wording so
/// the user knows whether to expect a paste to work.
fn clipboard_message(kind: &str, token: &str, copied: bool) -> String {
    if copied {
        format!("copied {kind} {token} to clipboard")
    } else {
        format!("yanked {kind} {token} (clipboard unavailable)")
    }
}

impl App {
    /// `yr` — capture the block ref handle of the currently selected
    /// block.
    ///
    /// Looks up the block in the workspace index by `(source_slug,
    /// source_block_path)` and stashes its `((blk-XXXXXX))` form on
    /// `last_yanked_ref` + the status line. `arboard` also writes it
    /// to the OS clipboard so a regular paste works in other apps;
    /// the status line falls back to `(clipboard unavailable)` on
    /// headless / no-display environments.
    ///
    /// Lookup is O(1) — `WorkspaceIndex::block_at_location` uses the
    /// `(slug, dfs_path) → NodeId` secondary index, so the chord stays
    /// snappy regardless of workspace size.
    pub(crate) fn yank_current_ref(&mut self) {
        match self.current_block_ref_handle() {
            Some(h) => {
                let token = format!("(({h}))");
                self.last_yanked_ref = Some(token.clone());
                self.status = clipboard_message("ref", &token, copy_to_os_clipboard(&token));
            }
            None => {
                self.status = "no ref handle yet — save and retry".into();
            }
        }
    }

    /// Yank the **embed** form of the current block: `!((blk-XXXXXX))`.
    ///
    /// Same lookup as [`yank_current_ref`] but stores the embed
    /// formatting so a downstream paste expands the source block
    /// inline instead of linking to it.
    pub(crate) fn yank_current_embed(&mut self) {
        match self.current_block_ref_handle() {
            Some(h) => {
                let token = format!("!(({h}))");
                self.last_yanked_ref = Some(token.clone());
                self.status = clipboard_message("embed", &token, copy_to_os_clipboard(&token));
            }
            None => {
                self.status = "no ref handle yet — save and retry".into();
            }
        }
    }

    /// Resolve the selected block's stable ref handle by looking up
    /// `(source_slug, source_block_path)` in the workspace index.
    ///
    /// O(1) thanks to `WorkspaceIndex::block_at_location`. Returns
    /// `None` when:
    /// - the cursor isn't on a real block (empty page edge case), or
    /// - the block was just created in-memory and the sidecar hasn't
    ///   landed yet (no `BlockEntry` to find).
    pub(crate) fn current_block_ref_handle(&self) -> Option<String> {
        let path = path_for_index(&self.page.blocks, self.selected)?;
        let slug = match &self.view {
            View::Page(p) => p
                .file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.to_string())
                .unwrap_or_default(),
            View::Journal(d) => d.format("%Y-%m-%d").to_string(),
        };
        self.index
            .block_at_location(&slug, &path)
            .map(|b| b.ref_handle.clone())
    }

    /// Copy the current block (with its subtree) into the yank
    /// register. Doesn't mutate the page.
    pub(crate) fn yank_current(&mut self) {
        let Some(path) = path_for_index(&self.page.blocks, self.selected) else {
            return;
        };
        if let Some(node) = node_at_path(&self.page.blocks, &path) {
            self.yank_register = vec![node.clone()];
            let md = outl_actions::copy_markdown_nodes(&self.yank_register);
            let copied = copy_markdown_to_clipboard(&md);
            self.status = block_yank_status(1, copied);
        }
    }

    /// Copy every block in the Visual range. The range stays in
    /// flat-index space, so we walk it twice — once to grab nodes,
    /// once to drop the Visual mode.
    pub(crate) fn yank_visual_range(&mut self) {
        let Some((lo, hi)) = self.visual_range() else {
            return;
        };
        let mut grabbed = Vec::new();
        for idx in lo..=hi {
            let Some(path) = path_for_index(&self.page.blocks, idx) else {
                continue;
            };
            // Skip a block whose parent is also inside the range — it's
            // already carried inside that parent's cloned subtree, so
            // taking it again would duplicate it on paste / copy. Only
            // the range's top-level roots are grabbed.
            if let Some((_, parent_path)) = path.split_last() {
                if !parent_path.is_empty() {
                    if let Some(pidx) = index_for_path(&self.page.blocks, parent_path) {
                        if (lo..=hi).contains(&pidx) {
                            continue;
                        }
                    }
                }
            }
            if let Some(node) = node_at_path(&self.page.blocks, &path) {
                grabbed.push(node.clone());
            }
        }
        let n = grabbed.len();
        self.yank_register = grabbed;
        let md = outl_actions::copy_markdown_nodes(&self.yank_register);
        let copied = copy_markdown_to_clipboard(&md);
        self.remember_visual_range();
        self.mode = Mode::Normal;
        self.status = block_yank_status(n, copied);
    }
}
