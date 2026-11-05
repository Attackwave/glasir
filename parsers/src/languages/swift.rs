//! Language-specific syntax; lexical and call rules come from the registry.

use crate::facts::FileFacts;
use crate::lexer::{CommentStyle, Lexer, Token, TokenKind};
use crate::scope::ScopeStack;

/// The name of the computed property declared by the `var` at `i`.
fn computed_property<'a>(tokens: &[Token<'a>], i: usize) -> Option<&'a str> {
    let TokenKind::Ident(name) = tokens.get(i + 1)?.kind else {
        return None;
    };
    if tokens.get(i + 2)?.kind != TokenKind::Symbol(':') {
        return None;
    }
    tokens[i + 3..]
        .iter()
        .find(|t| {
            matches!(
                t.kind,
                TokenKind::Newline | TokenKind::Symbol('=' | '{' | ';')
            )
        })
        .filter(|t| t.kind == TokenKind::Symbol('{'))
        .map(|_| name)
}

pub(crate) fn parse(src: &str, style: CommentStyle<'_>, calls: &crate::rules::Calls) -> FileFacts {
    let mut facts = FileFacts::new();
    let mut lexer = Lexer::new(src, style);
    lexer.nested_comments = true;
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
            TokenKind::Symbol('@') => {
                i += 1;
                if i < tokens.len() {
                    if let TokenKind::Ident(_) = tokens[i].kind {
                        i += 1;
                        if i < tokens.len() && tokens[i].kind == TokenKind::Symbol('(') {
                            let mut depth = 1;
                            i += 1;
                            while i < tokens.len() && depth > 0 {
                                if tokens[i].kind == TokenKind::Symbol('(') {
                                    depth += 1;
                                } else if tokens[i].kind == TokenKind::Symbol(')') {
                                    depth -= 1;
                                }
                                i += 1;
                            }
                        }
                    }
                }
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
            TokenKind::Symbol('.') | TokenKind::DoubleSymbol("?.") => {
                scope.on_receiver();
                i += 1;
                continue;
            }
            TokenKind::Ident(ident) => {
                if matches!(
                    *ident,
                    "public"
                        | "private"
                        | "fileprivate"
                        | "internal"
                        | "open"
                        | "static"
                        | "final"
                        | "mutating"
                        | "nonmutating"
                        | "override"
                        | "convenience"
                        | "required"
                        | "lazy"
                        | "weak"
                        | "unowned"
                        | "async"
                        | "throws"
                        | "rethrows"
                ) {
                    i += 1;
                    continue;
                }

                match *ident {
                    // `let maxRetries = 3` at file scope is a constant. Inside
                    // a body it is an ordinary binding, which nobody asks about.
                    "let" if scope.depth == 0 => {
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
                    "class" | "struct" | "enum" | "protocol" | "extension" | "actor" => {
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
                    // `var body: T { … }` computes: a stored one has no brace,
                    // or `=` before it.
                    "var" if computed_property(&tokens, i).is_some() => {
                        if let Some(name) = computed_property(&tokens, i) {
                            scope.open_definition_with_body_docs(
                                name,
                                tok.start as usize,
                                true,
                                true,
                                &mut facts,
                            );
                            scope.on_word(name);
                            i += 2;
                            continue;
                        }
                    }
                    "func" => {
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
                    "init" | "deinit" => {
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
