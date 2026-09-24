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
            TokenKind::Symbol('.') => {
                scope.on_receiver();
                i += 1;
                continue;
            }
            TokenKind::Symbol('@') => {
                i += 1;
                if i < tokens.len() {
                    if let TokenKind::Ident(attr) = tokens[i].kind {
                        if matches!(attr, "doc" | "moduledoc" | "typedoc") {
                            let mut j = i + 1;
                            while j < tokens.len() && tokens[j].kind != TokenKind::Newline {
                                if let TokenKind::StringLit(s) = tokens[j].kind {
                                    scope.push_comment(s);
                                    break;
                                }
                                j += 1;
                            }
                            i = j;
                            continue;
                        }
                    }
                    // `@max_retries 3` is how Elixir writes a module
                    // constant. Only at module level and only with a value —
                    // `@behaviour Foo` names a module, not a tunable.
                    if let TokenKind::Ident(attr) = tokens[i].kind {
                        let has_value = i + 1 < tokens.len()
                            && matches!(
                                tokens[i + 1].kind,
                                TokenKind::Number(_) | TokenKind::StringLit(_)
                            );
                        if has_value {
                            scope.open_statement_definition(attr, tok.start as usize, &mut facts);
                            scope.on_word(attr);
                            while i < tokens.len() && tokens[i].kind != TokenKind::Newline {
                                i += 1;
                            }
                            continue;
                        }
                    }
                }
                while i < tokens.len() && tokens[i].kind != TokenKind::Newline {
                    i += 1;
                }
                continue;
            }
            TokenKind::Ident(ident) => match *ident {
                "do" => {
                    scope.on_open_delimiter();
                }
                "end" => {
                    scope.on_close_delimiter(tok.end as usize, &mut facts);
                }
                "defmodule" | "defprotocol" | "defimpl" => {
                    let start_byte = tok.start;
                    let mut mod_name = String::new();
                    let mut j = i + 1;
                    while j < tokens.len() {
                        if let TokenKind::Ident(part) = tokens[j].kind {
                            mod_name.push_str(part);
                            if j + 1 < tokens.len() && tokens[j + 1].kind == TokenKind::Symbol('.')
                            {
                                mod_name.push('.');
                                j += 2;
                                continue;
                            }
                            j += 1;
                            break;
                        } else {
                            break;
                        }
                    }
                    if !mod_name.is_empty() {
                        scope.open_definition_with_body_docs(
                            &mod_name,
                            start_byte as usize,
                            true,
                            false,
                            &mut facts,
                        );
                        scope.on_word(&mod_name);
                        i = j;
                        continue;
                    }
                }
                "def" | "defp" | "defmacro" | "defguard" | "defdelegate" => {
                    let start_byte = tok.start;
                    let mut j = i + 1;
                    while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                        j += 1;
                    }
                    if j < tokens.len() {
                        if let TokenKind::Ident(fn_name) = tokens[j].kind {
                            scope.open_definition(fn_name, start_byte as usize, true, &mut facts);
                            scope.on_word(fn_name);
                            i = j + 1;
                            continue;
                        }
                    }
                }
                _ => {
                    let mut j = i + 1;
                    while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                        j += 1;
                    }
                    if j < tokens.len()
                        && tokens[j].kind == TokenKind::Symbol('(')
                        && calls.allows(ident)
                    {
                        scope.record_call(ident, &mut facts);
                    }
                    scope.on_word(ident);
                }
            },
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
