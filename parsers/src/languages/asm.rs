//! Language-specific syntax; lexical and call rules come from the registry.
//!
//! Assembly defines by *shape*: a label is a name at the start of a line,
//! with no keyword anywhere, and a call is an opcode and its operand rather
//! than `name(`. One module reads NASM and the Motorola and GNU dialects,
//! because the three differ in words, not in shape.

use crate::facts::FileFacts;
use crate::lexer::{CommentStyle, Lexer, Token, TokenKind};
use crate::scope::ScopeStack;

/// Opcodes whose operand is the next routine: a call, a jump or a branch.
/// A branch to a global label is how a shared tail is reached (`BRA
/// settle`), so it is an edge like a call.
const BRANCHES: &[&str] = &[
    "bsr", "jsr", "jmp", "bra", "call", "callq", "jmpq", "bl", "blx", "jal", "bhi", "bls", "bcc",
    "bcs", "bne", "beq", "bvc", "bvs", "bpl", "bmi", "bge", "blt", "bgt", "ble", "bhs", "blo",
    "ja", "jae", "jb", "jbe", "jc", "je", "jg", "jge", "jl", "jle", "jna", "jnae", "jnb", "jnbe",
    "jnc", "jne", "jng", "jnge", "jnl", "jnle", "jno", "jnp", "jns", "jnz", "jo", "jp", "jpe",
    "jpo", "js", "jz", "loop",
];

/// `DBRA D0,.loop`: the target is the second operand.
const COUNTED_BRANCHES: &[&str] = &[
    "dbra", "dbf", "dbt", "dbhi", "dbls", "dbcc", "dbcs", "dbne", "dbeq", "dbvc", "dbvs", "dbpl",
    "dbmi", "dbge", "dblt", "dbgt", "dble",
];

/// Instructions and directives that are not a macro, so an opcode outside
/// this list is one. Motorola code writes both in upper case, which is why
/// the list is what separates them rather than the case.
const OPCODES: &[&str] = &[
    "abcd", "add", "adda", "addi", "addq", "addx", "and", "andi", "asl", "asr", "bchg", "bclr",
    "bset", "btst", "bfchg", "bfclr", "bfexts", "bfextu", "bfffo", "bfins", "bfset", "bftst",
    "bkpt", "callm", "cas", "cas2", "chk", "chk2", "clr", "cmp", "cmp2", "cmpa", "cmpi", "cmpm",
    "divs", "divsl", "divu", "divul", "eor", "eori", "exg", "ext", "extb", "illegal", "lea",
    "link", "lsl", "lsr", "move", "movea", "movec", "movem", "movep", "moveq", "moves", "muls",
    "mulu", "nbcd", "neg", "negx", "nop", "not", "or", "ori", "pack", "pea", "reset", "rol", "ror",
    "roxl", "roxr", "rtd", "rte", "rtm", "rtr", "rts", "sbcd", "stop", "sub", "suba", "subi",
    "subq", "subx", "swap", "tas", "trap", "trapv", "tst", "unlk", "unpk", "st", "sf", "shi",
    "sls", "scc", "scs", "sne", "seq", "svc", "svs", "spl", "smi", "sge", "slt", "sgt", "sle",
    "fmove", "fmovem", "fadd", "fsub", "fmul", "fdiv", "fcmp", "ftst", "fneg", "fabs", "fsqrt",
    "fint", "fbeq", "fbne", "dc", "ds", "dcb", "blk", "equ", "equr", "set", "reg", "macro", "endm",
    "mexit", "section", "xdef", "xref", "public", "global", "globl", "extern", "include", "incbin",
    "incdir", "even", "odd", "cnop", "align", "org", "rsreset", "rsset", "rs", "so", "fo", "if",
    "ifd", "ifnd", "ifeq", "ifne", "ifc", "ifnc", "ifgt", "ifge", "iflt", "ifle", "else", "elseif",
    "endc", "endif", "end", "rept", "endr", "opt", "output", "machine", "mc68000", "mc68010",
    "mc68020", "mc68030", "mc68040", "mc68060", "near", "far", "text", "data", "bss", "code",
    "idnt", "ttl", "printt", "printv", "fail", "echo", "list", "nolist", "offset", "ret", "db",
    "dw", "dd", "dq", "resb", "resw", "resd", "resq", "times", "bits",
];

/// What ends a routine's own flow: a return, or a branch that always leaves.
const ENDS: &[&str] = &[
    "rts", "rte", "rtr", "rtd", "rtm", "ret", "bra", "jmp", "jmpq", "illegal",
];

/// Words in `OPCODES` that assemble no instruction: data, layout and
/// conditional assembly. A routine whose last statement is one of these is
/// not known to fall into the next label.
const DIRECTIVES: &[&str] = &[
    "dc", "ds", "dcb", "blk", "equ", "equr", "set", "reg", "macro", "endm", "mexit", "section",
    "xdef", "xref", "public", "global", "globl", "extern", "include", "incbin", "incdir", "even",
    "odd", "cnop", "align", "org", "rsreset", "rsset", "rs", "so", "fo", "if", "ifd", "ifnd",
    "ifeq", "ifne", "ifc", "ifnc", "ifgt", "ifge", "iflt", "ifle", "else", "elseif", "endc",
    "endif", "end", "rept", "endr", "opt", "output", "machine", "mc68000", "mc68010", "mc68020",
    "mc68030", "mc68040", "mc68060", "near", "far", "text", "data", "bss", "code", "idnt", "ttl",
    "printt", "printv", "fail", "echo", "list", "nolist", "offset", "db", "dw", "dd", "dq", "resb",
    "resw", "resd", "resq", "times", "bits",
];

fn is(word: &str, list: &[&str]) -> bool {
    list.iter().any(|w| word.eq_ignore_ascii_case(w))
}

pub(crate) fn parse(src: &str, style: CommentStyle<'_>, calls: &crate::rules::Calls) -> FileFacts {
    let mut facts = FileFacts::new();
    let mut lexer = Lexer::new(src, style);
    let tokens = lexer.collect_all_tokens();
    let bytes = src.as_bytes();
    let at_column_zero =
        |t: &Token<'_>| t.start == 0 || bytes.get(t.start as usize - 1) == Some(&b'\n');
    let kind = |i: usize| tokens.get(i).map(|t| &t.kind);

    let mut scope = ScopeStack::new();
    // A label or a macro body is open: the next one closes it.
    let mut in_body = false;
    // An `EQU` constant is open until the end of its line.
    let mut in_constant = false;
    // The first word of a statement is its opcode; the rest are operands.
    let mut seen_opcode = false;
    // The macro whose body is being read, and the name of its first
    // parameter where the dialect names it (`.macro var name`).
    let mut macro_name: Option<(String, Option<String>)> = None;
    // The label whose body is open, and how its last statement leaves it:
    // `Flow` runs on into the next label, `Jump` always branches to a name,
    // `Other` (a directive, data, a macro, nothing yet) says neither.
    let mut routine: Option<String> = None;
    let mut last = Exit::Other;
    let line_of = |at: usize| {
        let start = src[..at].rfind('\n').map_or(0, |n| n + 1);
        let end = src[at..].find('\n').map_or(src.len(), |n| at + n);
        (start as u32, end as u32)
    };

    let open_body = |scope: &mut ScopeStack,
                     in_body: &mut bool,
                     name: &str,
                     start: usize,
                     facts: &mut FileFacts| {
        if *in_body {
            scope.on_close_delimiter(start, facts);
        }
        scope.open_definition(name, start, true, facts);
        scope.on_word(name);
        scope.on_open_delimiter();
        *in_body = true;
    };

    let mut i = 0;
    while i < tokens.len() {
        let tok = &tokens[i];
        match &tok.kind {
            TokenKind::DocComment(text) | TokenKind::LineComment(text) => {
                scope.push_comment(text);
            }
            TokenKind::Newline => {
                if in_constant {
                    scope.on_statement_end(tok.start as usize, &mut facts);
                    in_constant = false;
                }
                seen_opcode = false;
            }
            // A Motorola comment line starts with `*` in column zero. A `#`
            // where an opcode belongs is a GNU comment or a preprocessor line,
            // since an immediate never starts a statement; a `\` at the end
            // carries a `#define` on to the next line.
            TokenKind::Symbol(c @ ('*' | '#'))
                if (*c == '*' && at_column_zero(tok)) || (*c == '#' && !seen_opcode) =>
            {
                let mut end = tok.start as usize;
                loop {
                    end = src[end..].find('\n').map_or(src.len(), |n| end + n);
                    if *c == '*' || !src[..end].trim_end().ends_with('\\') || end == src.len() {
                        break;
                    }
                    end += 1;
                }
                if *c == '*' {
                    scope.push_comment(&src[tok.start as usize + 1..end]);
                }
                while i < tokens.len() && (tokens[i].start as usize) < end {
                    i += 1;
                }
                continue;
            }
            // `\1 EQU …` or `\1:` in column zero of a macro body: every
            // invocation of this macro defines its first argument.
            TokenKind::Symbol('\\') if at_column_zero(tok) && macro_name.is_some() => {
                let (name, param) = macro_name.as_ref().expect("checked");
                let first = match kind(i + 1) {
                    Some(TokenKind::Number(n)) => *n == "1",
                    Some(TokenKind::Ident(p)) => param.as_deref() == Some(*p),
                    _ => false,
                };
                let defines = matches!(kind(i + 2), Some(TokenKind::Symbol(':' | '=')))
                    || matches!(kind(i + 2), Some(TokenKind::Ident(w)) if is(w, &["equ", "set", "equr", "reg"]));
                if first && defines && !facts.macro_definers.contains(name) {
                    facts.macro_definers.push(name.clone());
                }
            }
            // GNU directives: `.macro name`, `.endm`, `.equ name, value`. Any
            // other dot is a local label, a directive or a size suffix, and
            // what follows it is not a statement's opcode.
            TokenKind::Symbol('.') => {
                if !seen_opcode {
                    last = Exit::Other;
                }
                match kind(i + 1) {
                    Some(TokenKind::Ident(d)) if d.eq_ignore_ascii_case("macro") => {
                        settle(&mut routine, &mut last, None, calls, &mut scope, &mut facts);
                        if let Some(TokenKind::Ident(name)) = kind(i + 2) {
                            let param = match kind(i + 3) {
                                Some(TokenKind::Ident(p)) => Some(p.to_string()),
                                _ => None,
                            };
                            macro_name = Some((name.to_string(), param));
                            open_body(
                                &mut scope,
                                &mut in_body,
                                name,
                                tok.start as usize,
                                &mut facts,
                            );
                            i += 3;
                            seen_opcode = true;
                            continue;
                        }
                    }
                    Some(TokenKind::Ident(d)) if d.eq_ignore_ascii_case("endm") => {
                        macro_name = None;
                        if in_body {
                            scope.on_close_delimiter(tokens[i + 1].end as usize, &mut facts);
                            in_body = false;
                        }
                    }
                    Some(TokenKind::Ident(d))
                        if d.eq_ignore_ascii_case("equ") || d.eq_ignore_ascii_case("set") =>
                    {
                        if let Some(TokenKind::Ident(name)) = kind(i + 2) {
                            scope.open_statement_definition(*name, tok.start as usize, &mut facts);
                            in_constant = true;
                            i += 3;
                            seen_opcode = true;
                            continue;
                        }
                    }
                    _ => {}
                }
                // Local label, size suffix or directive: skip the word.
                if matches!(kind(i + 1), Some(TokenKind::Ident(_))) {
                    i += 1;
                }
                seen_opcode = true;
            }
            TokenKind::Ident(ident) if at_column_zero(tok) => {
                let next = kind(i + 1);
                let start = tok.start as usize;
                match next {
                    // `name EQU value` and `name = value`: a constant.
                    Some(TokenKind::Ident(w)) if is(w, &["equ", "equr", "reg"]) => {
                        scope.open_statement_definition(*ident, start, &mut facts);
                        scope.on_word(ident);
                        in_constant = true;
                        i += 2;
                        seen_opcode = true;
                        continue;
                    }
                    Some(TokenKind::Symbol('=')) => {
                        scope.open_statement_definition(*ident, start, &mut facts);
                        scope.on_word(ident);
                        in_constant = true;
                        i += 2;
                        seen_opcode = true;
                        continue;
                    }
                    // `name MACRO`: a body up to `ENDM`.
                    Some(TokenKind::Ident(w)) if w.eq_ignore_ascii_case("macro") => {
                        settle(&mut routine, &mut last, None, calls, &mut scope, &mut facts);
                        macro_name = Some((ident.to_string(), None));
                        open_body(&mut scope, &mut in_body, ident, start, &mut facts);
                        i += 2;
                        seen_opcode = true;
                        continue;
                    }
                    // `name:` is a label in every dialect; Motorola also takes
                    // a bare name in column zero, which an opcode never is.
                    Some(TokenKind::Symbol(':')) => {
                        settle(
                            &mut routine,
                            &mut last,
                            Some(ident),
                            calls,
                            &mut scope,
                            &mut facts,
                        );
                        open_body(&mut scope, &mut in_body, ident, start, &mut facts);
                        routine = Some(ident.to_string());
                        i += 2;
                        continue;
                    }
                    Some(TokenKind::Newline | TokenKind::Ident(_)) | None
                        if !is(ident, OPCODES) && !is(ident, BRANCHES) =>
                    {
                        settle(
                            &mut routine,
                            &mut last,
                            Some(ident),
                            calls,
                            &mut scope,
                            &mut facts,
                        );
                        open_body(&mut scope, &mut in_body, ident, start, &mut facts);
                        routine = Some(ident.to_string());
                        i += 1;
                        continue;
                    }
                    _ => {}
                }
                seen_opcode = true;
            }
            TokenKind::Ident(ident) if !seen_opcode => {
                seen_opcode = true;
                last = if is(ident, ENDS) {
                    Exit::Ends
                } else if is(ident, BRANCHES)
                    || is(ident, COUNTED_BRANCHES)
                    || (is(ident, OPCODES) && !is(ident, DIRECTIVES))
                {
                    Exit::Flow
                } else {
                    Exit::Other
                };
                // A size suffix is not an operand: `BEQ.S .missing`.
                let mut j = i + 1;
                if kind(j) == Some(&TokenKind::Symbol('.'))
                    && matches!(kind(j + 1), Some(TokenKind::Ident(s)) if s.len() == 1)
                {
                    j += 2;
                }
                if is(ident, COUNTED_BRANCHES) {
                    while !matches!(
                        kind(j),
                        Some(TokenKind::Symbol(',') | TokenKind::Newline) | None
                    ) {
                        j += 1;
                    }
                    if kind(j) == Some(&TokenKind::Symbol(',')) {
                        j += 1;
                    }
                }
                if is(ident, BRANCHES) || is(ident, COUNTED_BRANCHES) {
                    // `JMP name(PC)` reaches `name`; `JSR off(A6)` and
                    // `JSR (A2)` go through a register and name no routine.
                    if let Some(TokenKind::Ident(target)) = kind(j) {
                        let indexed = kind(j + 1) == Some(&TokenKind::Symbol('('))
                            && !matches!(kind(j + 2), Some(TokenKind::Ident(r)) if r.eq_ignore_ascii_case("pc"));
                        if !indexed && calls.allows(target) {
                            scope.record_call(target, &mut facts);
                        }
                        if !indexed && is(ident, ENDS) {
                            last = Exit::Jump(target.to_string());
                        }
                        scope.on_word(target);
                        i = j + 1;
                        continue;
                    }
                } else if ident.eq_ignore_ascii_case("endm") {
                    macro_name = None;
                    if in_body {
                        scope.on_close_delimiter(tok.end as usize, &mut facts);
                        in_body = false;
                    }
                } else if ident.eq_ignore_ascii_case("macro") {
                    settle(&mut routine, &mut last, None, calls, &mut scope, &mut facts);
                    if let Some(TokenKind::Ident(name)) = kind(j) {
                        macro_name = Some((name.to_string(), None));
                        open_body(
                            &mut scope,
                            &mut in_body,
                            name,
                            tok.start as usize,
                            &mut facts,
                        );
                        i = j + 1;
                        continue;
                    }
                } else if !is(ident, OPCODES) {
                    // A macro invocation. Lower-case words outside the list
                    // are x86 or ARM instructions far more often than macros,
                    // so they make no call; whether an invocation defines its
                    // argument is decided later, against the macros the tree
                    // defines, so every candidate is kept.
                    if ident.chars().any(|c| c.is_ascii_uppercase() || c == '_')
                        && calls.allows(ident)
                    {
                        scope.record_call(ident, &mut facts);
                    }
                    if let Some(TokenKind::Ident(arg)) = kind(j) {
                        facts.macro_calls.push((
                            ident.to_string(),
                            arg.to_string(),
                            line_of(tok.start as usize),
                        ));
                    }
                }
                scope.on_word(ident);
            }
            TokenKind::Ident(ident) => scope.on_word(ident),
            TokenKind::Number(num) => scope.on_word(num),
            _ => {}
        }
        i += 1;
    }

    settle(&mut routine, &mut last, None, calls, &mut scope, &mut facts);
    scope.finish(src.len(), &mut facts);
    facts
}

/// How a routine's last statement leaves it.
enum Exit {
    Flow,
    Ends,
    Jump(String),
    Other,
}

/// Closes the open routine's flow as the next label (`next`) or the end of
/// the routines begins: a routine whose last instruction runs on falls into
/// that label, which is a call into it like a branch would be.
fn settle(
    routine: &mut Option<String>,
    last: &mut Exit,
    next: Option<&str>,
    calls: &crate::rules::Calls,
    scope: &mut ScopeStack,
    facts: &mut FileFacts,
) {
    if let Some(from) = routine.take() {
        match (std::mem::replace(last, Exit::Other), next) {
            (Exit::Flow, Some(to)) if calls.allows(to) => {
                scope.record_call(to, facts);
                facts.continues_into.push((from, to.to_string()));
            }
            (Exit::Jump(to), _) => facts.continues_into.push((from, to)),
            _ => {}
        }
    }
    *last = Exit::Other;
}
