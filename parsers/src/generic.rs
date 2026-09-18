//! Declarative extraction for languages whose structure fits the shared scanner.

use crate::facts::FileFacts;
use crate::lexer::{CommentStyle, Lexer, TokenKind};
use crate::scope::ScopeStack;

/// How a definition keyword behaves once its name is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Opens {
    /// A body delimited by the language's block delimiters: `class`, `func`.
    Body,
    /// A body whose inner comments document it — a function, not a struct,
    /// since a field's comment does not describe the type enclosing it.
    BodyWithDocs,
    /// No body of its own; scope ends at the statement end. A constant with an
    /// initialiser, so the calls in it belong to the constant.
    Statement,
    /// A name with no scope at all: `package`, `module`, `namespace`.
    Bare,
    /// A body whose name may sit behind a parenthesised receiver, as Go writes
    /// `func (l *Ledger) Charge()`. Per keyword rather than per language: Go's
    /// `var (` opens a declaration group with the same shape and no name, and
    /// a language-wide flag consumed the group and took the *next*
    /// declaration's keyword as a name — measured on gin, `var (` at line 21
    /// produced a definition called `func`.
    BodyAfterReceiver,
}

/// Everything this scanner needs to read one language.
///
/// Every field is a list or a flag. That is the constraint, not an accident —
/// the moment one of them needs an `if`, the language belongs in its own file.
#[derive(Debug, Clone)]
pub struct LangSpec<'s> {
    /// Comment forms. `doc` is the subset that documents what follows.
    pub line_comment: &'s [&'s str],
    pub doc_comment: &'s [&'s str],
    pub block_comment: Option<(&'s str, &'s str)>,
    /// Whether `!`/`?` end an identifier or belong to it (Ruby's `empty?`).
    pub ident_suffix_marks: bool,
    /// Whether `-` may appear inside an identifier (a Lisp's `commit-entry`).
    pub ident_dashes: bool,

    /// Keywords opening a definition, with what they open. Read in order, so a
    /// keyword may appear once only.
    pub definitions: &'s [(&'s str, Opens)],
    /// Words that may precede a definition keyword and are skipped:
    /// `pub`, `static`, `private`, `async`.
    pub modifiers: &'s [&'s str],
    /// Words that look like a call — `name(` — and are not in this language.
    pub not_a_call: &'s [&'s str],
    /// Tokens that make the *following* name a call through a receiver, so the
    /// same-file rule does not bind `Instant::now()` to a local `now`.
    pub receivers: &'s [&'s str],

    /// The token closing a block where the language writes a word rather than
    /// a brace: Ruby's and Lua's `end`, Bash's `fi`.
    pub block_end: &'s [&'s str],
    /// Words opening a block that is not a definition, so `end` can close the
    /// right thing: Ruby's `do`, `begin`, `if`.
    pub block_open: &'s [&'s str],
    /// Whether a `{` opens and a `}` closes a block. False for a language that
    /// uses only `block_end`.
    pub braces: bool,
    /// Whether a name reached through a receiver is a call even with no
    /// argument list. Ruby writes `@entries.sum` and Elixir `list |> length`;
    /// C cannot, because `p.field` would then be a call. A flag rather than a
    /// list: it is a property of the language's grammar, and the alternative —
    /// naming every parenthesis-free method — is unbounded.
    pub bare_receiver_calls: bool,
    /// Whether `SCREAMING_CASE = value` is a constant definition. Ruby, Python
    /// and Bash mark a constant by casing alone — there is no keyword to put in
    /// `definitions`, and without this every one of them is missed: measured on
    /// thor, four constants per file and, with them, the calls in their
    /// initialisers, which fell to `<module>` instead.
    pub constant_by_case: bool,
}

impl LangSpec<'_> {
    /// Every field a caller does not set. Chosen so a C-shaped language needs
    /// only its keywords.
    pub const DEFAULT: LangSpec<'static> = LangSpec {
        line_comment: &["//"],
        doc_comment: &["///"],
        block_comment: Some(("/*", "*/")),
        ident_suffix_marks: false,
        ident_dashes: false,
        definitions: &[],
        modifiers: &[],
        not_a_call: &[],
        receivers: &[".", "::", "->"],
        block_end: &[],
        block_open: &[],
        braces: true,
        bare_receiver_calls: false,
        constant_by_case: false,
    };
}

pub fn parse(src: &str, spec: &LangSpec<'_>) -> FileFacts {
    let mut facts = FileFacts::new();
    let style = CommentStyle {
        line_comment_prefix: spec.line_comment,
        doc_comment_prefix: spec.doc_comment,
        block_comment_start: spec.block_comment.map(|(s, _)| s),
        block_comment_end: spec.block_comment.map(|(_, e)| e),
        ident_suffix_marks: spec.ident_suffix_marks,
        ident_dashes: spec.ident_dashes,
    };
    let mut lexer = Lexer::new(src, style);
    let tokens = lexer.collect_all_tokens();

    let mut i = 0;
    let mut scope = ScopeStack::new();
    // Whether nothing but whitespace precedes this token on its line.
    let mut at_line_start = true;
    // Where the current run of modifiers began, so a definition's range starts
    // at `abstract contract C` rather than at `contract`. The range is what
    // `get_code_snippet` hands back, so a modifier dropped from it returns
    // source that does not compile on its own.
    let mut modifier_start: Option<usize> = None;

    while i < tokens.len() {
        let tok = &tokens[i];
        // Read before any branch consumes the token: every `continue` below
        // would otherwise skip the update and leave the flag set for the whole
        // line.
        let starts_statement = std::mem::take(&mut at_line_start);
        match &tok.kind {
            TokenKind::DocComment(text)
            | TokenKind::LineComment(text)
            | TokenKind::BlockComment(text) => {
                scope.push_comment(text);
            }
            TokenKind::Newline => {
                at_line_start = true;
                i += 1;
                continue;
            }
            TokenKind::Symbol('{') if spec.braces => scope.on_open_delimiter(),
            TokenKind::Symbol('}') if spec.braces => {
                scope.on_close_delimiter(tok.end as usize, &mut facts)
            }
            TokenKind::Symbol(';') => {
                // A statement boundary ends any receiver context. Without
                // this, `a.b();` leaves the flag set and the *next*
                // statement's first call is recorded as receiver-qualified —
                // measured on OpenZeppelin, `emit OperationExecuted(...)`
                // after a method call was marked as reached through one, which
                // is exactly what the same-file rule keys on.
                scope.had_receiver = false;
                scope.on_statement_end(tok.end as usize, &mut facts);
            }
            TokenKind::Symbol('.') if spec.receivers.contains(&".") => scope.on_receiver(),
            TokenKind::DoubleSymbol(s) if spec.receivers.contains(s) => scope.on_receiver(),
            TokenKind::Number(num) => scope.on_word(num),
            TokenKind::Ident(ident) => {
                // `MAX = 3` — a constant marked by casing rather than by a
                // keyword. Checked before the definition keywords, since the
                // name is the token itself rather than the one after it.
                if spec.constant_by_case
                    && crate::scope::is_screaming_case(ident)
                    && tokens
                        .get(i + 1)
                        .is_some_and(|t| t.kind == TokenKind::Symbol('='))
                {
                    scope.open_statement_definition(*ident, tok.start as usize, &mut facts);
                    scope.on_word(ident);
                    i += 1;
                    continue;
                }
                if spec.modifiers.contains(ident) {
                    modifier_start.get_or_insert(tok.start as usize);
                    i += 1;
                    continue;
                }
                // A word-delimited block, for a language writing `end` rather
                // than `}`. Checked before the definition keywords, since Ruby
                // opens a block with `do` and closes a definition with `end`.
                if spec.block_end.contains(ident) {
                    scope.on_close_delimiter(tok.end as usize, &mut facts);
                    i += 1;
                    continue;
                }
                if spec.block_open.contains(ident) {
                    // `return x if y` is a modifier, not a block: Ruby writes
                    // both with the same word, and only the leading one opens
                    // something to close. Counting the modifier leaves the
                    // depth permanently one too deep, after which every later
                    // call in the file is attributed to `<module>` — measured
                    // on thor, 876 edges moved off their own method.
                    if starts_statement {
                        scope.on_open_delimiter();
                    }
                    i += 1;
                    continue;
                }
                if let Some((_, opens)) = spec.definitions.iter().find(|(k, _)| k == ident) {
                    let start = modifier_start.take().unwrap_or(tok.start as usize);
                    let mut j = i + 1;
                    // `func (l *Ledger) Charge()` — the receiver group sits
                    // between the keyword and the name, and is not the name.
                    if *opens == Opens::BodyAfterReceiver
                        && j < tokens.len()
                        && tokens[j].kind == TokenKind::Symbol('(')
                    {
                        let mut depth = 1;
                        j += 1;
                        while j < tokens.len() && depth > 0 {
                            match tokens[j].kind {
                                TokenKind::Symbol('(') => depth += 1,
                                TokenKind::Symbol(')') => depth -= 1,
                                _ => {}
                            }
                            j += 1;
                        }
                    }
                    while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                        j += 1;
                    }
                    if let Some(TokenKind::Ident(name)) = tokens.get(j).map(|t| &t.kind) {
                        let name = *name;
                        match opens {
                            Opens::Body | Opens::BodyAfterReceiver => scope
                                .open_definition_with_body_docs(
                                    name,
                                    start,
                                    true,
                                    *opens == Opens::BodyAfterReceiver,
                                    &mut facts,
                                ),
                            Opens::BodyWithDocs => scope.open_definition_with_body_docs(
                                name, start, true, true, &mut facts,
                            ),
                            Opens::Statement => {
                                scope.open_statement_definition(name, start, &mut facts)
                            }
                            Opens::Bare => scope.open_definition_with_body_docs(
                                name, start, false, false, &mut facts,
                            ),
                        }
                        scope.on_word(name);
                        i = j + 1;
                        continue;
                    }
                    // A keyword with no name after it is not a definition —
                    // Go's `type (` group, Rust's `impl Trait for`. It may
                    // still open a block: Ruby's `class << self` reopens the
                    // singleton class, defining no name while grouping the
                    // methods that follow, and skipping it outright let the
                    // matching `end` close the real class instead. Measured on
                    // thor, that put 876 edges on `<module>`.
                    if !spec.braces
                        && matches!(
                            opens,
                            Opens::Body | Opens::BodyWithDocs | Opens::BodyAfterReceiver
                        )
                    {
                        scope.on_open_delimiter();
                    }
                    i += 1;
                    continue;
                }
                modifier_start = None;
                // A call is `name (`. Newlines are skipped so a call whose
                // arguments start on the next line still counts.
                let mut j = i + 1;
                while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                    j += 1;
                }
                let called = tokens
                    .get(j)
                    .is_some_and(|t| t.kind == TokenKind::Symbol('('))
                    || (spec.bare_receiver_calls && scope.had_receiver);
                if called && !spec.not_a_call.contains(ident) {
                    scope.record_call(ident, &mut facts);
                }
                scope.on_word(ident);
            }
            _ => {
                if !matches!(tok.kind, TokenKind::StringLit(_)) {
                    scope.had_receiver = false;
                }
            }
        }
        i += 1;
    }

    scope.finish(src.len(), &mut facts);
    facts
}
