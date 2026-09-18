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
        let line_start_pos = src[..tok.start as usize].rfind('\n').map(|p| p + 1).unwrap_or(0);
        let current_line_indent = (tok.start as usize).saturating_sub(line_start_pos) as i32;

        match &tok.kind {
            TokenKind::DocComment(text) | TokenKind::LineComment(text) => {
                while scope.open.last().is_some_and(|o| o.depth > current_line_indent) {
                    scope.on_close_delimiter(tok.start as usize, &mut facts);
                }
                scope.depth = current_line_indent;
                scope.push_comment(text);
                i += 1;
                continue;
            }
            TokenKind::Newline => {
                i += 1;
                continue;
            }
            TokenKind::StringLit(s) => {
                if s.starts_with("\"\"\"") || s.starts_with("'''") {
                    scope.push_comment(s);
                }
                i += 1;
                continue;
            }
            TokenKind::Symbol('@') => {
                while i < tokens.len() && tokens[i].kind != TokenKind::Newline {
                    i += 1;
                }
                continue;
            }
            TokenKind::Symbol('.') => {
                scope.on_receiver();
                i += 1;
                continue;
            }
            TokenKind::Ident(ident) => {
                while scope.open.last().is_some_and(|o| o.depth > current_line_indent) {
                    scope.on_close_delimiter(tok.start as usize, &mut facts);
                }
                scope.depth = current_line_indent;

                match *ident {
                    "fn" | "def" => {
                        let start_byte = tok.start;
                        let mut j = i + 1;
                        while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                            j += 1;
                        }
                        if j < tokens.len() {
                            if let TokenKind::Ident(fn_name) = tokens[j].kind {
                                scope.open_definition_with_body_docs(fn_name, start_byte as usize, true, true, &mut facts);
                                scope.on_word(fn_name);
                                i = j + 1;
                                continue;
                            }
                        }
                    }
                    "struct" | "trait" => {
                        let start_byte = tok.start;
                        let mut j = i + 1;
                        while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                            j += 1;
                        }
                        if j < tokens.len() {
                            if let TokenKind::Ident(name) = tokens[j].kind {
                                scope.open_definition_with_body_docs(name, start_byte as usize, true, false, &mut facts);
                                scope.on_word(name);
                                i = j + 1;
                                continue;
                            }
                        }
                    }
                    "alias" => {
                        let start_byte = tok.start;
                        let mut j = i + 1;
                        while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                            j += 1;
                        }
                        if j < tokens.len() {
                            if let TokenKind::Ident(name) = tokens[j].kind {
                                scope.open_definition_with_body_docs(name, start_byte as usize, false, false, &mut facts);
                                scope.on_word(name);
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
