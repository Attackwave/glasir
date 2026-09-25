//! Language-specific syntax; lexical and call rules come from the registry.
//!
//! Shared by TypeScript and JavaScript, which differ only in their rule file.

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
            TokenKind::Symbol('@') => {
                while i < tokens.len() && tokens[i].kind != TokenKind::Newline {
                    i += 1;
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
                    "export"
                        | "default"
                        | "async"
                        | "declare"
                        | "public"
                        | "private"
                        | "protected"
                        | "static"
                        | "readonly"
                ) {
                    i += 1;
                    continue;
                }

                match *ident {
                    "function" => {
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
                    "class" | "interface" | "type" | "enum" => {
                        let start_byte = tok.start;
                        if i + 1 < tokens.len() {
                            if let TokenKind::Ident(name) = tokens[i + 1].kind {
                                let opens_body = matches!(*ident, "class" | "interface" | "enum");
                                scope.open_definition_with_body_docs(
                                    name,
                                    start_byte as usize,
                                    opens_body,
                                    false,
                                    &mut facts,
                                );
                                scope.on_word(name);
                                i += 2;
                                continue;
                            }
                        }
                    }
                    "const" | "let" | "var" if scope.depth == 0 => {
                        let start_byte = tok.start;
                        if i + 1 < tokens.len() {
                            if let TokenKind::Ident(name) = tokens[i + 1].kind {
                                scope.open_statement_definition(
                                    name,
                                    start_byte as usize,
                                    &mut facts,
                                );
                                scope.on_word(name);
                                i += 2;
                                continue;
                            }
                        }
                    }
                    _ => {
                        let mut j = skip_newlines(&tokens, i + 1);
                        // `find<T>(` is a method as much as `find(`.
                        if let Some(after) = skip_type_arguments(&tokens, j) {
                            j = skip_newlines(&tokens, after);
                        }
                        let call_paren = j;
                        let mut is_method_def = false;
                        // `delete(id) {` in a class body is a method: the
                        // operator never takes a parameter list and a body.
                        if (!crate::scope::is_control_keyword(ident) || *ident == "delete")
                            && j < tokens.len()
                            && tokens[j].kind == TokenKind::Symbol('(')
                        {
                            let mut k = j + 1;
                            let mut paren_depth = 1;
                            while k < tokens.len() && paren_depth > 0 {
                                if tokens[k].kind == TokenKind::Symbol('(') {
                                    paren_depth += 1;
                                } else if tokens[k].kind == TokenKind::Symbol(')') {
                                    paren_depth -= 1;
                                }
                                k += 1;
                            }
                            while k < tokens.len()
                                && (tokens[k].kind == TokenKind::Newline
                                    || matches!(
                                        tokens[k].kind,
                                        TokenKind::Ident(_)
                                            | TokenKind::Symbol(':')
                                            | TokenKind::DoubleSymbol("=>")
                                    ))
                            {
                                // A return type is a type expression, not a
                                // run of names: `Observable<IBankAccount>`,
                                // `Promise<A | B>`, `string[]`. Ending the scan
                                // at its `<` hid every annotated method.
                                if tokens[k].kind == TokenKind::Symbol(':') {
                                    k = skip_return_type(&tokens, k + 1);
                                    break;
                                }
                                k += 1;
                            }
                            if k < tokens.len()
                                && tokens[k].kind == TokenKind::Symbol('{')
                                && scope.depth > 0
                            {
                                is_method_def = true;
                            }
                        }

                        if is_method_def {
                            let start_byte = tok.start;
                            scope.open_definition(*ident, start_byte as usize, true, &mut facts);
                            scope.on_word(ident);
                            i += 1;
                            continue;
                        }

                        let j = call_paren;
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

fn skip_newlines(tokens: &[crate::lexer::Token<'_>], mut j: usize) -> usize {
    while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
        j += 1;
    }
    j
}

/// The index after `<...>` when `j` opens a type argument list, `None` when
/// the `<` is a comparison.
fn skip_type_arguments(tokens: &[crate::lexer::Token<'_>], j: usize) -> Option<usize> {
    if tokens.get(j)?.kind != TokenKind::Symbol('<') {
        return None;
    }
    let mut depth = 0i32;
    let mut k = j;
    while let Some(t) = tokens.get(k) {
        match t.kind {
            TokenKind::Symbol('<') => depth += 1,
            TokenKind::Symbol('>') => depth -= 1,
            TokenKind::DoubleSymbol(">>") => depth -= 2,
            TokenKind::Ident(_)
            | TokenKind::Symbol('.' | ',' | '?' | '[' | ']' | '|' | '&')
            | TokenKind::Newline => {}
            _ => return None,
        }
        k += 1;
        if depth <= 0 {
            return (depth == 0).then_some(k);
        }
    }
    None
}

/// The index of what follows a return type starting at `k`: the body's `{`
/// when there is one. Stops at anything a type cannot hold on its line.
fn skip_return_type(tokens: &[crate::lexer::Token<'_>], mut k: usize) -> usize {
    let mut angle = 0i32;
    let mut paren = 0i32;
    while let Some(t) = tokens.get(k) {
        match t.kind {
            TokenKind::Symbol('{') if angle == 0 && paren == 0 => return k,
            TokenKind::Symbol('<') => angle += 1,
            TokenKind::Symbol('>') => angle -= 1,
            TokenKind::DoubleSymbol(">>") => angle -= 2,
            TokenKind::Symbol('(') => paren += 1,
            TokenKind::Symbol(')') if paren > 0 => paren -= 1,
            TokenKind::Ident(_)
            | TokenKind::StringLit(_)
            | TokenKind::Number(_)
            | TokenKind::Symbol('.' | ',' | '?' | '[' | ']' | '|' | '&' | ':')
            | TokenKind::DoubleSymbol("=>") => {}
            _ => return k,
        }
        if angle < 0 {
            return k;
        }
        k += 1;
    }
    k
}
