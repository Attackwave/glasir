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
            TokenKind::Ident(ident) => {
                let lower = ident.to_ascii_lowercase();

                if lower == "end" {
                    scope.on_close_delimiter(tok.end as usize, &mut facts);
                    i += 1;
                    continue;
                }

                match lower.as_str() {
                    "package" => {
                        let start_byte = tok.start;
                        let mut j = i + 1;
                        if j < tokens.len() && matches!(tokens[j].kind, TokenKind::Ident("body" | "BODY")) {
                            j += 1;
                        }
                        let mut pkg_name = String::new();
                        while j < tokens.len() && tokens[j].kind != TokenKind::Symbol(';') && tokens[j].kind != TokenKind::Newline && !matches!(tokens[j].kind, TokenKind::Ident("is" | "IS")) {
                            if let TokenKind::Ident(part) = tokens[j].kind {
                                pkg_name.push_str(part);
                            } else if let TokenKind::Symbol('.') = tokens[j].kind {
                                pkg_name.push('.');
                            }
                            j += 1;
                        }
                        if !pkg_name.is_empty() {
                            scope.open_definition_with_body_docs(&pkg_name, start_byte as usize, true, false, &mut facts);
                            scope.on_open_delimiter();
                            scope.on_word(&pkg_name);
                            i = j;
                            continue;
                        }
                    }
                    "procedure" | "function" | "entry" => {
                        let start_byte = tok.start;
                        if i + 1 < tokens.len() {
                            if let TokenKind::Ident(name) = tokens[i + 1].kind {
                                scope.open_definition_with_body_docs(name, start_byte as usize, true, true, &mut facts);
                                scope.on_open_delimiter();
                                scope.on_word(name);
                                i += 2;
                                continue;
                            }
                        }
                    }
                    "type" | "subtype" | "task" => {
                        let start_byte = tok.start;
                        if i + 1 < tokens.len() {
                            if let TokenKind::Ident(name) = tokens[i + 1].kind {
                                scope.open_definition_with_body_docs(name, start_byte as usize, false, false, &mut facts);
                                scope.on_word(name);
                                i += 2;
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
                            && (tokens[j].kind == TokenKind::Symbol('(') || scope.had_receiver)
                            && calls.allows(&lower)
                        {
                            scope.record_call(ident, &mut facts);
                        }
                        scope.on_word(ident);
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }

    scope.finish(src.len(), &mut facts);
    facts
}
