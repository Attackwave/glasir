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
            TokenKind::DocComment(text)
            | TokenKind::LineComment(text)
            | TokenKind::BlockComment(text) => {
                scope.push_comment(text);
                i += 1;
                continue;
            }
            TokenKind::Newline => {
                i += 1;
                continue;
            }
            TokenKind::Symbol('{') => {
                scope.on_open_delimiter();
                i += 1;
                continue;
            }
            TokenKind::Symbol('}') => {
                scope.on_close_delimiter(tok.end as usize, &mut facts);
                i += 1;
                continue;
            }
            TokenKind::DoubleSymbol("->")
            | TokenKind::DoubleSymbol("?->")
            | TokenKind::DoubleSymbol("::") => {
                scope.on_receiver();
                i += 1;
                continue;
            }
            TokenKind::Ident(ident) => {
                if matches!(
                    *ident,
                    "public"
                        | "protected"
                        | "private"
                        | "static"
                        | "final"
                        | "abstract"
                        | "readonly"
                ) {
                    i += 1;
                    continue;
                }

                match *ident {
                    "const" => {
                        if i + 1 < tokens.len() {
                            if let TokenKind::Ident(name) = tokens[i + 1].kind {
                                scope.open_statement_definition(
                                    name,
                                    tok.start as usize,
                                    &mut facts,
                                );
                                scope.on_word(name);
                                i += 2;
                                continue;
                            }
                        }
                    }
                    "class" | "interface" | "trait" | "enum" => {
                        let start_byte = tok.start;
                        if i + 1 < tokens.len() {
                            if let TokenKind::Ident(name) = tokens[i + 1].kind {
                                scope.open_definition_with_body_docs(
                                    name,
                                    start_byte as usize,
                                    true,
                                    false,
                                    &mut facts,
                                );
                                scope.on_word(name);
                                i += 2;
                                continue;
                            }
                        }
                    }
                    "function" | "fn" => {
                        let start_byte = tok.start;
                        if i + 1 < tokens.len() {
                            if let TokenKind::Ident(name) = tokens[i + 1].kind {
                                scope.open_definition_with_body_docs(
                                    name,
                                    start_byte as usize,
                                    true,
                                    true,
                                    &mut facts,
                                );
                                scope.on_word(name);
                                i += 2;
                                continue;
                            }
                        }
                    }
                    "namespace" => {
                        let start_byte = tok.start;
                        let mut j = i + 1;
                        let mut ns_name = String::new();
                        while j < tokens.len()
                            && tokens[j].kind != TokenKind::Symbol(';')
                            && tokens[j].kind != TokenKind::Symbol('{')
                            && tokens[j].kind != TokenKind::Newline
                        {
                            if let TokenKind::Ident(part) = tokens[j].kind {
                                ns_name.push_str(part);
                            } else if let TokenKind::Symbol('\\') = tokens[j].kind {
                                ns_name.push('\\');
                            }
                            j += 1;
                        }
                        if !ns_name.is_empty() {
                            scope.open_definition_with_body_docs(
                                &ns_name,
                                start_byte as usize,
                                false,
                                false,
                                &mut facts,
                            );
                            scope.on_word(&ns_name);
                            i = j;
                            continue;
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
