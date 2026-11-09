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
