//! Language-specific syntax; lexical and call rules come from the registry.
//!
//! Assembly defines by *shape*: a label is a name at the start of a line
//! followed by a colon, with no keyword anywhere — the same shape Make uses,
//! and the reason neither can be driven by a keyword table.
//!
//! A call is `call name`, which is an opcode and its operand rather than
//! `name(`, so the generic scanner's call test finds nothing either.

use crate::facts::FileFacts;
use crate::lexer::{CommentStyle, Lexer, TokenKind};
use crate::scope::ScopeStack;

pub(crate) fn parse(src: &str, style: CommentStyle<'_>, calls: &crate::rules::Calls) -> FileFacts {
    let mut facts = FileFacts::new();
    let mut lexer = Lexer::new(src, style);
    let tokens = lexer.collect_all_tokens();

    let mut i = 0;
    let mut scope = ScopeStack::new();

    while i < tokens.len() {
        let tok = &tokens[i];
        match &tok.kind {
            TokenKind::DocComment(text) | TokenKind::LineComment(text) => {
                scope.push_comment(text);
                i += 1;
                continue;
            }
            TokenKind::Newline => {
                i += 1;
                continue;
            }
            TokenKind::Ident(ident) => {
                let at_column_zero = tok.start == 0
                    || src
                        .as_bytes()
                        .get(tok.start as usize - 1)
                        .is_some_and(|b| *b == b'\n');

                // `name:` at column zero is a label, which is this language's
                // only definition. A local label (`.over:`) belongs to the
                // routine above it and is deliberately not one.
                if at_column_zero
                    && tokens.get(i + 1).map(|t| &t.kind) == Some(&TokenKind::Symbol(':'))
                {
                    scope.on_close_delimiter(tok.start as usize, &mut facts);
                    scope.open_definition(*ident, tok.start as usize, true, &mut facts);
                    scope.on_word(ident);
                    scope.on_open_delimiter();
                    i += 2;
                    continue;
                }

                // `call name` / `jmp name`: the operand is the callee.
                if matches!(*ident, "call" | "jmp" | "je" | "jne" | "jg" | "jl") {
                    if let Some(TokenKind::Ident(target)) = tokens.get(i + 1).map(|t| &t.kind) {
                        if calls.allows(target) {
                            scope.record_call(target, &mut facts);
                        }
                        scope.on_word(target);
                        i += 2;
                        continue;
                    }
                }
                scope.on_word(ident);
            }
            TokenKind::Number(num) => {
                scope.on_word(num);
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
