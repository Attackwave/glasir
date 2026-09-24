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
    let mut in_let_body = false;
    let mut expect_callee = false;
    let mut paren_expect_callee: Vec<bool> = Vec::new();

    // Helper to check if a `let` token at index `pos` is a top-level (or module-level) definition,
    // i.e., it does NOT have a corresponding `in` before the next top-level item/end.
    let is_top_level_let = |pos: usize| -> bool {
        let mut depth = 0;
        let mut k = pos + 1;
        while k < tokens.len() {
            match &tokens[k].kind {
                TokenKind::Symbol('(' | '[' | '{') => depth += 1,
                TokenKind::Symbol(')' | ']' | '}') => {
                    if depth > 0 {
                        depth -= 1;
                    }
                }
                TokenKind::Ident("struct" | "sig" | "begin") => depth += 1,
                TokenKind::Ident("end") => {
                    if depth > 0 {
                        depth -= 1;
                    } else {
                        return true;
                    }
                }
                TokenKind::Ident("in") if depth == 0 => {
                    return false;
                }
                TokenKind::Ident("let" | "type" | "module" | "val" | "exception") if depth == 0 => {
                    return true;
                }
                TokenKind::DoubleSymbol(";;") if depth == 0 => {
                    return true;
                }
                _ => {}
            }
            k += 1;
        }
        true
    };

    while i < tokens.len() {
        let tok = &tokens[i];
        match &tok.kind {
            TokenKind::DocComment(text) | TokenKind::BlockComment(text) => {
                scope.push_comment(text);
                i += 1;
                continue;
            }
            TokenKind::Newline => {
                i += 1;
                continue;
            }
            TokenKind::Symbol('=') => {
                in_let_body = true;
                expect_callee = true;
                i += 1;
                continue;
            }
            TokenKind::Symbol('(' | '[' | '{') => {
                paren_expect_callee.push(expect_callee);
                expect_callee = true;
                i += 1;
                continue;
            }
            TokenKind::Symbol(')' | ']' | '}') => {
                let _ = paren_expect_callee.pop();
                expect_callee = false;
                i += 1;
                continue;
            }
            TokenKind::Symbol('.') => {
                scope.on_receiver();
                expect_callee = true;
                i += 1;
                continue;
            }
            TokenKind::DoubleSymbol("|>") => {
                expect_callee = true;
                scope.had_receiver = false;
                i += 1;
                continue;
            }
            TokenKind::Symbol(c) if matches!(*c, '+' | '-' | '*' | '/' | '^' | '@' | ',' | ';') => {
                expect_callee = true;
                i += 1;
                continue;
            }
            TokenKind::DoubleSymbol(s)
                if matches!(
                    *s,
                    "==" | "!=" | "<=" | ">=" | "||" | "&&" | "::" | "->" | ":="
                ) =>
            {
                expect_callee = true;
                i += 1;
                continue;
            }
            TokenKind::Ident(ident) => {
                let is_keyword = matches!(
                    *ident,
                    "let"
                        | "rec"
                        | "in"
                        | "and"
                        | "type"
                        | "match"
                        | "with"
                        | "fun"
                        | "function"
                        | "if"
                        | "then"
                        | "else"
                        | "module"
                        | "open"
                        | "struct"
                        | "sig"
                        | "end"
                        | "val"
                        | "exception"
                        | "for"
                        | "to"
                        | "do"
                        | "done"
                        | "while"
                        | "begin"
                        | "try"
                        | "as"
                );

                match *ident {
                    "let" | "val" => {
                        let start_byte = tok.start;
                        let mut j = i + 1;
                        if j < tokens.len() && tokens[j].kind == TokenKind::Ident("rec") {
                            j += 1;
                        }
                        if j < tokens.len() {
                            if let TokenKind::Ident(name) = tokens[j].kind {
                                if is_top_level_let(i) {
                                    scope.close_definitions_at_or_above(
                                        scope.depth,
                                        start_byte as usize,
                                        &mut facts,
                                    );
                                    scope.open_definition_with_body_docs(
                                        name,
                                        start_byte as usize,
                                        true,
                                        true,
                                        &mut facts,
                                    );
                                    scope.on_word(name);
                                    in_let_body = false;
                                    expect_callee = false;
                                    // Advance to '=' if present
                                    let mut k = j + 1;
                                    while k < tokens.len()
                                        && tokens[k].kind != TokenKind::Symbol('=')
                                        && tokens[k].kind != TokenKind::Newline
                                    {
                                        k += 1;
                                    }
                                    if k < tokens.len() && tokens[k].kind == TokenKind::Symbol('=')
                                    {
                                        in_let_body = true;
                                        expect_callee = true;
                                        i = k + 1;
                                        continue;
                                    }
                                    i = j + 1;
                                    continue;
                                } else {
                                    // Local let binding: let x = ...
                                    let mut k = j + 1;
                                    while k < tokens.len()
                                        && tokens[k].kind != TokenKind::Symbol('=')
                                        && tokens[k].kind != TokenKind::Newline
                                    {
                                        k += 1;
                                    }
                                    if k < tokens.len() && tokens[k].kind == TokenKind::Symbol('=')
                                    {
                                        expect_callee = true;
                                        i = k + 1;
                                        continue;
                                    }
                                }
                            }
                        }
                    }
                    "type" | "exception" => {
                        let start_byte = tok.start;
                        if i + 1 < tokens.len() {
                            if let TokenKind::Ident(name) = tokens[i + 1].kind {
                                scope.close_definitions_at_or_above(
                                    scope.depth,
                                    start_byte as usize,
                                    &mut facts,
                                );
                                scope.open_definition_with_body_docs(
                                    name,
                                    start_byte as usize,
                                    false,
                                    false,
                                    &mut facts,
                                );
                                scope.on_word(name);
                                in_let_body = false;
                                expect_callee = false;
                                i += 2;
                                continue;
                            }
                        }
                    }
                    "module" => {
                        let start_byte = tok.start;
                        let mut j = i + 1;
                        if j < tokens.len() && tokens[j].kind == TokenKind::Ident("type") {
                            j += 1;
                        }
                        if j < tokens.len() {
                            if let TokenKind::Ident(name) = tokens[j].kind {
                                scope.close_definitions_at_or_above(
                                    scope.depth,
                                    start_byte as usize,
                                    &mut facts,
                                );
                                scope.open_definition_with_body_docs(
                                    name,
                                    start_byte as usize,
                                    true,
                                    false,
                                    &mut facts,
                                );
                                scope.on_word(name);
                                in_let_body = false;
                                expect_callee = false;
                                i = j + 1;
                                continue;
                            }
                        }
                    }
                    "struct" | "sig" | "begin" => {
                        scope.on_open_delimiter();
                    }
                    "end" => {
                        scope.on_close_delimiter(tok.end as usize, &mut facts);
                    }
                    _ => {
                        if in_let_body && !is_keyword {
                            if scope.had_receiver {
                                if calls.allows(ident) {
                                    scope.record_call(ident, &mut facts);
                                }
                                expect_callee = false;
                            } else if expect_callee {
                                let j = i + 1;
                                let is_call = if j < tokens.len() {
                                    matches!(
                                        tokens[j].kind,
                                        TokenKind::Ident(_)
                                            | TokenKind::Symbol('(' | '[' | '{' | '~' | '?')
                                            | TokenKind::StringLit(_)
                                            | TokenKind::Number(_)
                                    )
                                } else {
                                    false
                                };

                                if is_call && calls.allows(ident) {
                                    scope.record_call(ident, &mut facts);
                                }
                                expect_callee = false;
                            }
                            scope.on_word(ident);
                        } else if matches!(
                            *ident,
                            "in" | "then"
                                | "else"
                                | "do"
                                | "match"
                                | "with"
                                | "try"
                                | "function"
                                | "fun"
                        ) {
                            expect_callee = true;
                            scope.on_word(ident);
                        } else {
                            scope.on_word(ident);
                        }
                    }
                }
            }
            TokenKind::Number(num) => {
                expect_callee = false;
                scope.on_word(num);
            }
            TokenKind::StringLit(_) => {
                expect_callee = false;
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
