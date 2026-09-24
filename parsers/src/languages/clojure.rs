//! Language-specific syntax; lexical and call rules come from the registry.

use crate::facts::FileFacts;
use crate::lexer::{CommentStyle, Lexer, Token, TokenKind};
use crate::scope::ScopeStack;

fn get_lisp_ident<'a>(tokens: &[Token<'a>], idx: &mut usize, src: &'a str) -> Option<&'a str> {
    while *idx < tokens.len() && tokens[*idx].kind == TokenKind::Newline {
        *idx += 1;
    }
    if *idx >= tokens.len() {
        return None;
    }
    if matches!(tokens[*idx].kind, TokenKind::Ident(_)) {
        let start = tokens[*idx].start as usize;
        let mut end = tokens[*idx].end as usize;
        *idx += 1;
        while *idx < tokens.len() {
            match tokens[*idx].kind {
                TokenKind::Symbol('-')
                | TokenKind::Symbol('.')
                | TokenKind::Symbol('/')
                | TokenKind::Symbol('_') => {
                    if *idx + 1 < tokens.len()
                        && matches!(tokens[*idx + 1].kind, TokenKind::Ident(_))
                    {
                        end = tokens[*idx + 1].end as usize;
                        *idx += 2;
                    } else {
                        break;
                    }
                }
                _ => break,
            }
        }
        Some(&src[start..end])
    } else {
        None
    }
}

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
            TokenKind::Symbol('(') => {
                scope.on_open_delimiter();
                let mut next_idx = i + 1;
                if let Some(form) = get_lisp_ident(&tokens, &mut next_idx, src) {
                    match form {
                        // Counted in Clojure's own library rather than
                        // guessed: `defmethod` 142, `defonce` 12, `definline`
                        // 8, `defstruct` 5 in `src/`, and `deftest` 672 with
                        // `defspec` 36 in `test/`. Missing `defmethod` put 85
                        // calls on `<module>` and `deftest` 382 — every
                        // multimethod implementation's and every test's body.
                        // `defcurried` and `def-aset` appear eight times each
                        // and are this project's own macros, not the language.
                        "defn" | "defn-" | "defmacro" | "defmethod" | "defmulti"
                        | "defprotocol" | "defrecord" | "deftype" | "defonce" | "definline"
                        | "defstruct" | "deftest" | "defspec" | "def" => {
                            let start_byte = tok.start;
                            if let Some(name) = get_lisp_ident(&tokens, &mut next_idx, src) {
                                let is_fn = matches!(
                                    form,
                                    "defn"
                                        | "defn-"
                                        | "defmacro"
                                        | "defmethod"
                                        | "definline"
                                        | "deftest"
                                );
                                scope.open_definition_with_body_docs(
                                    name,
                                    start_byte as usize,
                                    true,
                                    is_fn,
                                    &mut facts,
                                );
                                if let Some(last) = scope.open.last_mut() {
                                    last.depth = scope.depth - 1;
                                }
                                if let Some(last) = scope.enclosing.last_mut() {
                                    last.1 = scope.depth - 1;
                                }
                                scope.on_word(name);
                                i = next_idx;
                                continue;
                            }
                        }
                        "ns" => {
                            let start_byte = tok.start;
                            if let Some(name) = get_lisp_ident(&tokens, &mut next_idx, src) {
                                scope.open_definition_with_body_docs(
                                    name,
                                    start_byte as usize,
                                    true,
                                    false,
                                    &mut facts,
                                );
                                if let Some(last) = scope.open.last_mut() {
                                    last.depth = scope.depth - 1;
                                }
                                if let Some(last) = scope.enclosing.last_mut() {
                                    last.1 = scope.depth - 1;
                                }
                                scope.on_word(name);
                                i = next_idx;
                                continue;
                            }
                        }
                        _ => {
                            if calls.allows(form) {
                                scope.record_call(form, &mut facts);
                            }
                        }
                    }
                }
                i += 1;
                continue;
            }
            TokenKind::Symbol(')') => {
                scope.on_close_delimiter(tok.end as usize, &mut facts);
                i += 1;
                continue;
            }
            TokenKind::Ident(ident) => {
                scope.on_word(ident);
            }
            _ => {}
        }
        i += 1;
    }

    scope.finish(src.len(), &mut facts);
    facts
}
