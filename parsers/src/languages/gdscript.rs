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

                if matches!(*ident, "static" | "remote" | "master" | "puppet" | "sync") {
                    i += 1;
                    continue;
                }
                match *ident {
                    "class_name" => {
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
                    "class" => {
                        let start_byte = tok.start;
                        if i + 1 < tokens.len() {
                            if let TokenKind::Ident(name) = tokens[i + 1].kind {
                                scope.close_definitions_at_or_above(0, start_byte as usize, &mut facts);
                                scope.open_definition_with_body_docs(name, start_byte as usize, true, false, &mut facts);
                                scope.on_word(name);
                                i += 2;
                                continue;
                            }
                        }
                    }
                    "func" => {
                        let start_byte = tok.start;
                        if i + 1 < tokens.len() {
                            if let TokenKind::Ident(name) = tokens[i + 1].kind {
                                scope.close_definitions_at_or_above(1, start_byte as usize, &mut facts);
                                scope.open_definition_with_body_docs(name, start_byte as usize, true, true, &mut facts);
                                scope.on_word(name);
                                i += 2;
                                continue;
                            }
                        }
                    }
                    // **A `signal`, `enum`, `const` or class-level `var` is a
                    // declaration, and only `func` and `class` were read.**
                    // Measured on a real Godot project: 475 functions were
                    // found against 449 declarations that were not — 355
                    // `var`, 71 `const`, 21 `signal`, 2 `enum` — so nearly
                    // half the public surface of every node was missing and
                    // 24% of references had nothing to belong to. A signal is
                    // what another node connects to; a `const` is what a
                    // question about a tunable is phrased from.
                    "signal" | "enum" | "const" | "var" => {
                        // Only at file scope. Inside a function, `var x` is a
                        // local and a graph node nobody searches for — the
                        // same bound Lua's `local` needed, and a depth test is
                        // what expresses it.
                        let top_level = scope.depth == 0;
                        if top_level {
                            if let Some(TokenKind::Ident(name)) =
                                tokens.get(i + 1).map(|t| &t.kind)
                            {
                                let start_byte = tok.start;
                                scope.close_definitions_at_or_above(1, start_byte as usize, &mut facts);
                                // A statement, not a body: the declaration ends
                                // at its line, so a call in an initialiser
                                // belongs to it and the next `func` is not
                                // swallowed.
                                scope.open_statement_definition(*name, start_byte as usize, &mut facts);
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
                        if is_call && calls.allows(ident) {
                            scope.record_call(ident, &mut facts);
                        } else if !is_call {
                            scope.had_receiver = false;
                        }
                        scope.on_word(ident);
                    }
                }
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
