//! Roam `{{[[pdf]]: url}}` components → outl file links.
//!
//! Roam shows the PDF inline. outl's equivalent for a file is the plain
//! link `[name](url)`, drawn as a file chip that opens it. Rewriting the
//! component before the asset scan lets a local path be copied into
//! `assets/` like any other attachment, while a remote URL stays a
//! hyperlink to the same file.

use crate::report::ImportReport;

/// Rewrite every `{{[[pdf]]: target}}` / `{{pdf: target}}` in `raw` to
/// `[name](target)`. Text inside a `` `code` `` span is left alone, and
/// so is a component whose target can't be a link target as written
/// (empty, or with whitespace or parentheses); the tokenizer then keeps
/// it verbatim and counts it.
pub(super) fn link_pdf_components(raw: &str, report: &mut ImportReport) -> String {
    if !raw.contains("{{") {
        return raw.to_string();
    }
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix('`') {
            if let Some(close) = r.find('`') {
                out.push_str(&rest[..close + 2]);
                rest = &r[close + 1..];
                continue;
            }
        }
        if rest.starts_with("{{") {
            if let Some((link, len)) = pdf_link(rest) {
                out.push_str(&link);
                report.pdfs_linked += 1;
                rest = &rest[len..];
                continue;
            }
        }
        let ch = rest.chars().next().expect("rest is non-empty");
        out.push(ch);
        rest = &rest[ch.len_utf8()..];
    }
    out
}

/// `Some((link, len))` when `s` opens with a pdf component.
fn pdf_link(s: &str) -> Option<(String, usize)> {
    let close = s.find("}}")?;
    let inner = s[2..close].trim();
    let (head, target) = inner.split_once(':')?;
    let head = head.trim();
    if !(head.eq_ignore_ascii_case("[[pdf]]") || head.eq_ignore_ascii_case("pdf")) {
        return None;
    }
    let target = target.trim();
    if target.is_empty() || target.contains(|c: char| c.is_whitespace() || "()<>".contains(c)) {
        return None;
    }
    Some((format!("[{}]({target})", file_name(target)), close + 2))
}

/// The file name a link label should show: the last path segment,
/// percent-decoded (Firebase URLs encode the folders as `%2F`), with
/// the characters that would end a link label dropped.
fn file_name(target: &str) -> String {
    let path = target.split(['?', '#']).next().unwrap_or(target);
    let segment = path.rsplit('/').next().unwrap_or(path);
    let decoded = percent_decode(segment);
    let name: String = decoded
        .rsplit('/')
        .next()
        .unwrap_or(&decoded)
        .chars()
        .filter(|c| !matches!(c, '[' | ']' | '\\'))
        .collect();
    if name.trim().is_empty() {
        "pdf".to_string()
    } else {
        name
    }
}

/// Decode `%XX` escapes; an invalid escape or non-UTF-8 result keeps the
/// input as written.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(raw: &str) -> (String, usize) {
        let mut report = ImportReport::new("roam");
        let out = link_pdf_components(raw, &mut report);
        (out, report.pdfs_linked)
    }

    #[test]
    fn both_spellings_become_a_file_link() {
        let (out, n) = link(
            "see {{[[pdf]]: https://example.com/files/a%20b.pdf}} and {{pdf: ../assets/c.pdf}}",
        );
        assert_eq!(
            out,
            "see [a b.pdf](https://example.com/files/a%20b.pdf) and [c.pdf](../assets/c.pdf)"
        );
        assert_eq!(n, 2);
    }

    #[test]
    fn an_encoded_folder_path_shows_only_the_file_name() {
        let url = "https://storage.example.com/o/imgs%2Fapp%2Fgraph%2Fdoc.pdf?alt=media&token=t";
        let (out, _) = link(&format!("{{{{[[pdf]]: {url}}}}}"));
        assert_eq!(out, format!("[doc.pdf]({url})"));
    }

    #[test]
    fn code_spans_and_odd_targets_stay_verbatim() {
        for raw in [
            "`{{[[pdf]]: https://example.com/a.pdf}}`",
            "{{[[pdf]]: }}",
            "{{[[pdf]]: https://example.com/a b.pdf}}",
            "{{[[pdf]]: https://example.com/a(1).pdf}}",
            "{{[[video]]: https://example.com/v}}",
        ] {
            let (out, n) = link(raw);
            assert_eq!(out, raw);
            assert_eq!(n, 0, "{raw}");
        }
    }
}
