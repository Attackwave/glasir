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
    let mut in_equation = false;
    let mut expect_callee = false;
    let mut paren_expect_callee: Vec<bool> = Vec::new();

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
                in_equation = false;
                expect_callee = false;
                paren_expect_callee.clear();
                i += 1;
                continue;
            }
            TokenKind::Symbol('=') => {
                in_equation = true;
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
                i += 1;
                continue;
            }
            TokenKind::Symbol('$') => {
                expect_callee = true;
                i += 1;
                continue;
            }
            TokenKind::Symbol(c) if matches!(*c, '+' | '-' | '*' | '/' | '|' | ',' | ';') => {
                expect_callee = true;
                i += 1;
                continue;
            }
            TokenKind::DoubleSymbol(s)
                if matches!(
                    *s,
                    "++" | "==" | "/=" | "<=" | ">=" | "||" | "&&" | ">>" | "->"
                ) =>
            {
                expect_callee = true;
                i += 1;
                continue;
            }
            TokenKind::Ident(ident) => {
                let is_keyword = matches!(
                    *ident,
                    "module"
                        | "import"
                        | "data"
                        | "type"
                        | "newtype"
                        | "class"
                        | "instance"
                        | "deriving"
                        | "where"
                        | "let"
                        | "in"
                        | "do"
                        | "case"
                        | "of"
                        | "if"
                        | "then"
                        | "else"
                        | "default"
                        | "foreign"
                );

                if !in_equation {
                    match *ident {
                        "module" => {
                            let start_byte = tok.start;
                            if i + 1 < tokens.len() {
                                if let TokenKind::Ident(mod_name) = tokens[i + 1].kind {
                                    scope.close_definitions_at_or_above(
                                        scope.depth,
                                        start_byte as usize,
                                        &mut facts,
                                    );
                                    scope.open_definition(
                                        mod_name,
                                        start_byte as usize,
                                        false,
                                        &mut facts,
                                    );
                                    scope.on_word(mod_name);
                                    i += 2;
                                    continue;
                                }
                            }
                        }
                        "data" | "newtype" | "type" | "class" | "instance" => {
                            let start_byte = tok.start;
                            if i + 1 < tokens.len() {
                                if let TokenKind::Ident(type_name) = tokens[i + 1].kind {
                                    scope.close_definitions_at_or_above(
                                        scope.depth,
                                        start_byte as usize,
                                        &mut facts,
                                    );
                                    scope.open_definition_with_body_docs(
                                        type_name,
                                        start_byte as usize,
                                        true,
                                        false,
                                        &mut facts,
                                    );
                                    scope.on_word(type_name);
                                    let mut j = i + 2;
                                    while j < tokens.len()
                                        && tokens[j].kind != TokenKind::Newline
                                        && tokens[j].kind != TokenKind::Symbol('=')
                                    {
                                        j += 1;
                                    }
                                    i = j;
                                    continue;
                                }
                            }
                        }
                        _ => {
                            let start_byte = tok.start;
                            if i + 1 < tokens.len()
                                && tokens[i + 1].kind == TokenKind::DoubleSymbol("::")
                            {
                                if !facts.defines.contains(&ident.to_string()) {
                                    scope.close_definitions_at_or_above(
                                        scope.depth,
                                        start_byte as usize,
                                        &mut facts,
                                    );
                                    scope.open_definition(
                                        *ident,
                                        start_byte as usize,
                                        true,
                                        &mut facts,
                                    );
                                } else {
                                    scope.enclosing.push((ident.to_string(), scope.depth));
                                }
                                scope.on_word(ident);
                                i += 2;
                                continue;
                            }

                            let mut j = i + 1;
                            while j < tokens.len()
                                && tokens[j].kind != TokenKind::Newline
                                && tokens[j].kind != TokenKind::Symbol('=')
                            {
                                j += 1;
                            }
                            if j < tokens.len()
                                && tokens[j].kind == TokenKind::Symbol('=')
                                && !is_keyword
                            {
                                if !facts.defines.contains(&ident.to_string()) {
                                    scope.close_definitions_at_or_above(
                                        scope.depth,
                                        start_byte as usize,
                                        &mut facts,
                                    );
                                    scope.open_definition(
                                        *ident,
                                        start_byte as usize,
                                        true,
                                        &mut facts,
                                    );
                                } else {
                                    scope.enclosing.push((ident.to_string(), scope.depth));
                                }
                                scope.on_word(ident);
                                in_equation = true;
                                expect_callee = true;
                                i = j + 1;
                                continue;
                            }
                        }
                    }
                } else if !is_keyword {
                    if expect_callee {
                        let j = i + 1;
                        let is_call = if j < tokens.len() {
                            matches!(
                                tokens[j].kind,
                                TokenKind::Ident(_)
                                    | TokenKind::Symbol('(' | '[' | '{' | '$')
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
                } else if matches!(*ident, "then" | "else" | "do" | "in" | "where" | "return") {
                    expect_callee = true;
                    scope.on_word(ident);
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
                scope.had_receiver = false;
            }
        }
        i += 1;
    }

    scope.finish(src.len(), &mut facts);
    facts
}
