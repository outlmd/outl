//! Atom classification for the EDN reader: numbers, keywords, symbols
//! and character literals, i.e. everything a token can turn into.

use super::Edn;

pub(super) fn classify_atom(token: &str) -> Result<Edn, String> {
    match token {
        "nil" => return Ok(Edn::Nil),
        "true" => return Ok(Edn::Bool(true)),
        "false" => return Ok(Edn::Bool(false)),
        _ => {}
    }
    let bytes = token.as_bytes();
    let starts_number = bytes[0].is_ascii_digit()
        || (matches!(bytes[0], b'+' | b'-') && bytes.get(1).is_some_and(u8::is_ascii_digit));
    if starts_number {
        return parse_number(token);
    }
    match token.strip_prefix(':') {
        Some("") => Err("a bare `:` is not a keyword".to_string()),
        Some(name) if name.starts_with(':') => {
            Err("auto-resolved keywords (`::`) are not supported".to_string())
        }
        Some(name) => Ok(Edn::Keyword(name.to_string())),
        None => Ok(Edn::Symbol(token.to_string())),
    }
}

fn split_sign(s: &str) -> (bool, &str) {
    match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    }
}

fn all_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

fn skip_digits(b: &[u8], mut i: usize) -> usize {
    while b.get(i).is_some_and(u8::is_ascii_digit) {
        i += 1;
    }
    i
}

/// `digits [. digits] [(e|E) [+-] digits]`, which is stricter than what
/// `f64::from_str` accepts (`1.` and `.5` are not EDN).
fn valid_float(s: &str) -> bool {
    let b = s.as_bytes();
    let mut i = skip_digits(b, 0);
    if i == 0 {
        return false;
    }
    if b.get(i) == Some(&b'.') {
        let frac = skip_digits(b, i + 1);
        if frac == i + 1 {
            return false;
        }
        i = frac;
    }
    if matches!(b.get(i), Some(b'e' | b'E')) {
        let mut j = i + 1;
        if matches!(b.get(j), Some(b'+' | b'-')) {
            j += 1;
        }
        let exp = skip_digits(b, j);
        if exp == j {
            return false;
        }
        i = exp;
    }
    i == b.len()
}

fn parse_number(token: &str) -> Result<Edn, String> {
    let bad = || format!("invalid number `{token}`");
    if let Some(body) = token.strip_suffix('N') {
        let (negative, digits) = split_sign(body);
        if !all_digits(digits) {
            return Err(bad());
        }
        return Ok(big_int(negative, digits));
    }
    let (body, decimal) = match token.strip_suffix('M') {
        Some(body) => (body, true),
        None => (token, false),
    };
    let (negative, rest) = split_sign(body);
    if !decimal && all_digits(rest) {
        return Ok(body
            .parse::<i64>()
            .map_or_else(|_| big_int(negative, rest), Edn::Int));
    }
    if !valid_float(rest) {
        return Err(bad());
    }
    body.parse::<f64>().map(Edn::Float).map_err(|_| bad())
}

fn big_int(negative: bool, digits: &str) -> Edn {
    let sign = if negative { "-" } else { "" };
    Edn::BigInt(format!("{sign}{digits}"))
}

/// `name` is what follows the backslash of a character literal.
pub(super) fn parse_char(name: &str) -> Result<char, String> {
    let mut chars = name.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        return Ok(c);
    }
    match name {
        "space" => return Ok(' '),
        "newline" => return Ok('\n'),
        "tab" => return Ok('\t'),
        "return" => return Ok('\r'),
        "backspace" => return Ok('\u{8}'),
        "formfeed" => return Ok('\u{c}'),
        _ => {}
    }
    let unicode = name
        .strip_prefix('u')
        .filter(|hex| hex.len() == 4 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
        .and_then(|hex| u32::from_str_radix(hex, 16).ok())
        .and_then(char::from_u32);
    unicode.ok_or_else(|| format!("unknown character literal `\\{name}`"))
}
