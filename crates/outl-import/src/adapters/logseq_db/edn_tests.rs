use super::*;

fn ok(src: &str) -> Edn {
    parse(src).unwrap_or_else(|e| panic!("`{src}` should parse: {e}"))
}

fn fails(src: &str) -> EdnError {
    match parse(src) {
        Ok(v) => panic!("`{src}` should fail, got {v:?}"),
        Err(e) => e,
    }
}

/// Asserts the error position and that the message mentions `needle`.
fn assert_err(src: &str, line: usize, column: usize, needle: &str) {
    let e = fails(src);
    assert_eq!((e.line, e.column), (line, column), "{src:?}: {e}");
    assert!(
        e.message.contains(needle),
        "{src:?}: message {:?} lacks {needle:?}",
        e.message
    );
}

fn kw(s: &str) -> Edn {
    Edn::Keyword(s.to_string())
}

fn int(n: i64) -> Edn {
    Edn::Int(n)
}

#[test]
fn literals_nil_and_booleans() {
    assert_eq!(ok("nil"), Edn::Nil);
    assert_eq!(ok("true"), Edn::Bool(true));
    assert_eq!(ok("false"), Edn::Bool(false));
}

#[test]
fn integers() {
    assert_eq!(ok("42"), int(42));
    assert_eq!(ok("-7"), int(-7));
    assert_eq!(ok("+3"), int(3));
    assert_eq!(ok("0"), int(0));
    assert_eq!(ok("9223372036854775807"), int(i64::MAX));
    assert_eq!(ok("-9223372036854775808"), int(i64::MIN));
}

#[test]
fn big_integers() {
    assert_eq!(
        ok("9223372036854775808"),
        Edn::BigInt("9223372036854775808".into())
    );
    assert_eq!(
        ok("-9223372036854775809"),
        Edn::BigInt("-9223372036854775809".into())
    );
    assert_eq!(ok("42N"), Edn::BigInt("42".into()));
    assert_eq!(ok("-5N"), Edn::BigInt("-5".into()));
    assert_eq!(ok("+5N"), Edn::BigInt("5".into()));
    assert_eq!(
        ok("123456789012345678901234567890N"),
        Edn::BigInt("123456789012345678901234567890".into())
    );
}

#[test]
fn floats() {
    assert_eq!(ok("1.5"), Edn::Float(1.5));
    assert_eq!(ok("-0.5"), Edn::Float(-0.5));
    assert_eq!(ok("+2.25"), Edn::Float(2.25));
    assert_eq!(ok("1e10"), Edn::Float(1e10));
    assert_eq!(ok("2.5E-3"), Edn::Float(2.5e-3));
    assert_eq!(ok("1E+2"), Edn::Float(100.0));
    assert_eq!(ok("1.5M"), Edn::Float(1.5));
    assert_eq!(ok("42M"), Edn::Float(42.0));
    assert_eq!(ok("-0.5M"), Edn::Float(-0.5));
}

#[test]
fn symbolic_floats() {
    assert_eq!(ok("##Inf"), Edn::Float(f64::INFINITY));
    assert_eq!(ok("##-Inf"), Edn::Float(f64::NEG_INFINITY));
    match ok("##NaN") {
        Edn::Float(f) => assert!(f.is_nan()),
        other => panic!("expected a float, got {other:?}"),
    }
}

#[test]
fn characters() {
    assert_eq!(ok(r"\a"), Edn::Char('a'));
    assert_eq!(ok(r"\space"), Edn::Char(' '));
    assert_eq!(ok(r"\newline"), Edn::Char('\n'));
    assert_eq!(ok(r"\tab"), Edn::Char('\t'));
    assert_eq!(ok(r"\return"), Edn::Char('\r'));
    assert_eq!(ok(r"\backspace"), Edn::Char('\u{8}'));
    assert_eq!(ok(r"\formfeed"), Edn::Char('\u{c}'));
    assert_eq!(ok(r"\u0041"), Edn::Char('A'));
    assert_eq!(ok(r"\u00e9"), Edn::Char('é'));
    assert_eq!(ok("\\é"), Edn::Char('é'));
}

#[test]
fn characters_that_are_delimiters() {
    assert_eq!(ok(r"\("), Edn::Char('('));
    assert_eq!(ok(r#"\""#), Edn::Char('"'));
    assert_eq!(ok(r"[\] \,]"), Edn::Vector(vec![Edn::Char(']'), Edn::Char(',')]));
}

#[test]
fn strings_plain_and_escapes() {
    assert_eq!(ok(r#""""#), Edn::Str(String::new()));
    assert_eq!(ok(r#""hello world""#), Edn::Str("hello world".into()));
    assert_eq!(
        ok(r#""a\tb\rc\nd\\e\"f\bg\fh""#),
        Edn::Str("a\tb\rc\nd\\e\"f\u{8}g\u{c}h".into())
    );
    assert_eq!(ok("\"multi\nline\""), Edn::Str("multi\nline".into()));
    assert_eq!(ok("\"héllo ✓\""), Edn::Str("héllo ✓".into()));
}

#[test]
fn string_unicode_escapes() {
    assert_eq!(ok(r#""\u0041\u00e9""#), Edn::Str("Aé".into()));
    assert_eq!(ok(r#""\uD83D\uDE00""#), Edn::Str("\u{1F600}".into()));
    assert_eq!(ok(r#""x\ud83d\ude00y""#), Edn::Str("x\u{1F600}y".into()));
}

#[test]
fn keywords() {
    assert_eq!(ok(":a"), kw("a"));
    assert_eq!(
        ok(":logseq.property/status.todo"),
        kw("logseq.property/status.todo")
    );
    assert_eq!(ok(":block/collapsed?"), kw("block/collapsed?"));
    assert_eq!(ok(":save!"), kw("save!"));
    assert_eq!(ok(":a*"), kw("a*"));
    assert_eq!(ok(":a-b_c+d"), kw("a-b_c+d"));
    assert_eq!(ok(":v2/x1"), kw("v2/x1"));
    assert_eq!(ok(":1a"), kw("1a"));
}

#[test]
fn symbols() {
    assert_eq!(ok("foo"), Edn::Symbol("foo".into()));
    assert_eq!(ok("ns/foo"), Edn::Symbol("ns/foo".into()));
    assert_eq!(ok("+"), Edn::Symbol("+".into()));
    assert_eq!(ok("-"), Edn::Symbol("-".into()));
    assert_eq!(ok("-foo"), Edn::Symbol("-foo".into()));
    assert_eq!(ok("a.b/c?"), Edn::Symbol("a.b/c?".into()));
    assert_eq!(ok("nilly"), Edn::Symbol("nilly".into()));
}

#[test]
fn collections() {
    assert_eq!(ok("[]"), Edn::Vector(vec![]));
    assert_eq!(ok("()"), Edn::List(vec![]));
    assert_eq!(ok("{}"), Edn::Map(vec![]));
    assert_eq!(ok("#{}"), Edn::Set(vec![]));
    assert_eq!(ok("[1 2 3]"), Edn::Vector(vec![int(1), int(2), int(3)]));
    assert_eq!(ok("(1 :a \"x\")"), Edn::List(vec![int(1), kw("a"), Edn::Str("x".into())]));
    assert_eq!(ok("#{1 2}"), Edn::Set(vec![int(1), int(2)]));
    assert_eq!(ok("{:a 1 :b 2}"), Edn::Map(vec![(kw("a"), int(1)), (kw("b"), int(2))]));
}

#[test]
fn delimiters_end_tokens_without_whitespace() {
    assert_eq!(ok("[1[2]3]"), Edn::Vector(vec![int(1), Edn::Vector(vec![int(2)]), int(3)]));
    assert_eq!(ok("[:a\"x\":b]"), Edn::Vector(vec![kw("a"), Edn::Str("x".into()), kw("b")]));
    assert_eq!(ok("{:a 1}"), ok("{:a,1}"));
}

#[test]
fn map_keeps_order_and_duplicates() {
    let m = ok("{:b 1 :a 2 :b 3}");
    assert_eq!(
        m,
        Edn::Map(vec![(kw("b"), int(1)), (kw("a"), int(2)), (kw("b"), int(3))])
    );
    assert_eq!(m.map_get("b"), Some(&int(1)));
}

#[test]
fn nested_structure() {
    let v = ok("{:a [1 {:b (2 #{3})}] :c {:d {:e nil}}}");
    let a = v.map_get("a").and_then(Edn::as_seq).expect("a is a seq");
    assert_eq!(a[0], int(1));
    let list = a[1].map_get("b").and_then(Edn::as_seq).expect("b is a seq");
    assert_eq!(list[0], int(2));
    assert_eq!(list[1].as_seq(), Some(&[int(3)][..]));
    let e = v
        .map_get("c")
        .and_then(|c| c.map_get("d"))
        .and_then(|d| d.map_get("e"));
    assert_eq!(e, Some(&Edn::Nil));
}

#[test]
fn commas_are_whitespace() {
    assert_eq!(ok("[1,2,,3,]"), Edn::Vector(vec![int(1), int(2), int(3)]));
    assert_eq!(ok(",,{,:a,1,},,"), Edn::Map(vec![(kw("a"), int(1))]));
}

#[test]
fn line_comments() {
    assert_eq!(ok("; leading\n42 ; trailing"), int(42));
    assert_eq!(ok("[1 ; one\n 2 ;two\n]"), Edn::Vector(vec![int(1), int(2)]));
    assert_eq!(ok("42 ; no newline at the end"), int(42));
    assert_eq!(ok("[1 ;]\n 2]"), Edn::Vector(vec![int(1), int(2)]));
    assert_eq!(ok("\"a;b\""), Edn::Str("a;b".into()));
}

#[test]
fn discard_forms() {
    assert_eq!(ok("[1 #_2 3]"), Edn::Vector(vec![int(1), int(3)]));
    assert_eq!(ok("[#_ 1]"), Edn::Vector(vec![]));
    assert_eq!(ok("#_ 1 2"), int(2));
    assert_eq!(ok("1 #_ 2"), int(1));
    assert_eq!(ok("1 #_ [2 #_3] ; done"), int(1));
    assert_eq!(ok("[1 #_ [2 #_ 3] 4]"), Edn::Vector(vec![int(1), int(4)]));
    assert_eq!(ok("{:a #_ :x 1}"), Edn::Map(vec![(kw("a"), int(1))]));
    assert_eq!(ok("{#_ :x :a 1}"), Edn::Map(vec![(kw("a"), int(1))]));
    assert_eq!(ok("#{1 #_2}"), Edn::Set(vec![int(1)]));
    assert_eq!(ok("#_#uuid \"x\" 5"), int(5));
}

#[test]
fn stacked_discards_consume_one_form_each() {
    assert_eq!(ok("#_ #_ 1 2 3"), int(3));
    assert_eq!(ok("[#_ #_ 1 2 3 4]"), Edn::Vector(vec![int(3), int(4)]));
}

#[test]
fn tagged_literals() {
    assert_eq!(
        ok("#uuid \"00000002-1128-4a8a-8e6c-2f1f6f5a1c01\""),
        Edn::Tagged(
            "uuid".into(),
            Box::new(Edn::Str("00000002-1128-4a8a-8e6c-2f1f6f5a1c01".into()))
        )
    );
    assert_eq!(
        ok("#inst \"2024-01-02T03:04:05.000-00:00\""),
        Edn::Tagged(
            "inst".into(),
            Box::new(Edn::Str("2024-01-02T03:04:05.000-00:00".into()))
        )
    );
    assert_eq!(
        ok("#my.app/thing {:a 1}"),
        Edn::Tagged("my.app/thing".into(), Box::new(Edn::Map(vec![(kw("a"), int(1))])))
    );
    assert_eq!(
        ok("#a #b 1"),
        Edn::Tagged("a".into(), Box::new(Edn::Tagged("b".into(), Box::new(int(1)))))
    );
    assert_eq!(
        ok("[#t 1 #t 2]"),
        Edn::Vector(vec![
            Edn::Tagged("t".into(), Box::new(int(1))),
            Edn::Tagged("t".into(), Box::new(int(2))),
        ])
    );
}

#[test]
fn namespaced_maps() {
    assert_eq!(
        ok("#:ns{:a 1 :b/c 2 other 3}"),
        Edn::Map(vec![
            (kw("ns/a"), int(1)),
            (kw("b/c"), int(2)),
            (Edn::Symbol("other".into()), int(3)),
        ])
    );
    assert_eq!(ok("#:ns{:_/x 1}"), Edn::Map(vec![(kw("x"), int(1))]));
    assert_eq!(ok("#:ns {:a 1}"), Edn::Map(vec![(kw("ns/a"), int(1))]));
    assert_eq!(ok("#:a.b{}"), Edn::Map(vec![]));
    // Only keys are qualified, values and nested maps are left alone.
    assert_eq!(
        ok("#:ns{:a {:b :c}}"),
        Edn::Map(vec![(kw("ns/a"), Edn::Map(vec![(kw("b"), kw("c"))]))])
    );
    assert_eq!(
        ok("#:ns{\"s\" 1 2 3}"),
        Edn::Map(vec![(Edn::Str("s".into()), int(1)), (int(2), int(3))])
    );
}

#[test]
fn accessors() {
    assert_eq!(int(5).as_int(), Some(5));
    assert_eq!(Edn::Str("5".into()).as_int(), None);
    assert_eq!(Edn::Str("x".into()).as_str(), Some("x"));
    assert_eq!(kw("x").as_str(), None);
    assert_eq!(kw("x").as_keyword(), Some("x"));
    assert_eq!(Edn::Symbol("x".into()).as_keyword(), None);
    assert_eq!(Edn::Vector(vec![int(1)]).as_seq(), Some(&[int(1)][..]));
    assert_eq!(Edn::List(vec![int(1)]).as_seq(), Some(&[int(1)][..]));
    assert_eq!(Edn::Set(vec![int(1)]).as_seq(), Some(&[int(1)][..]));
    assert_eq!(Edn::Map(vec![]).as_seq(), None);
    let m = ok("{:a 1 \"b\" 2}");
    assert_eq!(m.map_get("a"), Some(&int(1)));
    assert_eq!(m.map_get("b"), None);
    assert_eq!(m.map_get("zzz"), None);
    assert_eq!(Edn::Vector(vec![]).map_get("a"), None);
}

#[test]
fn realistic_logseq_export() {
    let src = r#"{:logseq.db.sqlite.export/schema-version {:major 65, :minor 34}, :logseq.db.sqlite.export/graph-format :datoms, :datoms [[1 :db/ident :logseq.class/Page] [2 :block/uuid #uuid "00000002-1128-4a8a-8e6c-2f1f6f5a1c01"] [3 :block/title "a \"quoted\" \\ title\nline2"] [4 :block/collapsed? true] [5 :block/created-at 1700000000000]], :logseq.db.sqlite.export/export-type :graph}"#;
    let root = ok(src);

    let version = root
        .map_get("logseq.db.sqlite.export/schema-version")
        .expect("schema-version");
    assert_eq!(version.map_get("major").and_then(Edn::as_int), Some(65));
    assert_eq!(version.map_get("minor").and_then(Edn::as_int), Some(34));
    assert_eq!(
        root.map_get("logseq.db.sqlite.export/graph-format")
            .and_then(Edn::as_keyword),
        Some("datoms")
    );
    assert_eq!(
        root.map_get("logseq.db.sqlite.export/export-type")
            .and_then(Edn::as_keyword),
        Some("graph")
    );

    let datoms = root.map_get("datoms").and_then(Edn::as_seq).expect("datoms");
    assert_eq!(datoms.len(), 5);
    let row = |i: usize| datoms[i].as_seq().expect("datom row");
    assert_eq!(row(0)[2].as_keyword(), Some("logseq.class/Page"));
    assert_eq!(
        row(1)[2],
        Edn::Tagged(
            "uuid".into(),
            Box::new(Edn::Str("00000002-1128-4a8a-8e6c-2f1f6f5a1c01".into()))
        )
    );
    assert_eq!(row(2)[2].as_str(), Some("a \"quoted\" \\ title\nline2"));
    assert_eq!(row(3)[2], Edn::Bool(true));
    assert_eq!(row(4)[2].as_int(), Some(1_700_000_000_000));
}

#[test]
fn big_input_parses() {
    let mut src = String::from("[");
    for i in 0..200_000 {
        src.push_str(&format!("[{i} :a/b \"s{i}\"] "));
    }
    src.push(']');
    let v = ok(&src);
    assert_eq!(v.as_seq().map(<[Edn]>::len), Some(200_000));
}

// Errors. Each case asserts the exact 1-based line / column.

#[test]
fn error_empty_input() {
    assert_err("", 1, 1, "no form");
    assert_err("  \n ", 2, 2, "no form");
    assert_err("; only a comment", 1, 17, "no form");
    assert_err("#_ 1", 1, 5, "no form");
}

#[test]
fn error_unterminated_string() {
    assert_err("\"abc", 1, 1, "unterminated string");
    assert_err("[1\n  \"abc", 2, 3, "unterminated string");
    assert_err("\"abc\\", 1, 1, "unterminated string");
}

#[test]
fn error_unterminated_collection() {
    assert_err("(1 2", 1, 1, "unterminated");
    assert_err("{:a 1\n :b [1 2", 2, 5, "unterminated");
    assert_err("[1 2\n  #{3", 2, 3, "unterminated");
    assert_err("{:a 1", 1, 1, "unterminated");
}

#[test]
fn error_unexpected_closer() {
    assert_err(")", 1, 1, "unexpected closing delimiter `)`");
    assert_err("[1 2\n 3)", 2, 3, "unexpected closing delimiter `)`");
    assert_err("1 ]", 1, 3, "unexpected closing delimiter `]`");
    assert_err("{:a 1]", 1, 6, "unexpected closing delimiter `]`");
    assert_err("[1\n}", 2, 1, "unexpected closing delimiter `}`");
}

#[test]
fn error_bad_escape() {
    assert_err("\"ab\n\\q\"", 2, 1, "invalid escape sequence `\\q`");
    assert_err("[\"x\" \"a\\'b\"]", 1, 8, "invalid escape");
    assert_err("\"\\u12G4\"", 1, 2, "four hex digits");
    assert_err("\"\\u12\"", 1, 2, "four hex digits");
    assert_err("\"é\\x\"", 1, 3, "invalid escape");
}

#[test]
fn error_surrogates() {
    assert_err("\"\\uD83D\"", 1, 2, "unpaired high surrogate");
    assert_err("\"\\uD83Dx\"", 1, 2, "unpaired high surrogate");
    assert_err("\"\\uD83D\\u0041\"", 1, 2, "unpaired high surrogate");
    assert_err("\"ab\\uDE00\"", 1, 4, "unpaired low surrogate");
}

#[test]
fn error_bad_number() {
    assert_err("[1\n 12abc]", 2, 2, "invalid number `12abc`");
    assert_err("1.", 1, 1, "invalid number");
    assert_err("1e", 1, 1, "invalid number");
    assert_err("1.2.3", 1, 1, "invalid number");
    assert_err("0x10", 1, 1, "invalid number");
    assert_err("1/2", 1, 1, "invalid number");
    assert_err("[1 -5x]", 1, 4, "invalid number `-5x`");
    assert_err("1.5N", 1, 1, "invalid number");
    assert_err("1e5N", 1, 1, "invalid number");
}

#[test]
fn error_odd_map() {
    assert_err("\n{:a 1 :b}", 2, 1, "even number");
    assert_err("[1\n  {:a}]", 2, 3, "even number");
    assert_err("#:ns{:a}", 1, 1, "even number");
    assert_err("{:a #_ 1}", 1, 1, "even number");
}

#[test]
fn error_dangling_discard() {
    assert_err("[1 #_]", 1, 4, "no form to discard");
    assert_err("1 #_", 1, 3, "no form to discard");
    assert_err("#_", 1, 1, "no form to discard");
    assert_err("{:a 1\n  #_}", 2, 3, "no form to discard");
    assert_err("#_ #_ 1", 1, 1, "no form to discard");
}

#[test]
fn error_trailing_garbage() {
    assert_err("1 2", 1, 3, "unexpected input");
    assert_err("{}\n  foo", 2, 3, "unexpected input");
    assert_err("[1] [2]", 1, 5, "unexpected input");
    assert_err("\"é\" x", 1, 5, "unexpected input");
    assert_err("1 ; ok\n#_ 2\n:x", 3, 1, "unexpected input");
}

#[test]
fn error_bad_keyword() {
    assert_err(":", 1, 1, "bare `:`");
    assert_err("::foo", 1, 1, "auto-resolved keywords");
    assert_err("[1 :]", 1, 4, "bare `:`");
    assert_err("[1\n ::a]", 2, 2, "auto-resolved keywords");
}

#[test]
fn error_unsupported_dispatch() {
    assert_err("#::{:a 1}", 1, 1, "auto-resolved namespaced maps");
    assert_err("#::ns{:a 1}", 1, 1, "auto-resolved namespaced maps");
    assert_err("[1\n #?(:clj 1)]", 2, 2, "reader conditionals");
    assert_err("#\"re\"", 1, 1, "regex");
    assert_err("^:meta {}", 1, 1, "metadata");
    assert_err("[1 ^{:a 1} x]", 1, 4, "metadata");
    assert_err("#(inc %)", 1, 1, "anonymous function");
    assert_err("# 1", 1, 1, "invalid `#` dispatch");
    assert_err("#", 1, 1, "invalid `#` dispatch");
}

#[test]
fn error_bad_tags_and_namespaced_maps() {
    assert_err("[#foo]", 1, 2, "has no value");
    assert_err("#foo", 1, 1, "has no value");
    assert_err("#1foo 2", 1, 1, "invalid tag");
    assert_err("#:{:a 1}", 1, 1, "missing namespace");
    assert_err("#:ns 1", 1, 6, "expected `{`");
    assert_err("#:ns", 1, 5, "expected `{`");
}

#[test]
fn error_bad_symbolic_value() {
    assert_err("##Foo", 1, 1, "unknown symbolic value `##Foo`");
    assert_err("[1\n ##]", 2, 2, "unknown symbolic value");
}

#[test]
fn error_bad_character() {
    assert_err(r"\foo", 1, 1, "unknown character literal `\\foo`");
    assert_err("[1 \\u12]", 1, 4, "unknown character literal");
    assert_err(r"\ud800", 1, 1, "unknown character literal");
    assert_err("\\", 1, 1, "incomplete character literal");
    // Adjacent characters are one token, not two.
    assert_err("[\\a\\b]", 1, 2, "unknown character literal");
}

#[test]
fn error_nesting_depth() {
    let deepest_ok = format!("{}{}", "[".repeat(256), "]".repeat(256));
    assert!(parse(&deepest_ok).is_ok());

    let too_deep = format!("{}{}", "[".repeat(257), "]".repeat(257));
    assert_err(&too_deep, 1, 257, "nesting deeper than 256");

    // No stack overflow on hostile input; the error comes as soon as the limit is hit.
    assert_err(&"[".repeat(1_000_000), 1, 257, "nesting deeper");
    assert_err(&"{:a ".repeat(1_000), 1, 1 + 4 * 256, "nesting deeper");
    assert_err(&"#_".repeat(1_000), 1, 513, "nesting deeper");
    assert_err(&"#a ".repeat(1_000), 1, 769, "nesting deeper");
}

#[test]
fn error_display_and_trait() {
    let e = fails("[1\n  \"abc");
    assert_eq!(e.to_string(), "line 2, column 3: unterminated string");
    let boxed: Box<dyn std::error::Error> = Box::new(e);
    assert!(boxed.to_string().starts_with("line 2, column 3"));
}
