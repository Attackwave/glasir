//! Modern C# writes most members as one of three shapes the scanner missed:
//! an expression body, a generic method, a `const` before them. Measured on
//! Polly, 59% of the foreign structural set before and 100% after.

use native_parsers::Language;

#[test]
fn csharp_expression_bodied_and_generic_members_are_definitions() {
    let src = "class U\n{\n    public const string M = \"The '{0}' must\";\n    public static bool Run(int x) => x > 0;\n    public static void Report<TResult>(\n        int x)\n    {\n        Log(x);\n    }\n    internal static T Pick<T>(T a) where T : class => Choose(a);\n}\n";
    let f = native_parsers::rules::active().parse(Language::CSharp, src);
    for name in ["M", "Run", "Report", "Pick"] {
        assert!(
            f.defines.iter().any(|d| d == name),
            "{name} missing: {:?}",
            f.defines
        );
    }
    // The constant ends at its semicolon; before, it stayed open to the end
    // of the class and took the calls of every member after it.
    let (_, (start, end)) = f.ranges.iter().find(|(n, _)| n == "M").unwrap();
    let m = &src[*start as usize..*end as usize];
    assert!(m.starts_with("public const") && m.ends_with(';'), "{m:?}");
    // A range starts at the modifiers, so a snippet is the whole declaration.
    let (_, (start, _)) = f.ranges.iter().find(|(n, _)| n == "Run").unwrap();
    assert!(src[*start as usize..].starts_with("public static bool Run"));
    assert!(
        f.calls
            .iter()
            .any(|(from, to, _)| from == "Report" && to == "Log"),
        "{:?}",
        f.calls
    );
    assert!(
        f.calls
            .iter()
            .any(|(from, to, _)| from == "Pick" && to == "Choose"),
        "{:?}",
        f.calls
    );
}
