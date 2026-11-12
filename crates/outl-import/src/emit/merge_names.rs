//! What a merged page's other spellings become: `alias::` names, and
//! the warning that says so.

/// What happened to `other`, a title merged into the page `title`.
pub(super) fn merged_warning(other: &str, title: &str, stem: &str) -> String {
    if other == title {
        format!("shares the outl page `{stem}` with [[{title}]], merged")
    } else if alias_entry(other).is_none() {
        format!(
            "shares the outl page `{stem}` with [[{title}]], merged; not kept as an alias \
             (the title has `]]`, which an alias:: name can't hold), links to it still \
             resolve by slug"
        )
    } else {
        format!("shares the outl page `{stem}` with [[{title}]], merged and kept as an alias")
    }
}

/// `title` as one entry of an `alias::` list. The list is split on
/// commas, so a title with one is written `[[title]]`, the bracketed
/// form `outl_md::parse_alias_list` reads back whole. `None` for the
/// one shape that can't be written either way: a comma title that also
/// carries `]]`.
pub(super) fn alias_entry(title: &str) -> Option<String> {
    if !title.contains(',') && !title.starts_with("[[") {
        Some(title.to_string())
    } else if !title.contains("]]") {
        Some(format!("[[{title}]]"))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alias_entries_bracket_only_what_needs_it() {
        assert_eq!(alias_entry("Plan v2").as_deref(), Some("Plan v2"));
        assert_eq!(alias_entry("Plan, v2").as_deref(), Some("[[Plan, v2]]"));
        // Starts like a bracketed entry: bracketed so it reads back whole.
        assert_eq!(alias_entry("[[x").as_deref(), Some("[[[[x]]"));
        assert_eq!(alias_entry("a, [[b]]"), None);
        assert!(merged_warning("a, [[b]]", "A", "a").contains("not kept as an alias"));
    }
}
