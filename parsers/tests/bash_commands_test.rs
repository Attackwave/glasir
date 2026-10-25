use native_parsers::Language;

/// Only the first word of a command is a call: flags, arguments, variables
/// and continuation lines are not.
#[test]
fn bash_records_commands_only() {
    let src = "set -e\necho hi\nout=${X}\nCI=1 cargo build\n[ -z \"$x\" ] && exit 1\ngh api \\\n  repos/o/r -H \"Accept: x\"\nif [ -f y ]; then rm -rf y; fi\nresult=$(git rev-parse HEAD)\n";
    let facts = Language::Bash.parse(src);
    let calls: Vec<&str> = facts.calls.iter().map(|(_, c, _)| c.as_str()).collect();
    for want in ["set", "cargo", "gh", "rm", "git"] {
        assert!(calls.contains(&want), "{want} missing from {calls:?}");
    }
    for noise in [
        "e",
        "hi",
        "X",
        "CI",
        "build",
        "z",
        "api",
        "repos",
        "H",
        "f",
        "y",
        "result",
        "rev-parse",
        "HEAD",
    ] {
        assert!(!calls.contains(&noise), "{noise} recorded in {calls:?}");
    }
}
