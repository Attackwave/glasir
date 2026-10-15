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
    let mut track = Track::default();
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
                if at_column_zero(tok) {
                    track.local_seen = true;
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
                        let (_, end) = line_of(start);
                        if let Some(value) = operands(
                            src.get(tokens[i + 1].end as usize..end as usize)
                                .unwrap_or(""),
                        )
                        .first()
                        {
                            facts.constants.push((ident.to_string(), value.to_string()));
                        }
                        scope.open_statement_definition(*ident, start, &mut facts);
                        scope.on_word(ident);
                        in_constant = true;
                        i += 2;
                        seen_opcode = true;
                        continue;
                    }
                    Some(TokenKind::Symbol('=')) => {
                        let (_, end) = line_of(start);
                        if let Some(value) = operands(
                            src.get(tokens[i + 1].end as usize..end as usize)
                                .unwrap_or(""),
                        )
                        .first()
                        {
                            facts.constants.push((ident.to_string(), value.to_string()));
                        }
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
                        if let (Some(from), Exit::Flow) = (&routine, &last) {
                            track.pass(from, ident, &mut facts);
                        }
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
                        track.open();
                        i += 2;
                        continue;
                    }
                    Some(TokenKind::Newline | TokenKind::Ident(_)) | None
                        if !is(ident, OPCODES) && !is(ident, BRANCHES) =>
                    {
                        if let (Some(from), Exit::Flow) = (&routine, &last) {
                            track.pass(from, ident, &mut facts);
                        }
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
                        track.open();
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
                let mut size = None;
                if kind(j) == Some(&TokenKind::Symbol('.'))
                    && matches!(kind(j + 1), Some(TokenKind::Ident(s)) if s.len() == 1)
                {
                    if let Some(TokenKind::Ident(s)) = kind(j + 1) {
                        size = Some(s.to_ascii_lowercase());
                    }
                    j += 2;
                }
                if let Some(from) = &routine {
                    let field = tokens[j - 1].end as usize;
                    let (_, line_end) = line_of(tok.start as usize);
                    let ops = operands(src.get(field..line_end as usize).unwrap_or(""));
                    let line = 1 + src.as_bytes()[..tok.start as usize]
                        .iter()
                        .filter(|&&b| b == b'\n')
                        .count() as u32;
                    track.step(
                        from,
                        &ident.to_ascii_lowercase(),
                        size.as_deref(),
                        &ops,
                        line,
                        matches!(last, Exit::Flow | Exit::Ends),
                        calls,
                        &mut scope,
                        &mut facts,
                    );
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
    track.finish(&mut facts);
    facts
}

/// Where a register's value came from, as far as one routine can tell: a
/// label's address (`LEA table,A0`), then the fields read along the way
/// (`MOVE.L MN_OPS(A1),A2`, `MOVE.L OP_READ(A2),A2`). An indexed read keeps
/// the chain, as an entry of what it pointed at.
#[derive(Clone)]
struct Origin {
    label: Option<String>,
    fields: Vec<String>,
}

impl Origin {
    fn label(name: &str) -> Self {
        Origin {
            label: Some(name.to_string()),
            fields: Vec::new(),
        }
    }

    fn field(name: &str) -> Self {
        Origin {
            label: None,
            fields: vec![name.to_string()],
        }
    }

    /// The value read at `off` from where this points.
    fn read(&self, off: Option<&str>) -> Self {
        let mut next = self.clone();
        next.fields.extend(off.map(str::to_string));
        next
    }
}

/// Register contents through one routine, read straight down: enough to
/// see which table or field an indirect call goes through, without
/// following branches. What a routine was handed in a register is not known
/// here; calls through it and stores of it are reported as facts about the
/// parameter, resolved across files later.
#[derive(Default)]
struct Track {
    regs: std::collections::HashMap<String, Origin>,
    /// Registers the open label has written; one it uses without writing it
    /// is a parameter.
    written: std::collections::HashSet<String>,
    /// Per label: (has an instruction, has data).
    kinds: std::collections::HashMap<String, (bool, bool)>,
    /// Calls made for `DC.L name` entries, by index into `facts.calls`, and
    /// the labels holding one in their own body.
    entries: Vec<usize>,
    with_entries: std::collections::HashSet<String>,
    /// A local label (`.table`) has opened since the label: its entries
    /// belong to a routine's private table, not to a table of its own.
    local_seen: bool,
    /// Byte offset into the open label's data, while it is known.
    offset: Option<u32>,
}

const REGISTERS: [&str; 15] = [
    "d0", "d1", "d2", "d3", "d4", "d5", "d6", "d7", "a0", "a1", "a2", "a3", "a4", "a5", "a6",
];

impl Track {
    /// A new label: nothing is known of its registers or its data yet.
    fn open(&mut self) {
        self.regs.clear();
        self.written.clear();
        self.local_seen = false;
        self.offset = Some(0);
    }

    /// Control passes from `from` to `to` — a call, a branch, or falling into
    /// the next label — and takes the registers with it.
    fn pass(&self, from: &str, to: &str, facts: &mut FileFacts) {
        for (reg, origin) in &self.regs {
            facts.passes.push((
                from.to_string(),
                to.to_string(),
                reg.clone(),
                origin.label.clone(),
                origin.fields.clone(),
            ));
        }
        let mask = REGISTERS
            .iter()
            .enumerate()
            .filter(|(_, r)| !self.written.contains(**r))
            .fold(0u16, |m, (k, _)| m | 1 << k);
        if mask != 0 {
            facts
                .forwards
                .push((from.to_string(), to.to_string(), mask));
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn step(
        &mut self,
        routine: &str,
        op: &str,
        size: Option<&str>,
        ops: &[&str],
        line: u32,
        code: bool,
        calls: &crate::rules::Calls,
        scope: &mut ScopeStack,
        facts: &mut FileFacts,
    ) {
        let kind = self.kinds.entry(routine.to_string()).or_default();
        if code {
            kind.0 = true;
            self.offset = None;
        }
        if is(op, &["dc", "dcb", "ds", "db", "dw", "dd", "dq"]) {
            kind.1 = true;
            self.data(routine, op, size, ops, calls, scope, facts);
            return;
        }
        if op == "even" {
            self.offset = self.offset.map(|o| o + o % 2);
        } else if is(op, &["cnop", "align", "org", "rs", "so", "fo"]) {
            self.offset = None;
        }
        let dest = ops.last().map(|o| operand(o));
        match op {
            "lea" if ops.len() == 2 => {
                if let Some(Operand::Reg(d)) = dest {
                    let origin = match operand(ops[0]) {
                        Operand::Mem {
                            off, base: None, ..
                        }
                        | Operand::Mem { off, pc: true, .. } => ident(off).map(Origin::label),
                        Operand::Mem { base: Some(b), .. } => self.regs.get(&b).cloned(),
                        _ => None,
                    };
                    self.set(d, origin);
                }
            }
            "move" | "movea" if ops.len() == 2 => {
                let src = operand(ops[0]);
                match dest {
                    Some(Operand::Reg(d)) => {
                        let origin = match src {
                            Operand::Imm(x) => ident(x).map(Origin::label),
                            Operand::Reg(m) => self.regs.get(&m).cloned(),
                            // A field read through a known value extends its
                            // chain; a field of an unknown record starts one.
                            Operand::Mem {
                                off,
                                base: Some(b),
                                pc: false,
                            } => match self.regs.get(&b) {
                                Some(o) => Some(o.read(ident(off))),
                                None => ident(off).map(Origin::field),
                            },
                            Operand::Mem { off, .. } => ident(off).map(Origin::field),
                        };
                        self.set(d, origin);
                    }
                    Some(Operand::Mem { off, pc: false, .. }) => {
                        if let Some(field) = ident(off) {
                            match src {
                                Operand::Imm(x) => {
                                    if let Some(label) = ident(x) {
                                        facts.stores.push((field.to_string(), label.to_string()));
                                    }
                                }
                                Operand::Reg(m) => match self.regs.get(&m) {
                                    Some(Origin {
                                        label: Some(t),
                                        fields,
                                    }) if fields.is_empty() => {
                                        facts.stores.push((field.to_string(), t.clone()));
                                    }
                                    None if !self.written.contains(&m) => {
                                        facts.param_stores.push((
                                            routine.to_string(),
                                            m,
                                            field.to_string(),
                                        ));
                                    }
                                    _ => {}
                                },
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                }
            }
            _ if is(op, BRANCHES)
                && !matches!(
                    ops.first().map(|o| operand(o)),
                    Some(Operand::Mem {
                        base: Some(_),
                        pc: false,
                        ..
                    })
                ) =>
            {
                // Control goes to a label and takes the registers along.
                if let Some(Operand::Mem { off, .. }) = ops.first().map(|o| operand(o)) {
                    if let Some(to) = ident(off).filter(|t| !t.starts_with('.')) {
                        self.pass(routine, to, facts);
                    }
                }
            }
            "jsr" | "jmp" => {
                if let Some(Operand::Mem {
                    off,
                    base: Some(b),
                    pc: false,
                }) = ops.first().map(|o| operand(o))
                {
                    match self.regs.get(&b).map(|o| o.read(ident(off))) {
                        // A table local to this routine lists its targets
                        // as this routine's own entries already.
                        Some(Origin { label: Some(t), .. }) if t.starts_with('.') => {}
                        Some(o) => {
                            facts
                                .dispatch
                                .push((routine.to_string(), o.label, o.fields, line))
                        }
                        None if !self.written.contains(&b) => facts.param_calls.push((
                            routine.to_string(),
                            b,
                            ident(off).map(str::to_string).into_iter().collect(),
                            line,
                        )),
                        None => match ident(off) {
                            Some(f) => facts.dispatch.push((
                                routine.to_string(),
                                None,
                                vec![f.to_string()],
                                line,
                            )),
                            None => facts.indirect.push((routine.to_string(), line)),
                        },
                    }
                }
            }
            // A comparison or a test writes no register.
            "cmp" | "cmpa" | "cmpi" | "cmpm" | "cmp2" | "tst" | "btst" | "chk" | "chk2" => {}
            // Pointer arithmetic keeps an address register's origin.
            "adda" | "suba" | "addq" | "subq" | "add" | "sub" if matches!(&dest, Some(Operand::Reg(d)) if d.starts_with('a')) => {
                if let Some(Operand::Reg(d)) = dest {
                    self.written.insert(d);
                }
            }
            // A register list restored, or two registers swapped: forget.
            "movem" if !ops.last().is_some_and(|o| o.contains('(')) => {
                for r in REGISTERS {
                    self.set(r.to_string(), None);
                }
            }
            "exg" => {
                for o in ops {
                    if let Operand::Reg(r) = operand(o) {
                        self.set(r, None);
                    }
                }
            }
            _ => {
                if let Some(Operand::Reg(d)) = dest {
                    self.set(d, None);
                }
            }
        }
    }

    /// A data directive: pointer entries become calls, and their byte
    /// offset is kept while every size before them is known.
    #[allow(clippy::too_many_arguments)]
    fn data(
        &mut self,
        routine: &str,
        op: &str,
        size: Option<&str>,
        ops: &[&str],
        calls: &crate::rules::Calls,
        scope: &mut ScopeStack,
        facts: &mut FileFacts,
    ) {
        let unit = match size {
            Some("b") => 1,
            Some("l") => 4,
            _ => 2,
        };
        match op {
            "dc" => {
                for o in ops {
                    let name = (ident(o) == Some(*o)).then_some(*o);
                    if let (4, Some(name)) = (unit, name) {
                        // A pointer entry: the table, or the routine owning a
                        // local table, reaches what it lists.
                        if calls.allows(name) {
                            scope.record_call(name, facts);
                            self.entries.push(facts.calls.len() - 1);
                            if !self.local_seen {
                                self.with_entries.insert(routine.to_string());
                            }
                        }
                        if let (Some(at), false) = (self.offset, self.local_seen) {
                            facts
                                .slots
                                .push((routine.to_string(), at, name.to_string()));
                        }
                    }
                    let bytes = match (
                        unit,
                        o.strip_prefix('\'').and_then(|q| q.strip_suffix('\'')),
                    ) {
                        (1, Some(text)) => Some(text.replace("''", "'").len() as u32),
                        _ => Some(unit),
                    };
                    self.offset = self.offset.zip(bytes).map(|(a, b)| a + b);
                }
            }
            "ds" | "dcb" => {
                let count = ops.first().and_then(|n| number(n));
                self.offset = self.offset.zip(count).map(|(a, n)| a + n * unit);
            }
            _ => self.offset = None,
        }
    }

    fn set(&mut self, reg: String, origin: Option<Origin>) {
        self.written.insert(reg.clone());
        match origin {
            Some(o) => self.regs.insert(reg, o),
            None => self.regs.remove(&reg),
        };
    }

    /// Drops the entries naming this file's own data (a string, a buffer),
    /// and names the labels holding pointer entries of their own.
    fn finish(self, facts: &mut FileFacts) {
        let data: std::collections::HashSet<&str> = self
            .kinds
            .iter()
            .filter(|(_, &(code, data))| data && !code)
            .map(|(n, _)| n.as_str())
            .collect();
        let drop: std::collections::HashSet<usize> = self
            .entries
            .iter()
            .copied()
            .filter(|&k| data.contains(facts.calls[k].1.as_str()))
            .collect();
        if !drop.is_empty() {
            let mut k = 0;
            facts.calls.retain(|_| {
                k += 1;
                !drop.contains(&(k - 1))
            });
            let mut k = 0;
            facts.call_modules.retain(|_| {
                k += 1;
                !drop.contains(&(k - 1))
            });
            facts.slots.retain(|(_, _, to)| !data.contains(to.as_str()));
        }
        let kept: std::collections::HashSet<&str> = facts
            .calls
            .iter()
            .map(|(from, _, _)| from.as_str())
            .collect();
        let mut tables: Vec<String> = self
            .with_entries
            .into_iter()
            .filter(|t| kept.contains(t.as_str()))
            .collect();
        tables.sort();
        facts.tables = tables;
        facts.dispatch.sort();
        facts.dispatch.dedup();
    }
}

/// A plain number: decimal, `$` hex or `%` binary.
fn number(s: &str) -> Option<u32> {
    let s = s.trim();
    if let Some(h) = s.strip_prefix('$') {
        u32::from_str_radix(h, 16).ok()
    } else if let Some(b) = s.strip_prefix('%') {
        u32::from_str_radix(b, 2).ok()
    } else {
        s.parse().ok()
    }
}

/// One operand, as far as the tracking needs it.
enum Operand<'a> {
    Reg(String),
    Imm(&'a str),
    /// `off(base)`, `(base,Dn)`, `-(base)`, `off(PC)`, or an absolute `off`.
    Mem {
        off: &'a str,
        base: Option<String>,
        pc: bool,
    },
}

fn register(s: &str) -> Option<String> {
    let r = s.trim().to_ascii_lowercase();
    if r == "sp" {
        return Some("a7".into());
    }
    let b = r.as_bytes();
    (b.len() == 2 && (b[0] == b'a' || b[0] == b'd') && (b'0'..=b'7').contains(&b[1])).then_some(r)
}

fn operand(s: &str) -> Operand<'_> {
    let s = s.trim();
    if let Some(r) = register(s) {
        return Operand::Reg(r);
    }
    if let Some(imm) = s.strip_prefix('#') {
        return Operand::Imm(imm);
    }
    match (s.find('('), s.rfind(')')) {
        (Some(open), Some(close)) if close > open => {
            let first = s[open + 1..close].split(',').next().unwrap_or("");
            Operand::Mem {
                off: s[..open].trim_end_matches('-'),
                base: register(first),
                pc: first.trim().eq_ignore_ascii_case("pc"),
            }
        }
        _ => Operand::Mem {
            off: s,
            base: None,
            pc: false,
        },
    }
}

/// The name an operand starts with: `table`, `.local`, `kx_hooks` out of
/// `kx_hooks+KX_HOOK*4`; none for a number.
fn ident(s: &str) -> Option<&str> {
    let end = s
        .char_indices()
        .find(|&(k, c)| !(c.is_ascii_alphanumeric() || c == '_' || (c == '.' && k == 0)))
        .map_or(s.len(), |(k, _)| k);
    let name = &s[..end];
    let first = name.trim_start_matches('.').chars().next()?;
    (first.is_ascii_alphabetic() || first == '_').then_some(name)
}

/// The operand field of a statement: up to the first blank or `;` outside
/// parentheses and quotes, split at its top-level commas. A blank beside a
/// comma (`DC.L name,  handler`) is inside the field, not after it.
fn operands(rest: &str) -> Vec<&str> {
    let rest = rest.trim_start();
    let beside_comma =
        |k: usize| rest[..k].trim_end().ends_with(',') || rest[k..].trim_start().starts_with(',');
    let (mut depth, mut quote, mut end) = (0i32, None, rest.len());
    for (k, c) in rest.char_indices() {
        match (quote, c) {
            (None, '\'' | '"') => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '(') => depth += 1,
            (None, ')') => depth -= 1,
            (None, ';') => {
                end = k;
                break;
            }
            (None, c) if c.is_whitespace() && depth <= 0 && !beside_comma(k) => {
                end = k;
                break;
            }
            _ => {}
        }
    }
    let field = &rest[..end];
    let mut out = Vec::new();
    let (mut depth, mut from) = (0i32, 0);
    for (k, c) in field.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                out.push(field[from..k].trim());
                from = k + 1;
            }
            _ => {}
        }
    }
    if from < field.len() {
        out.push(field[from..].trim());
    }
    out
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
