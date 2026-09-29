//! Six scanners read a keyword, a prefix or a type as a name. Each was found
//! by `glasir langcheck --anomalies`, none by the yield floors: the count of
//! definitions was right, only the names were wrong.

use native_parsers::Language;

fn defines(lang: Language, src: &str) -> Vec<String> {
    native_parsers::rules::active().parse(lang, src).defines
}

#[test]
fn a_keyword_prefix_or_type_is_not_a_definition() {
    for (lang, src, want, not) in [
        // `if (x) {` has the shape of a C function definition.
        (
            Language::C,
            "int f(int x)\n{\n    if (x) {\n        g();\n    }\n}\n",
            &["f"][..],
            &["if"][..],
        ),
        (
            Language::Cpp,
            "int f(int x) {\n    for (;;) {\n        g();\n    }\n}\n",
            &["f"],
            &["for"],
        ),
        // A comment between the signature and the body.
        (
            Language::C,
            "int f(void)\n/* why */\n{\n    g();\n}\n",
            &["f"],
            &[],
        ),
        // A const group inside a proc ends where the proc's statements begin.
        (
            Language::Nim,
            "proc f(): int =\n  const\n    GRACE = 1\n  if x > GRACE:\n    result = 2\n",
            &["f", "GRACE"],
            &["if", "result"],
        ),
        // Pony writes the reference capability between `fun` and the name.
        (
            Language::Pony,
            "class A\n  fun ref charge(a: I64): I64 =>\n    a\n",
            &["charge"],
            &["ref"],
        ),
        // Vim's scope prefix belongs to the name's call, not to the name.
        (
            Language::Vimscript,
            "function! s:Charge(owner)\n    return 0\nendfunction\n",
            &["Charge"],
            &["s"],
        ),
        // Zig's `[]const u8` qualifies a type.
        (
            Language::Zig,
            "fn f(owner: []const u8) i64 {\n    return 0;\n}\n",
            &["f"],
            &["u8"],
        ),
        // GN's `sources = [...]` is a property of the target.
        (
            Language::Gn,
            "source_set(\"charge\") {\n  sources = [ \"a.cc\" ]\n}\n",
            &["charge"],
            &["sources"],
        ),
    ] {
        let d = defines(lang, src);
        for w in want {
            assert!(d.iter().any(|n| n == w), "{lang:?}: {w} missing from {d:?}");
        }
        for n in not {
            assert!(!d.iter().any(|x| x == n), "{lang:?}: {n} defined in {d:?}");
        }
    }
}

/// A definition without a body ends where the next one at its level begins,
/// and a statement a line break ends does too. Before, a constant's range held
/// every function after it, and in a language without `;` the constant also
/// took every file-level call after it.
#[test]
fn a_constant_ends_before_the_next_definition() {
    let rules = native_parsers::rules::active();
    for (lang, src, constant, next) in [
        (
            Language::C,
            "#define LIMIT 5\n\nint f(void)\n{\n    g();\n}\n",
            "LIMIT",
            "f",
        ),
        (
            Language::Java,
            "class A {\n    private static final String LIMIT = \"x\";\n\n    void f() {\n        g();\n    }\n}\n",
            "LIMIT",
            "f",
        ),
        (
            Language::D,
            "enum LIMIT = 5000;\n\nint f()\n{\n    return g();\n}\n",
            "LIMIT",
            "f",
        ),
        (
            Language::Bash,
            "LIMIT=5\n\nf() {\n    g\n}\n\nh\n",
            "LIMIT",
            "f",
        ),
    ] {
        let facts = rules.parse(lang, src);
        let range = |n: &str| facts.ranges.iter().find(|(x, _)| x == n).map(|(_, r)| *r);
        let (Some(c), Some(f)) = (range(constant), range(next)) else {
            panic!("{lang:?}: {constant} or {next} missing: {:?}", facts.ranges);
        };
        assert!(
            c.1 <= f.0,
            "{lang:?}: {constant} {c:?} runs into {next} {f:?}"
        );
    }
    // A file-level command after the functions is the file's, not the constant's.
    let bash = rules.parse(Language::Bash, "LIMIT=5\n\nf() {\n    g\n}\n\nh\n");
    assert!(
        bash.calls
            .iter()
            .any(|(from, to, _)| from == "<module>" && to == "h"),
        "{:?}",
        bash.calls
    );
}

/// A `#define` ends with its logical line, even before an `extern "C" {`
/// that puts the next definition a level deeper, and a call in its body is
/// the macro's.
#[test]
fn a_macro_ends_with_its_line() {
    let src = "#define CHECK(x) \\\n    verify(x)\n#ifdef __cplusplus\nextern \"C\" {\n#endif\nint f(void)\n{\n    g();\n}\n#ifdef __cplusplus\n}\n#endif\n";
    for lang in [Language::C, Language::Cpp] {
        let facts = native_parsers::rules::active().parse(lang, src);
        let (_, (s, e)) = facts
            .ranges
            .iter()
            .find(|(n, _)| n == "CHECK")
            .expect("CHECK");
        assert_eq!(
            &src[*s as usize..*e as usize],
            "#define CHECK(x) \\\n    verify(x)",
            "{lang:?}"
        );
        assert!(
            facts
                .calls
                .iter()
                .any(|(from, to, _)| from == "CHECK" && to == "verify"),
            "{lang:?}: {:?}",
            facts.calls
        );
    }
}

/// `'` opens a string except where a language names things with it. Read as
/// a lifetime everywhere, `'a b'` became the name `'a`, and the closing quote
/// a string to the end of the line that swallowed the code after it.
#[test]
fn a_single_quote_is_a_string_unless_the_language_names_with_it() {
    let rules = native_parsers::rules::active();
    let js = rules.parse(Language::JavaScript, "run('a function if b', check(x));\n");
    assert!(
        js.calls.iter().any(|(_, to, _)| to == "check"),
        "{:?}",
        js.calls
    );
    assert!(!js.defines.iter().any(|d| d == "if"), "{:?}", js.defines);
    // Rust keeps its lifetimes, OCaml its type variables.
    let rust = rules.parse(Language::Rust, "fn f<'a>(x: &'a str) -> &'a str { g(x) }\n");
    assert!(
        rust.calls
            .iter()
            .any(|(from, to, _)| from == "f" && to == "g"),
        "{:?}",
        rust.calls
    );
    let ocaml = rules.parse(Language::OCaml, "type 'a t = 'a list\nlet f x = g x\n");
    assert!(
        ocaml.defines.iter().any(|d| d == "f"),
        "{:?}",
        ocaml.defines
    );
}

/// The leads left after the first round, each read at its source.
#[test]
fn the_remaining_leads_read_right() {
    let rules = native_parsers::rules::active();
    let check = |lang: Language, src: &str, want: &[&str], not: &[&str]| {
        let d = rules.parse(lang, src).defines;
        for w in want {
            assert!(d.iter().any(|n| n == w), "{lang:?}: {w} missing from {d:?}");
        }
        for n in not {
            assert!(!d.iter().any(|x| x == n), "{lang:?}: {n} defined in {d:?}");
        }
    };
    // A Go function type has no name after its parameters; it took the next
    // line's first word and swallowed the function declared there.
    check(Language::Go, "package p\n\ntype H func(w int)\n\nfunc run() {\n\tvar f func(a int)\n\tswitch x {\n\t}\n}\n", &["H", "run"], &["func", "type", "switch"]);
    // C#: `fixed (…) {` has a method's shape; `record` is also a plain name.
    check(Language::CSharp, "class A {\n  void M(object record) {\n    Rec r = record as Rec;\n    fixed (byte* pb = bytes)\n    {\n      Use(pb);\n    }\n  }\n}\n", &["A", "M"], &["as", "fixed"]);
    check(
        Language::Kotlin,
        "fun interface ProgressListener {\n    fun onProgress(sent: Long)\n}\n",
        &["ProgressListener", "onProgress"],
        &["interface"],
    );
    check(
        Language::Erlang,
        "'MACRO-@'(Caller, Tree) ->\n  unless_loaded(Tree, fun() -> nil end).\n",
        &[],
        &["fun"],
    );
    // Perl's POD is prose, and so is everything after __END__.
    check(Language::Perl, "=head1 CAVEATS\n\nThere are caveats if you want\nto use it for portable applications:\n\n=cut\n\nsub run {\n    go();\n}\n\n__END__\nsub gone { }\n", &["run"], &["if", "gone"]);
    // Keywords followed by a parenthesis are not calls.
    for (lang, src, keyword) in [
        (Language::Java, "class A {\n  void m() {\n    try (Res r = open()) {\n      use(r);\n    }\n  }\n}\n", "try"),
        (Language::Solidity, "contract C {\n  function f() public {\n    try t.g() {} catch (bytes memory reason) {}\n  }\n}\n", "catch"),
        (Language::Perl, "sub f {\n    if ($a) { x(); } elsif ($b) { y(); }\n}\n", "elsif"),
    ] {
        let calls = rules.parse(lang, src).calls;
        assert!(!calls.iter().any(|(_, to, _)| to == keyword), "{lang:?}: {keyword} called in {calls:?}");
    }
}

/// An Erlang function may be named by a quoted atom, as generated code does
/// thousands of times; its clause and its calls were invisible.
#[test]
fn an_erlang_quoted_atom_names_a_function() {
    let src = "'Mfa_0'(M) -> ('fun_Mfa_0'(M))().\n'fun_Mfa_0'(M) -> fun M:f/0.\n";
    let f = native_parsers::rules::active().parse(Language::Erlang, src);
    for name in ["Mfa_0", "fun_Mfa_0"] {
        assert!(
            f.defines.iter().any(|d| d == name),
            "{name} missing: {:?}",
            f.defines
        );
    }
    assert!(
        f.calls
            .iter()
            .any(|(from, to, _)| from == "Mfa_0" && to == "fun_Mfa_0"),
        "{:?}",
        f.calls
    );
}
