//! Assembly defines by shape and calls by opcode, so every rule here is one a
//! keyword table cannot express.

use native_parsers::Language;

fn parse(src: &str) -> native_parsers::facts::FileFacts {
    native_parsers::rules::active().parse(Language::Assembly, src)
}

fn calls(src: &str) -> Vec<(String, String)> {
    parse(src)
        .calls
        .into_iter()
        .map(|(from, to, _)| (from, to))
        .collect()
}

fn pair(from: &str, to: &str) -> (String, String) {
    (from.to_string(), to.to_string())
}

#[test]
fn labels_constants_and_macros_are_definitions() {
    let src = "\
ENTRY_SIZE     EQU     4
SIZE = 12
SAVE        MACRO
            JSR     save_regs
            ENDM
charge:
            MOVE.L  ENTRY_SIZE(A2),A2
.local:
            RTS
bare        MOVEQ   #0,D0
            RTS
";
    let defines = parse(src).defines;
    for want in ["ENTRY_SIZE", "SIZE", "SAVE", "charge", "bare"] {
        assert!(
            defines.iter().any(|d| d == want),
            "{want} missing from {defines:?}"
        );
    }
    // A local label belongs to the routine above it, and an opcode in
    // column zero is not a label.
    for not in ["local", "RTS", "MOVEQ"] {
        assert!(!defines.iter().any(|d| d == not), "{not} in {defines:?}");
    }
}

#[test]
fn a_constant_ends_with_its_line() {
    // A constant between two calls must not take the second one: as a label
    // it would, and every call after an `EQU` would leave its routine.
    let got = calls("run:\n    JSR     first\nLIMIT EQU 5\n    JSR     second\n");
    assert_eq!(got, vec![pair("run", "first"), pair("run", "second")]);
    // An opcode written in column zero is a statement, not a label.
    let defines = parse("run:\nRTS\n").defines;
    assert_eq!(defines, vec!["run".to_string()]);
}

#[test]
fn branches_to_global_labels_and_macros_are_calls() {
    let src = "\
SAVE        MACRO
            JSR     save_regs
            ENDM
refuse:
            BRA     settle
settle:
            SAVE    ledger
            BEQ.S   .missing
            JSR     (A2)
            JSR     off_open(A6)
            JMP     tail(PC)
            DBRA    D0,again
            MOVE.L  D0,-(SP)
.missing:
            RTS
";
    let got = calls(src);
    for want in [
        pair("SAVE", "save_regs"),
        pair("refuse", "settle"),
        pair("settle", "SAVE"),
        pair("settle", "tail"),
        pair("settle", "again"),
    ] {
        assert!(got.contains(&want), "{want:?} missing from {got:?}");
    }
    // A local branch, a jump through a register and an ordinary opcode
    // name no routine.
    for callee in ["missing", "A2", "off_open", "MOVE", "RTS"] {
        assert!(
            !got.iter().any(|(_, to)| to == callee),
            "{callee} in {got:?}"
        );
    }
    assert_eq!(got.len(), 5, "{got:?}");
}

#[test]
fn comment_lines_and_x86_are_not_statements() {
    // A `*` line is a Motorola comment: its words are not macro calls.
    assert!(calls("* SAVE THE DOOR\nrun:\n    RTS\n").is_empty());
    // An indented `#` is a GNU comment, and a `#define` continues across a
    // trailing backslash.
    assert!(calls("run:\n    # Mach-O only\n    RTS\n").is_empty());
    assert!(calls("#define XX(n) \\\n    JUMP n\nrun:\n    RTS\n").is_empty());
    // `#` after the opcode is an immediate, and the line goes on.
    assert_eq!(
        calls("run:\n    MOVEQ   #0,D0\n    JSR     next\n"),
        vec![pair("run", "next")]
    );
    // Lower-case words outside the opcode list are x86 instructions, not
    // macros; `call` and `jne` still reach their targets.
    let got = calls("run:\n    movl %eax, %ebx\n    call notify\n    jne run\n");
    assert_eq!(got, vec![pair("run", "notify"), pair("run", "run")]);
}

#[test]
fn a_displacement_is_a_use_not_a_call() {
    let names: Vec<&str> = native_parsers::rules::active()
        .identifiers(Language::Assembly, "run:\n    MOVE.L  ENTRY_SIZE(A2),A2\n")
        .into_iter()
        .map(|(_, name, _)| name)
        .collect();
    assert!(names.contains(&"ENTRY_SIZE"), "{names:?}");
}
