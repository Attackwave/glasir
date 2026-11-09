//! Literals and comments a lexer misread, so that a string or a comment
//! swallowed the code after it, or commented-out code came back.

use native_parsers::Language;

fn defines(lang: Language, src: &str) -> Vec<String> {
    native_parsers::rules::active().parse(lang, src).defines
}

#[test]
fn a_literal_ends_where_its_language_ends_it() {
    for (lang, src, want, not) in [
        // C# verbatim: a backslash before the closing quote is a character.
        (
            Language::CSharp,
            "class A {\n  string p = @\"C:\\dir\\\";\n  void RealOne() { Helper(); }\n  void RealTwo() {}\n}\n",
            &["RealOne", "RealTwo"][..],
            &[][..],
        ),
        // Lua long string holding a quote.
        (
            Language::Lua,
            "local s = [[ say \"hi ]]\nfunction real_one() helper() end\nfunction real_two() end\n",
            &["real_one", "real_two"],
            &[],
        ),
        // Lua level comment: the code inside stays commented out.
        (
            Language::Lua,
            "--[==[\nfunction ghost() end\n]==]\nfunction real() end\n",
            &["real"],
            &["ghost"],
        ),
        // Lua block comments do not nest.
        (
            Language::Lua,
            "--[[ old: --[[ ]]\nfunction real() end\n",
            &["real"],
            &[],
        ),
        // Ruby's `=begin` does not nest either.
        (
            Language::Ruby,
            "=begin\n=begin\n=end\ndef real\nend\n",
            &["real"],
            &[],
        ),
        // C++ raw string with an encoding prefix and a lone quote inside.
        (
            Language::Cpp,
            "auto s = u8R\"(x\")\";\nvoid real_two() {}\nvoid real_three() {}\n",
            &["real_two", "real_three"],
            &[],
        ),
    ] {
        let got = defines(lang, src);
        for w in want {
            assert!(got.iter().any(|g| g == w), "{lang:?} lost {w}: {got:?}");
        }
        for n in not {
            assert!(!got.iter().any(|g| g == n), "{lang:?} invented {n}: {got:?}");
        }
    }
}

/// A byte order mark is not part of the first word, and offsets do not move.
#[test]
fn a_byte_order_mark_hides_nothing() {
    let plain = "def first():\n    return second()\n\ndef second():\n    return 1\n";
    let marked = format!("\u{feff}{plain}");
    let facts = native_parsers::rules::active().parse(Language::Python, &marked);
    assert!(
        facts.defines.iter().any(|d| d == "first"),
        "{:?}",
        facts.defines
    );
    let at = |src: &str, name: &str| {
        native_parsers::rules::active()
            .parse(Language::Python, src)
            .ranges
            .into_iter()
            .find(|r| r.0 == name)
            .map(|r| r.1 .0)
    };
    assert_eq!(at(&marked, "second").map(|b| b - 3), at(plain, "second"));
    let java = "\u{feff}package a;\nclass B { void c() {} }\n";
    let tokens = native_parsers::rules::active().identifiers(Language::Java, java);
    assert_eq!(tokens.first().map(|t| t.1), Some("package"), "{tokens:?}");
}

/// Calls on a value: through `?.`, and to a closure or parameter by its name.
#[test]
fn a_call_on_a_value_is_not_a_call_by_name() {
    let calls = |src: &str| {
        native_parsers::rules::active()
            .parse(Language::Rust, src)
            .calls
    };
    let bare = |src: &str, callee: &str| calls(src).iter().any(|(_, c, recv)| c == callee && !recv);
    let any = |src: &str, callee: &str| calls(src).iter().any(|(_, c, _)| c == callee);
    let tried = "fn f(root: &Path) -> Option<bool> { Some(root.parent()?.join(\"s\").exists()) }\n";
    assert!(
        any(tried, "join") && !bare(tried, "join"),
        "{:?}",
        calls(tried)
    );
    let closure = "fn g(lines: &[&str]) -> usize { let key = |l: &str| l.len(); lines.iter().map(|l| key(l)).sum() }\n";
    assert!(!any(closure, "key"), "{:?}", calls(closure));
    let param = "fn h<F: Fn()>(base_targets: impl Fn(u32) -> u32, cb: F) { base_targets(1); cb(); helper(); }\n";
    assert!(
        !any(param, "base_targets") && !any(param, "cb"),
        "{:?}",
        calls(param)
    );
    assert!(any(param, "helper"), "{:?}", calls(param));
    let shadow = "fn p(x: u32) { let parse = parse(x); }\nfn q() { parse(1); }\n";
    assert_eq!(
        calls(shadow)
            .iter()
            .filter(|(_, c, _)| c == "parse")
            .count(),
        2,
        "{:?}",
        calls(shadow)
    );
}

/// A closure whose body binds names of its own is still a local.
#[test]
fn a_closure_with_lets_inside_is_local() {
    let src =
        "fn openapi(text: &str) { let key = |line: &str| { let k = line.len(); k }; key(text); }\n";
    let calls = native_parsers::rules::active()
        .parse(Language::Rust, src)
        .calls;
    assert!(!calls.iter().any(|(_, c, _)| c == "key"), "{calls:?}");
}
