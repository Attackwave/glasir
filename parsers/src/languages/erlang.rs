//! Language-specific syntax; lexical and call rules come from the registry.

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
            TokenKind::Symbol(':') => {
                scope.on_receiver();
                i += 1;
                continue;
            }
            TokenKind::Symbol('-') => {
                let start_byte = tok.start;
                if i + 1 < tokens.len() {
                    if let TokenKind::Ident(attr) = tokens[i + 1].kind {
                        match attr {
                            "module" => {
                                if i + 3 < tokens.len()
                                    && tokens[i + 2].kind == TokenKind::Symbol('(')
                                {
                                    if let TokenKind::Ident(mod_name) = tokens[i + 3].kind {
                                        scope.open_definition_with_body_docs(
                                            mod_name,
                                            start_byte as usize,
                                            false,
                                            false,
                                            &mut facts,
                                        );
                                        scope.on_word(mod_name);
                                        i += 4;
                                        continue;
                                    }
                                }
                            }
                            "record"
                                if i + 3 < tokens.len()
                                    && tokens[i + 2].kind == TokenKind::Symbol('(') =>
                            {
                                if let TokenKind::Ident(rec_name) = tokens[i + 3].kind {
                                    scope.open_definition_with_body_docs(
                                        rec_name,
                                        start_byte as usize,
                                        false,
                                        false,
                                        &mut facts,
                                    );
                                    scope.on_word(rec_name);
                                    i += 4;
                                    continue;
                                }
                            }
                            // Every other `-attribute(...)` is a directive,
                            // not a call: `-export([f/1])` and `-define(X, 1)`
                            // were being recorded as calls from `<module>`,
                            // which is what put the fixture at 29%. Skipping
                            // the parenthesised argument keeps a name inside
                            // it from being read as one either.
                            _ if i + 2 < tokens.len()
                                && tokens[i + 2].kind == TokenKind::Symbol('(') =>
                            {
                                let mut depth = 1;
                                i += 3;
                                while i < tokens.len() && depth > 0 {
                                    match tokens[i].kind {
                                        TokenKind::Symbol('(') => depth += 1,
                                        TokenKind::Symbol(')') => depth -= 1,
                                        _ => {}
                                    }
                                    i += 1;
                                }
                                continue;
                            }
                            _ => {}
                        }
                    }
                }
                i += 1;
                continue;
            }
            TokenKind::Symbol('.') => {
                scope.close_definitions_at_or_above(0, tok.end as usize, &mut facts);
                i += 1;
                continue;
            }
            TokenKind::Ident(ident) => {
                // Top-level function definition: fn_name(Args) -> ...
                if i + 1 < tokens.len() && tokens[i + 1].kind == TokenKind::Symbol('(') {
                    let mut k = i + 2;
                    let mut depth = 1;
                    while k < tokens.len() && depth > 0 {
                        if tokens[k].kind == TokenKind::Symbol('(') {
                            depth += 1;
                        } else if tokens[k].kind == TokenKind::Symbol(')') {
                            depth -= 1;
                        }
                        k += 1;
                    }
                    while k < tokens.len() && tokens[k].kind == TokenKind::Newline {
                        k += 1;
                    }
                    if k < tokens.len()
                        && (tokens[k].kind == TokenKind::DoubleSymbol("->")
                            || matches!(tokens[k].kind, TokenKind::Ident("when")))
                        && scope.open.is_empty()
                    {
                        let start_byte = tok.start;
                        scope.open_definition_with_body_docs(
                            *ident,
                            start_byte as usize,
                            true,
                            true,
                            &mut facts,
                        );
                        scope.on_word(ident);
                        i += 1;
                        continue;
                    }

                    if calls.allows(ident) {
                        scope.record_call(ident, &mut facts);
                    }
                }
                scope.on_word(ident);
            }
            _ => {
                if !matches!(tok.kind, TokenKind::StringLit(_) | TokenKind::Number(_)) {
                    scope.had_receiver = false;
                }
            }
        }
        i += 1;
    }

    scope.finish(src.len(), &mut facts);
    facts
}
