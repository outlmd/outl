//! A small EDN reader for Logseq DB exports.
//!
//! Exports can run to tens of megabytes, so the reader walks the source as
//! bytes and only slices `&str` at ASCII delimiters (always a char boundary).
//! Line and column are computed from the byte offset only when an error is
//! built, which keeps the happy path free of position bookkeeping.
//!
//! Only the subset of EDN that an export uses is supported.
//! Reader conditionals, regex literals, metadata and auto-resolved
//! namespaces are rejected with an error instead of being guessed at.

use std::fmt;

mod atoms;
use atoms::{classify_atom, parse_char};

/// Deepest allowed collection / tag nesting; protects the recursive reader
/// from a stack overflow on hostile input.
const MAX_DEPTH: usize = 256;

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Edn {
    Nil,
    Bool(bool),
    Int(i64),
    /// An integer that overflows `i64`, or one written with the `N` suffix.
    /// Digits with an optional leading `-`, suffix stripped.
    BigInt(String),
    /// Also covers `M` suffixed numbers and `##Inf` / `##-Inf` / `##NaN`.
    Float(f64),
    #[allow(dead_code)]
    Char(char),
    Str(String),
    Symbol(String),
    /// Without the leading `:`.
    Keyword(String),
    List(Vec<Edn>),
    Vector(Vec<Edn>),
    Set(Vec<Edn>),
    /// Insertion order is kept and duplicate keys are not an error.
    Map(Vec<(Edn, Edn)>),
    Tagged(String, Box<Edn>),
}

impl Edn {
    pub(super) fn as_int(&self) -> Option<i64> {
        match self {
            Edn::Int(n) => Some(*n),
            _ => None,
        }
    }

    pub(super) fn as_str(&self) -> Option<&str> {
        match self {
            Edn::Str(s) => Some(s),
            _ => None,
        }
    }

    pub(super) fn as_keyword(&self) -> Option<&str> {
        match self {
            Edn::Keyword(k) => Some(k),
            _ => None,
        }
    }

    pub(super) fn as_seq(&self) -> Option<&[Edn]> {
        match self {
            Edn::Vector(items) | Edn::List(items) | Edn::Set(items) => Some(items),
            _ => None,
        }
    }

    /// First entry of a map whose key is the keyword `keyword` (no colon).
    pub(super) fn map_get(&self, keyword: &str) -> Option<&Edn> {
        match self {
            Edn::Map(entries) => entries
                .iter()
                .find_map(|(k, v)| (k.as_keyword() == Some(keyword)).then_some(v)),
            _ => None,
        }
    }
}

/// A parse failure; `line` and `column` are 1-based, the column counts chars.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct EdnError {
    pub line: usize,
    pub column: usize,
    pub message: String,
}

impl fmt::Display for EdnError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "line {}, column {}: {}",
            self.line, self.column, self.message
        )
    }
}

impl std::error::Error for EdnError {}

/// Parses exactly one top-level form; trailing whitespace and comments are
/// allowed, anything else after the form is an error.
pub(super) fn parse(src: &str) -> Result<Edn, EdnError> {
    let mut reader = Reader {
        src,
        bytes: src.as_bytes(),
        pos: 0,
    };
    reader.skip_trivia(0)?;
    if reader.pos >= reader.bytes.len() {
        return Err(reader.err(reader.pos, "no form to read"));
    }
    let value = reader.read_value(0)?;
    reader.skip_trivia(0)?;
    match reader.bytes.get(reader.pos) {
        None => Ok(value),
        Some(b) if is_closer(*b) => Err(reader.unexpected_closer(reader.pos)),
        Some(_) => Err(reader.err(reader.pos, "unexpected input after the top-level form")),
    }
}

struct Reader<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
}

fn is_closer(b: u8) -> bool {
    matches!(b, b')' | b']' | b'}')
}

fn is_blank(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c | b',')
}

fn is_delimiter(b: u8) -> bool {
    is_blank(b) || matches!(b, b'(' | b')' | b'[' | b']' | b'{' | b'}' | b'"' | b';')
}

impl<'a> Reader<'a> {
    fn err(&self, offset: usize, message: impl Into<String>) -> EdnError {
        let offset = offset.min(self.bytes.len());
        let before = &self.bytes[..offset];
        let line = 1 + before.iter().filter(|&&b| b == b'\n').count();
        let line_start = before
            .iter()
            .rposition(|&b| b == b'\n')
            .map_or(0, |i| i + 1);
        let column = 1 + String::from_utf8_lossy(&self.bytes[line_start..offset])
            .chars()
            .count();
        EdnError {
            line,
            column,
            message: message.into(),
        }
    }

    fn unexpected_closer(&self, offset: usize) -> EdnError {
        let found = self.bytes.get(offset).map_or('?', |&b| b as char);
        self.err(offset, format!("unexpected closing delimiter `{found}`"))
    }

    /// Rejects a composite form that would nest past the limit.
    fn enter(&self, depth: usize, offset: usize) -> Result<(), EdnError> {
        if depth >= MAX_DEPTH {
            return Err(self.err(offset, format!("nesting deeper than {MAX_DEPTH} levels")));
        }
        Ok(())
    }

    fn skip_blank(&mut self) {
        while let Some(&b) = self.bytes.get(self.pos) {
            if is_blank(b) {
                self.pos += 1;
            } else if b == b';' {
                let rest = &self.bytes[self.pos..];
                self.pos += rest.iter().position(|&c| c == b'\n').unwrap_or(rest.len());
            } else {
                break;
            }
        }
    }

    /// Skips whitespace, comments and `#_` discards, so `read_value` always
    /// starts on a real form (or on the end of input / a closing delimiter).
    fn skip_trivia(&mut self, depth: usize) -> Result<(), EdnError> {
        loop {
            self.skip_blank();
            if !self.bytes[self.pos..].starts_with(b"#_") {
                return Ok(());
            }
            let start = self.pos;
            self.enter(depth, start)?;
            self.pos += 2;
            // A chain like `#_ #_ a b` discards two forms, which the nested
            // call handles by consuming the inner discard first.
            self.skip_trivia(depth + 1)?;
            match self.bytes.get(self.pos) {
                None => return Err(self.err(start, "`#_` has no form to discard")),
                Some(b) if is_closer(*b) => {
                    return Err(self.err(start, "`#_` has no form to discard"));
                }
                Some(_) => {
                    self.read_value(depth)?;
                }
            }
        }
    }

    /// Reads one form; the caller has already run `skip_trivia`.
    fn read_value(&mut self, depth: usize) -> Result<Edn, EdnError> {
        let start = self.pos;
        let Some(&b) = self.bytes.get(start) else {
            return Err(self.err(start, "unexpected end of input"));
        };
        match b {
            b'"' => Ok(Edn::Str(self.read_string()?)),
            b'[' => {
                self.enter(depth, start)?;
                self.pos += 1;
                Ok(Edn::Vector(self.read_seq(b']', start, depth)?))
            }
            b'(' => {
                self.enter(depth, start)?;
                self.pos += 1;
                Ok(Edn::List(self.read_seq(b')', start, depth)?))
            }
            b'{' => {
                self.enter(depth, start)?;
                self.pos += 1;
                Ok(Edn::Map(self.read_map(start, depth, None)?))
            }
            b')' | b']' | b'}' => Err(self.unexpected_closer(start)),
            b'#' => self.read_dispatch(depth),
            b'^' => Err(self.err(start, "metadata (`^`) is not supported")),
            b'\\' => self.read_char(),
            _ => self.read_atom(),
        }
    }

    /// Reads forms up to `close`; `open` is where the collection started, so
    /// an unterminated collection is reported at its opening delimiter.
    fn read_seq(&mut self, close: u8, open: usize, depth: usize) -> Result<Vec<Edn>, EdnError> {
        let mut items = Vec::new();
        loop {
            self.skip_trivia(depth + 1)?;
            match self.bytes.get(self.pos) {
                None => {
                    return Err(self.err(
                        open,
                        format!("unterminated collection, expected `{}`", close as char),
                    ));
                }
                Some(&b) if b == close => {
                    self.pos += 1;
                    return Ok(items);
                }
                Some(_) => items.push(self.read_value(depth + 1)?),
            }
        }
    }

    /// `ns` is the namespace of a `#:ns{...}` map; it qualifies keyword keys.
    fn read_map(
        &mut self,
        open: usize,
        depth: usize,
        ns: Option<&str>,
    ) -> Result<Vec<(Edn, Edn)>, EdnError> {
        let items = self.read_seq(b'}', open, depth)?;
        if items.len() % 2 != 0 {
            return Err(self.err(open, "map literal must contain an even number of forms"));
        }
        let mut entries = Vec::with_capacity(items.len() / 2);
        let mut iter = items.into_iter();
        while let (Some(key), Some(value)) = (iter.next(), iter.next()) {
            let key = match ns {
                Some(ns) => qualify_key(key, ns),
                None => key,
            };
            entries.push((key, value));
        }
        Ok(entries)
    }

    fn read_dispatch(&mut self, depth: usize) -> Result<Edn, EdnError> {
        let start = self.pos;
        match self.bytes.get(start + 1) {
            Some(b'{') => {
                self.enter(depth, start)?;
                self.pos += 2;
                Ok(Edn::Set(self.read_seq(b'}', start, depth)?))
            }
            Some(b':') => self.read_namespaced_map(depth),
            Some(b'#') => self.read_symbolic(),
            Some(b'?') => Err(self.err(start, "reader conditionals (`#?`) are not supported")),
            Some(b'"') => Err(self.err(start, "regex literals (`#\"`) are not supported")),
            Some(b'(') => Err(self.err(start, "anonymous function literals are not supported")),
            Some(&b) if !is_delimiter(b) => self.read_tagged(depth),
            _ => Err(self.err(start, "invalid `#` dispatch")),
        }
    }

    fn read_tagged(&mut self, depth: usize) -> Result<Edn, EdnError> {
        let start = self.pos;
        self.enter(depth, start)?;
        self.pos += 1;
        let tag = self.scan_token();
        if !tag.starts_with(|c: char| c.is_alphabetic()) {
            return Err(self.err(start, format!("invalid tag `#{tag}`")));
        }
        self.skip_trivia(depth + 1)?;
        match self.bytes.get(self.pos) {
            None => Err(self.err(start, format!("tag `#{tag}` has no value"))),
            Some(b) if is_closer(*b) => Err(self.err(start, format!("tag `#{tag}` has no value"))),
            Some(_) => {
                let value = self.read_value(depth + 1)?;
                Ok(Edn::Tagged(tag.to_string(), Box::new(value)))
            }
        }
    }

    fn read_namespaced_map(&mut self, depth: usize) -> Result<Edn, EdnError> {
        let start = self.pos;
        self.enter(depth, start)?;
        self.pos += 2;
        if self.bytes.get(self.pos) == Some(&b':') {
            return Err(self.err(
                start,
                "auto-resolved namespaced maps (`#::`) are not supported",
            ));
        }
        let ns = self.scan_token();
        if ns.is_empty() {
            return Err(self.err(start, "missing namespace after `#:`"));
        }
        self.skip_trivia(depth + 1)?;
        if self.bytes.get(self.pos) != Some(&b'{') {
            return Err(self.err(self.pos, "expected `{` after the map namespace"));
        }
        self.pos += 1;
        Ok(Edn::Map(self.read_map(start, depth, Some(ns))?))
    }

    fn read_symbolic(&mut self) -> Result<Edn, EdnError> {
        let start = self.pos;
        self.pos += 2;
        match self.scan_token() {
            "Inf" => Ok(Edn::Float(f64::INFINITY)),
            "-Inf" => Ok(Edn::Float(f64::NEG_INFINITY)),
            "NaN" => Ok(Edn::Float(f64::NAN)),
            other => Err(self.err(start, format!("unknown symbolic value `##{other}`"))),
        }
    }

    /// Reads up to the next delimiter. Delimiters are ASCII, so the slice
    /// always lands on char boundaries.
    fn scan_token(&mut self) -> &'a str {
        let src = self.src;
        let start = self.pos;
        let rest = &self.bytes[start..];
        let len = rest
            .iter()
            .position(|&b| is_delimiter(b))
            .unwrap_or(rest.len());
        self.pos = start + len;
        &src[start..self.pos]
    }

    fn read_atom(&mut self) -> Result<Edn, EdnError> {
        let start = self.pos;
        let token = self.scan_token();
        if token.is_empty() {
            return Err(self.err(start, "unexpected character"));
        }
        classify_atom(token).map_err(|msg| self.err(start, msg))
    }

    fn read_char(&mut self) -> Result<Edn, EdnError> {
        let start = self.pos;
        let src = self.src;
        let Some(first) = src[start + 1..].chars().next() else {
            return Err(self.err(start, "incomplete character literal"));
        };
        // The first char is taken even when it is a delimiter (`\(`, `\"`).
        self.pos = start + 1 + first.len_utf8();
        self.scan_token();
        parse_char(&src[start + 1..self.pos])
            .map(Edn::Char)
            .map_err(|msg| self.err(start, msg))
    }

    fn read_string(&mut self) -> Result<String, EdnError> {
        let open = self.pos;
        self.pos += 1;
        let mut out = String::new();
        loop {
            let run = &self.bytes[self.pos..];
            let len = run
                .iter()
                .position(|&b| b == b'"' || b == b'\\')
                .unwrap_or(run.len());
            out.push_str(&self.src[self.pos..self.pos + len]);
            self.pos += len;
            match self.bytes.get(self.pos) {
                None => return Err(self.err(open, "unterminated string")),
                Some(b'"') => {
                    self.pos += 1;
                    return Ok(out);
                }
                Some(_) => {
                    let c = self.read_escape(open)?;
                    out.push(c);
                }
            }
        }
    }

    /// Reads one escape with `self.pos` on the backslash.
    fn read_escape(&mut self, string_open: usize) -> Result<char, EdnError> {
        let esc = self.pos;
        let Some(&b) = self.bytes.get(esc + 1) else {
            return Err(self.err(string_open, "unterminated string"));
        };
        let simple = match b {
            b't' => Some('\t'),
            b'r' => Some('\r'),
            b'n' => Some('\n'),
            b'\\' => Some('\\'),
            b'"' => Some('"'),
            b'b' => Some('\u{8}'),
            b'f' => Some('\u{c}'),
            _ => None,
        };
        if let Some(c) = simple {
            self.pos += 2;
            return Ok(c);
        }
        if b == b'u' {
            return self.read_unicode_escape(esc);
        }
        let shown = self.src[esc + 1..].chars().next().unwrap_or('?');
        Err(self.err(esc, format!("invalid escape sequence `\\{shown}`")))
    }

    fn hex4(&self, at: usize) -> Option<u32> {
        let digits = self.bytes.get(at..at + 4)?;
        digits.iter().try_fold(0u32, |acc, &b| {
            Some(acc * 16 + (b as char).to_digit(16)?)
        })
    }

    /// Reads `\uXXXX`, combining a UTF-16 surrogate pair into one char.
    fn read_unicode_escape(&mut self, esc: usize) -> Result<char, EdnError> {
        let bad_hex = |r: &Self| r.err(esc, "invalid `\\u` escape: expected four hex digits");
        let hi = self.hex4(esc + 2).ok_or_else(|| bad_hex(self))?;
        self.pos = esc + 6;
        match hi {
            0xD800..=0xDBFF => {
                let is_escape = self.bytes[self.pos..].starts_with(b"\\u");
                let lo = if is_escape { self.hex4(self.pos + 2) } else { None };
                let Some(lo @ 0xDC00..=0xDFFF) = lo else {
                    return Err(self.err(esc, "unpaired high surrogate in `\\u` escape"));
                };
                self.pos += 6;
                let code = 0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00);
                char::from_u32(code).ok_or_else(|| self.err(esc, "invalid surrogate pair"))
            }
            0xDC00..=0xDFFF => Err(self.err(esc, "unpaired low surrogate in `\\u` escape")),
            _ => char::from_u32(hi).ok_or_else(|| self.err(esc, "invalid `\\u` escape")),
        }
    }
}

/// Applies a `#:ns{...}` namespace to a key: unqualified keywords gain `ns/`,
/// `:_/x` drops to `x`, qualified keywords and non-keywords stay as they are.
fn qualify_key(key: Edn, ns: &str) -> Edn {
    let Edn::Keyword(name) = key else {
        return key;
    };
    if let Some(bare) = name.strip_prefix("_/") {
        Edn::Keyword(bare.to_string())
    } else if name.contains('/') {
        Edn::Keyword(name)
    } else {
        Edn::Keyword(format!("{ns}/{name}"))
    }
}

#[cfg(test)]
#[path = "edn_tests.rs"]
mod tests;
