//! The properties overlay (`g p`): list, edit, create and delete the
//! `key:: value` pairs on the selected block — or on the page.
//!
//! `:prop` / `:prop-page` stay: they are the fast path for someone who
//! already knows the key. This overlay is for everyone else, and the
//! two affordances that make it worth opening are the ones a command
//! line cannot offer:
//!
//! - **Key completion from the workspace.** Keys in a real graph are
//!   few and repeat (`related`, `icon`, `status`, `work`), so `Tab`
//!   completes from [`outl_actions::known_keys`] — frequency-ranked,
//!   one owner, shared with the GUI clients.
//! - **`[[` in the value.** Most property values in a real graph are a
//!   `[[page]]` or a `#tag`, so the value field reuses the very same
//!   trigger detection and candidate ranking Insert mode ships
//!   (`actions::autocomplete`), rendered by the same popup.
//!
//! Writes route through [`App::set_property_on_current_block`] /
//! [`App::set_property_on_page`] — the AST-first path `:prop` uses, so
//! a property typed here shows up in the outline immediately instead of
//! waiting for a save to project it back.

use crate::actions::autocomplete::detect_trigger;
use crate::outline_ops::{node_at_path, path_for_index};
use crate::state::{
    App, AutocompleteKind, AutocompleteState, Focus, Mode, Overlay, PropertiesState, PropertyEdit,
    PropertyField, PropertyScope, ToastKind,
};

/// Catalogue entries that match what the user has typed so far,
/// most-used first.
///
/// Prefix matches come before substring matches — typing `re` should
/// offer `related` before `page-ref`, but must still find the second
/// one rather than pretending it doesn't exist. Within each group the
/// catalogue's own frequency order is preserved, which is the whole
/// reason to call [`outl_actions::known_keys`] instead of sorting
/// alphabetically here.
///
/// Case-insensitive, matching the parser and the catalogue's own
/// folding: a workspace holding `Remind::` must complete from `re`.
pub(crate) fn key_completions(known: &[String], query: &str) -> Vec<String> {
    if query.is_empty() {
        return known.to_vec();
    }
    let q = query.to_lowercase();
    let mut prefix = Vec::new();
    let mut contains = Vec::new();
    for key in known {
        let lower = key.to_lowercase();
        if lower.starts_with(&q) {
            prefix.push(key.clone());
        } else if lower.contains(&q) {
            contains.push(key.clone());
        }
    }
    prefix.extend(contains);
    prefix
}

/// Replace the open `[[` / `#` trigger at the end of `value` with the
/// completed token.
///
/// Pure, and separate from [`App::accept_autocomplete`] because that
/// one writes into the Insert-mode `EditBuffer`. Same *semantics*, a
/// different destination — the overlay's value is a plain `String`
/// with the caret pinned to the end.
pub(crate) fn apply_value_completion(
    value: &str,
    kind: AutocompleteKind,
    query: &str,
    choice: &str,
) -> String {
    let trigger_len = match kind {
        AutocompleteKind::PageRef => 2 + query.chars().count(),
        AutocompleteKind::Tag => 1 + query.chars().count(),
        // Every other trigger is inert in this field (see
        // `App::refresh_property_value_completion`), so leaving the
        // buffer untouched is the honest answer.
        _ => return value.to_string(),
    };
    let kept: String = {
        let chars: Vec<char> = value.chars().collect();
        chars[..chars.len().saturating_sub(trigger_len)]
            .iter()
            .collect()
    };
    match kind {
        AutocompleteKind::PageRef => format!("{kept}[[{choice}]]"),
        AutocompleteKind::Tag => format!("{kept}#{choice}"),
        _ => value.to_string(),
    }
}

impl App {
    /// `g p` — open the properties overlay on the selected block.
    pub(crate) fn open_properties(&mut self) {
        if !matches!(self.focus, Focus::Outline) || !matches!(self.mode, Mode::Normal) {
            // Silence here would read as "the chord is broken". The
            // backlinks panel shows blocks from *other* pages, and
            // editing their properties is a cross-page write with its
            // own commit semantics — see the crate's backlink-edit
            // note. Say so instead of doing nothing.
            self.toast(
                ToastKind::Info,
                "properties need a block in the outline — press Esc first",
            );
            return;
        }
        // The key catalogue reads the workspace tree, and the TUI's
        // saves are coalesced — a property typed a keystroke ago is
        // still in the pending write. Flush first so the completion
        // list is not one edit behind what the user just did.
        self.flush_pending_save();
        self.open_properties_scope(PropertyScope::Block, 0);
    }

    /// `p` inside the overlay — flip between the block's properties
    /// and the page's.
    pub(crate) fn toggle_properties_scope(&mut self) {
        let next = match &self.overlay {
            Some(Overlay::Properties(p)) => match p.scope {
                PropertyScope::Block => PropertyScope::Page,
                PropertyScope::Page => PropertyScope::Block,
            },
            _ => return,
        };
        self.open_properties_scope(next, 0);
    }

    /// (Re)build the overlay for `scope`, landing the cursor on `want`
    /// (clamped — a delete shortens the list).
    fn open_properties_scope(&mut self, scope: PropertyScope, want: usize) {
        let rows = self.property_rows(scope);
        let known_keys = outl_actions::known_keys(&self.workspace)
            .into_iter()
            .map(|(key, _count)| key)
            .collect();
        let selected = want.min(rows.len().saturating_sub(1));
        self.autocomplete = None;
        self.overlay = Some(Overlay::Properties(PropertiesState {
            scope,
            rows,
            selected,
            editing: None,
            pending_delete: false,
            message: None,
            known_keys,
        }));
    }

    /// The `(key, value)` pairs the overlay lists for `scope`.
    ///
    /// Read from the parsed AST, not from the workspace tree, because
    /// that is where [`App::set_property_on_current_block`] writes.
    /// Reading the tree instead would list a value the user cannot see
    /// in the outline and silently drop the one they just typed.
    fn property_rows(&self, scope: PropertyScope) -> Vec<(String, String)> {
        match scope {
            PropertyScope::Block => path_for_index(&self.page.blocks, self.selected)
                .and_then(|path| node_at_path(&self.page.blocks, &path))
                .map(|node| node.properties.clone())
                .unwrap_or_default(),
            // `page-slug` / `page-kind` are the page's identity, not
            // user metadata. `outl_actions::tree` owns that predicate;
            // asking it here keeps the overlay from ever offering the
            // two keys that would rewrite the page's identity.
            PropertyScope::Page => self
                .page
                .properties
                .iter()
                .filter(|(k, _)| !outl_actions::tree::is_page_model_key(k))
                .cloned()
                .collect(),
        }
    }

    /// Refresh `rows` from the AST after a write, keeping the cursor.
    fn refresh_properties(&mut self, want: usize) {
        let scope = match &self.overlay {
            Some(Overlay::Properties(p)) => p.scope,
            _ => return,
        };
        self.open_properties_scope(scope, want);
    }

    /// `j` / `k` in the overlay. Clamps rather than wrapping, matching
    /// every other TUI list.
    pub(crate) fn properties_move(&mut self, delta: i32) {
        let Some(Overlay::Properties(ref mut p)) = self.overlay else {
            return;
        };
        p.pending_delete = false;
        if p.rows.is_empty() {
            return;
        }
        let last = p.rows.len() - 1;
        p.selected = (p.selected as i32 + delta).clamp(0, last as i32) as usize;
    }

    /// `Enter` on a row — edit that property's value.
    ///
    /// The key stays fixed: renaming a key is a delete plus a create,
    /// and doing it silently under an `Enter` would strand whatever
    /// queries the old key.
    pub(crate) fn properties_begin_edit(&mut self) {
        let Some(Overlay::Properties(ref mut p)) = self.overlay else {
            return;
        };
        let Some((key, value)) = p.rows.get(p.selected).cloned() else {
            p.message = Some("nothing to edit — press `o` to add a property".into());
            return;
        };
        p.pending_delete = false;
        p.message = None;
        p.editing = Some(PropertyEdit {
            field: PropertyField::Value,
            key,
            value,
            original_key: p.rows.get(p.selected).map(|(k, _)| k.clone()),
            key_matches: Vec::new(),
            key_match_idx: 0,
        });
        self.refresh_property_value_completion();
    }

    /// `o` in the overlay — start a brand new property, key first.
    pub(crate) fn properties_begin_new(&mut self) {
        let Some(Overlay::Properties(ref mut p)) = self.overlay else {
            return;
        };
        p.pending_delete = false;
        p.message = None;
        let key_matches = key_completions(&p.known_keys, "");
        p.editing = Some(PropertyEdit {
            field: PropertyField::Key,
            key: String::new(),
            value: String::new(),
            original_key: None,
            key_matches,
            key_match_idx: 0,
        });
    }

    /// `d d` in the overlay — remove the highlighted property.
    pub(crate) fn properties_delete_selected(&mut self) {
        let target = match &self.overlay {
            Some(Overlay::Properties(p)) => p.rows.get(p.selected).map(|(k, _)| k.clone()),
            _ => return,
        };
        let Some(key) = target else {
            if let Some(Overlay::Properties(ref mut p)) = self.overlay {
                p.pending_delete = false;
                p.message = Some("nothing to delete".into());
            }
            return;
        };
        let want = match &self.overlay {
            Some(Overlay::Properties(p)) => p.selected,
            _ => 0,
        };
        // An empty value is the delete path in both writers.
        self.write_property(&key, "");
        self.refresh_properties(want);
        if let Some(Overlay::Properties(ref mut p)) = self.overlay {
            p.message = Some(format!("deleted `{key}`"));
        }
    }

    /// `Esc` while editing — drop the buffer, keep the overlay.
    pub(crate) fn properties_cancel_edit(&mut self) {
        self.autocomplete = None;
        let Some(Overlay::Properties(ref mut p)) = self.overlay else {
            return;
        };
        p.editing = None;
        p.message = None;
    }

    /// `Enter` while editing: Key → Value, then Value → commit.
    pub(crate) fn properties_advance_or_commit(&mut self) {
        let field = match &self.overlay {
            Some(Overlay::Properties(p)) => p.editing.as_ref().map(|e| e.field),
            _ => None,
        };
        match field {
            Some(PropertyField::Key) => self.properties_confirm_key(),
            Some(PropertyField::Value) => self.properties_commit_edit(),
            None => {}
        }
    }

    /// Leave the Key field for the Value field, refusing the two keys
    /// that are the page's identity rather than its metadata.
    fn properties_confirm_key(&mut self) {
        let Some(Overlay::Properties(ref mut p)) = self.overlay else {
            return;
        };
        let Some(edit) = p.editing.as_mut() else {
            return;
        };
        let key = edit.key.trim().to_string();
        if key.is_empty() {
            p.message = Some("a property needs a key".into());
            return;
        }
        if outl_actions::tree::is_page_model_key(&key) {
            p.message = Some(format!("`{key}` is the page's identity, not a property"));
            return;
        }
        edit.key = key;
        edit.field = PropertyField::Value;
        p.message = None;
        self.refresh_property_value_completion();
    }

    /// Write the in-flight edit through the same path `:prop` uses.
    fn properties_commit_edit(&mut self) {
        self.autocomplete = None;
        let (key, value) = {
            let Some(Overlay::Properties(ref p)) = self.overlay else {
                return;
            };
            let Some(edit) = p.editing.as_ref() else {
                return;
            };
            (edit.key.trim().to_string(), edit.value.trim().to_string())
        };
        if key.is_empty() {
            if let Some(Overlay::Properties(ref mut p)) = self.overlay {
                p.message = Some("a property needs a key".into());
            }
            return;
        }
        self.write_property(&key, &value);
        // Land the cursor on what was just written, so a follow-up
        // `Enter` re-opens the property the user is looking at.
        let rows = self.property_rows(match &self.overlay {
            Some(Overlay::Properties(p)) => p.scope,
            _ => return,
        });
        let want = rows
            .iter()
            .position(|(k, _)| k.eq_ignore_ascii_case(&key))
            .unwrap_or(0);
        self.refresh_properties(want);
        if let Some(Overlay::Properties(ref mut p)) = self.overlay {
            p.message = Some(if value.is_empty() {
                format!("deleted `{key}`")
            } else {
                format!("{key} = {value}")
            });
        }
    }

    /// Route a write to the scope's owner. Empty `value` deletes in
    /// both, which is why the overlay's `dd` needs no second writer.
    fn write_property(&mut self, key: &str, value: &str) {
        let Some(Overlay::Properties(p)) = &self.overlay else {
            return;
        };
        match p.scope {
            PropertyScope::Block => self.set_property_on_current_block(key, value),
            PropertyScope::Page => self.set_property_on_page(key, value),
        }
    }

    /// A printable character landed in the in-flight edit.
    pub(crate) fn properties_edit_push(&mut self, c: char) {
        {
            let Some(Overlay::Properties(ref mut p)) = self.overlay else {
                return;
            };
            let Some(edit) = p.editing.as_mut() else {
                return;
            };
            match edit.field {
                PropertyField::Key => {
                    edit.key.push(c);
                    edit.key_matches = key_completions(&p.known_keys, &edit.key);
                    edit.key_match_idx = 0;
                }
                PropertyField::Value => edit.value.push(c),
            }
        }
        self.refresh_property_value_completion();
    }

    /// Backspace in the in-flight edit.
    pub(crate) fn properties_edit_backspace(&mut self) {
        {
            let Some(Overlay::Properties(ref mut p)) = self.overlay else {
                return;
            };
            let Some(edit) = p.editing.as_mut() else {
                return;
            };
            match edit.field {
                PropertyField::Key => {
                    edit.key.pop();
                    edit.key_matches = key_completions(&p.known_keys, &edit.key);
                    edit.key_match_idx = 0;
                }
                PropertyField::Value => {
                    edit.value.pop();
                }
            }
        }
        self.refresh_property_value_completion();
    }

    /// `Tab` in the Key field — take the next catalogue match.
    ///
    /// Cycles, so holding `Tab` walks the list the way a shell does,
    /// and the matches are *not* recomputed from the completed text:
    /// completing `re` → `related` must not narrow the list down to
    /// the one key that now matches.
    pub(crate) fn properties_complete_key(&mut self) {
        let Some(Overlay::Properties(ref mut p)) = self.overlay else {
            return;
        };
        let Some(edit) = p.editing.as_mut() else {
            return;
        };
        if !matches!(edit.field, PropertyField::Key) || edit.key_matches.is_empty() {
            return;
        }
        let idx = edit.key_match_idx % edit.key_matches.len();
        edit.key = edit.key_matches[idx].clone();
        edit.key_match_idx = (idx + 1) % edit.key_matches.len();
    }

    /// Re-run Insert mode's trigger detection over the value buffer.
    ///
    /// Only `[[` and `#` are honoured here. `((block-ref))`, `/command`,
    /// `@mention` and `:emoji:` have meanings the property dialect
    /// doesn't share, and a popup that fires on a value like `14:00`
    /// or `a/b` would be noise on the one field where a colon and a
    /// slash are ordinary characters.
    fn refresh_property_value_completion(&mut self) {
        let buffer = match &self.overlay {
            Some(Overlay::Properties(p)) => match p.editing.as_ref() {
                Some(edit) if matches!(edit.field, PropertyField::Value) => edit.value.clone(),
                _ => {
                    self.autocomplete = None;
                    return;
                }
            },
            _ => {
                self.autocomplete = None;
                return;
            }
        };
        let chars: Vec<char> = buffer.chars().collect();
        let len = chars.len();
        let (kind, query) = match detect_trigger(&chars, len) {
            Some((kind @ (AutocompleteKind::PageRef | AutocompleteKind::Tag), query)) => {
                (kind, query)
            }
            _ => {
                self.autocomplete = None;
                return;
            }
        };
        let candidates = match kind {
            AutocompleteKind::PageRef => self.candidates_for_pageref(&query),
            _ => self.candidates_for_tag(&query),
        };
        if candidates.is_empty() {
            self.autocomplete = None;
            return;
        }
        self.autocomplete = Some(AutocompleteState {
            kind,
            query,
            candidates,
            selected: 0,
        });
    }

    /// Accept the highlighted page / tag into the value buffer.
    pub(crate) fn accept_property_value_completion(&mut self) {
        let Some(ac) = self.autocomplete.take() else {
            return;
        };
        let Some(choice) = ac.candidates.get(ac.selected).cloned() else {
            return;
        };
        if let Some(Overlay::Properties(ref mut p)) = self.overlay {
            if let Some(edit) = p.editing.as_mut() {
                edit.value = apply_value_completion(&edit.value, ac.kind, &ac.query, &choice);
            }
        }
    }

    /// `Esc` / `q` — close the overlay, taking any popup with it.
    pub(crate) fn close_properties(&mut self) {
        self.autocomplete = None;
        self.overlay = None;
    }

    /// Arm / fire the `dd` chord. Returns nothing; the second `d`
    /// within the overlay deletes.
    pub(crate) fn properties_pending_delete(&mut self) {
        let fire = match &self.overlay {
            Some(Overlay::Properties(p)) => p.pending_delete,
            _ => return,
        };
        if fire {
            self.properties_delete_selected();
            if let Some(Overlay::Properties(ref mut p)) = self.overlay {
                p.pending_delete = false;
            }
        } else if let Some(Overlay::Properties(ref mut p)) = self.overlay {
            p.pending_delete = true;
            p.message = Some("d again to delete".into());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{apply_value_completion, key_completions};
    use crate::state::AutocompleteKind;

    fn known() -> Vec<String> {
        ["related", "icon", "work", "status", "page-ref"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    #[test]
    fn an_empty_query_offers_the_whole_catalogue_in_frequency_order() {
        // The order is the catalogue's, not alphabetical — that ranking
        // is the entire reason `known_keys` returns counts.
        assert_eq!(key_completions(&known(), ""), known());
    }

    #[test]
    fn prefix_matches_come_before_substring_matches() {
        // `re` must offer `related` first, and must still find
        // `page-ref` rather than pretending it doesn't exist.
        assert_eq!(
            key_completions(&known(), "re"),
            vec!["related".to_string(), "page-ref".to_string()]
        );
    }

    #[test]
    fn completion_is_case_insensitive_like_the_parser() {
        // A workspace holding `Remind::` parses as `remind::`; a
        // catalogue the user cannot reach by typing lowercase is a
        // catalogue that looks empty.
        let catalogue = vec!["Remind".to_string()];
        assert_eq!(key_completions(&catalogue, "rem"), vec!["Remind"]);
    }

    #[test]
    fn a_query_matching_nothing_completes_to_nothing() {
        assert!(key_completions(&known(), "zzz").is_empty());
    }

    #[test]
    fn accepting_a_page_replaces_the_open_trigger_not_the_whole_value() {
        // The value can hold text before the `[[`; completing must not
        // eat it. 87% of real values are a ref, but "mostly" is not
        // "always".
        assert_eq!(
            apply_value_completion("see [[Av", AutocompleteKind::PageRef, "Av", "Avelino"),
            "see [[Avelino]]"
        );
    }

    #[test]
    fn accepting_a_tag_closes_it_without_brackets() {
        assert_eq!(
            apply_value_completion("#wo", AutocompleteKind::Tag, "wo", "work"),
            "#work"
        );
    }

    #[test]
    fn a_multibyte_value_is_cut_by_chars_not_bytes() {
        // `ç` is two bytes. Slicing by byte length here would panic
        // mid-frame on a Portuguese page title.
        assert_eq!(
            apply_value_completion("orçamento [[Av", AutocompleteKind::PageRef, "Av", "Avelino"),
            "orçamento [[Avelino]]"
        );
    }

    #[test]
    fn a_trigger_the_field_does_not_honour_leaves_the_value_alone() {
        // `((`, `/`, `@` and `:` are ordinary characters in a property
        // value; the accept path must be a no-op rather than mangling
        // the buffer.
        assert_eq!(
            apply_value_completion("14:00", AutocompleteKind::Emoji, "00", "smile"),
            "14:00"
        );
    }
}
