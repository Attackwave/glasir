//! Language-specific syntax; lexical and call rules come from the registry.
//!
//! Smali writes every declaration as a dot-directive — `.method`, `.class`,
//! `.field` — and the lexer emits the dot as a symbol of its own, so no
//! keyword in a `[generic]` table can ever match one. Measured with a table:
//! zero definitions and every reference on `<module>`.
//!
//! A call is `invoke-*` followed by a method reference `LClass;->name(...)`,
//! so the name worth recording is the one after `->`, not the invoke opcode.

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
            // A directive: `.` immediately followed by its word.
            TokenKind::Symbol('.') => {
                if let Some(TokenKind::Ident(word)) = tokens.get(i + 1).map(|t| &t.kind) {
                    match *word {
                        "method" | "class" => {
                            // Modifiers (`public`, `static`, …) sit between the
                            // directive and the name; the name is the last word
                            // before the parameter list or the end of the line.
                            let mut j = i + 2;
                            let mut name: Option<&str> = None;
                            while j < tokens.len() {
                                match &tokens[j].kind {
                                    TokenKind::Ident(w) => name = Some(w),
                                    TokenKind::Newline | TokenKind::Symbol('(') => break,
                                    _ => {}
                                }
                                j += 1;
                            }
                            if let Some(name) = name {
                                scope.open_definition(
                                    name,
                                    tok.start as usize,
                                    *word == "method",
                                    &mut facts,
                                );
                                scope.on_word(name);
                                scope.on_open_delimiter();
                                i = j;
                                continue;
                            }
                        }
                        "end" => {
                            scope.on_close_delimiter(tok.end as usize, &mut facts);
                            i += 2;
                            continue;
                        }
                        _ => {}
                    }
                }
                i += 1;
                continue;
            }
            // `LClass;->name(...)`: the callee is the word after the arrow.
            TokenKind::DoubleSymbol("->") => {
                if let Some(TokenKind::Ident(name)) = tokens.get(i + 1).map(|t| &t.kind) {
                    if calls.allows(name) {
                        scope.record_call(name, &mut facts);
                    }
                    scope.on_word(name);
                    i += 2;
                    continue;
                }
                i += 1;
                continue;
            }
            TokenKind::Ident(ident) => {
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
