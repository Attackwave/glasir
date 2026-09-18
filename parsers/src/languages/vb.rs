//! Language-specific syntax; lexical and call rules come from the registry.
//!
//! VB.NET is case-insensitive: definitions and the call exclusion are matched
//! on the lower-cased identifier.

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
            // `<Assembly: AssemblyTitle("x")>` is a declaration, not a call.
            // 129 of 469 VB files in the sample corpus are a generated
            // `AssemblyInfo.vb` that is nothing but these, and each attribute
            // was recorded as a call from `<module>`.
            TokenKind::Symbol('<') => {
                let mut depth = 1;
                i += 1;
                while i < tokens.len() && depth > 0 {
                    match tokens[i].kind {
                        TokenKind::Symbol('<') => depth += 1,
                        TokenKind::Symbol('>') => depth -= 1,
                        TokenKind::Newline => break,
                        _ => {}
                    }
                    i += 1;
                }
                continue;
            }
            TokenKind::Symbol('.') | TokenKind::DoubleSymbol("?.") => {
                let is_self = matches!(
                    scope.last_word.as_deref().map(|s| s.to_ascii_lowercase()),
                    Some(ref s) if s == "me" || s == "mybase" || s == "myclass"
                );
                scope.had_receiver = !is_self;
                i += 1;
                continue;
            }
            TokenKind::Ident(ident) => {
                let lower = ident.to_ascii_lowercase();

                if lower == "end" {
                    if i + 1 < tokens.len() {
                        if let TokenKind::Ident(kind) = tokens[i + 1].kind {
                            let k_lower = kind.to_ascii_lowercase();
                            if matches!(k_lower.as_str(), "sub" | "function" | "class" | "module" | "structure" | "interface" | "property" | "namespace" | "enum") {
                                scope.on_close_delimiter(tokens[i + 1].end as usize, &mut facts);
                                i += 2;
                                continue;
                            }
                        }
                    }
                    i += 1;
                    continue;
                }

                if matches!(
                    lower.as_str(),
                    "public"
                        | "private"
                        | "protected"
                        | "friend"
                        | "shared"
                        | "overridable"
                        | "overrides"
                        | "mustoverride"
                        | "notoverridable"
                        | "readonly"
                        | "writeonly"
                        | "shadows"
                        | "partial"
                        | "async"
                        | "iterator"
                        | "default"
                        | "dim"
                        | "const"
                ) {
                    i += 1;
                    continue;
                }

                match lower.as_str() {
                    "namespace" => {
                        let start_byte = tok.start;
                        let mut j = i + 1;
                        let mut ns_name = String::new();
                        while j < tokens.len() && tokens[j].kind != TokenKind::Newline {
                            if let TokenKind::Ident(part) = tokens[j].kind {
                                ns_name.push_str(part);
                            } else if let TokenKind::Symbol('.') = tokens[j].kind {
                                ns_name.push('.');
                            }
                            j += 1;
                        }
                        if !ns_name.is_empty() {
                            scope.open_definition_with_body_docs(&ns_name, start_byte as usize, true, false, &mut facts);
                            scope.on_open_delimiter();
                            scope.on_word(&ns_name);
                            i = j;
                            continue;
                        }
                    }
                    "class" | "module" | "structure" | "interface" | "enum" => {
                        let start_byte = tok.start;
                        if i + 1 < tokens.len() {
                            if let TokenKind::Ident(name) = tokens[i + 1].kind {
                                scope.open_definition_with_body_docs(name, start_byte as usize, true, false, &mut facts);
                                scope.on_open_delimiter();
                                scope.on_word(name);
                                i += 2;
                                continue;
                            }
                        }
                    }
                    "sub" | "function" | "property" => {
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
                    _ => {
                        let mut j = i + 1;
                        while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                            j += 1;
                        }
                        let is_call = j < tokens.len() && tokens[j].kind == TokenKind::Symbol('(');
                        if is_call && calls.allows(&lower) {
                            scope.record_call(ident, &mut facts);
                        } else if !is_call {
                            scope.had_receiver = false;
                        }
                        scope.on_word(ident);
                    }
                }
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
