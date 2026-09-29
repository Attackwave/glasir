//! Language-specific syntax; configurable lexical and call rules are supplied by the registry.

use crate::facts::FileFacts;
use crate::lexer::{CommentStyle, Lexer, TokenKind};
use crate::scope::ScopeStack;

pub(crate) fn parse(src: &str, style: CommentStyle<'_>, calls: &crate::rules::Calls) -> FileFacts {
    let mut facts = FileFacts::new();
    let mut lexer = Lexer::new(src, style);
    let tokens = lexer.collect_all_tokens();

    let mut i = 0;
    let mut scope = ScopeStack::new();
    // Brace depth at which an `extern` block opened, if one is open.
    let mut extern_depth: Option<i32> = None;

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
            // Skip Rust attributes: `#[derive(...)]` or `#![...]`
            TokenKind::Symbol('#') => {
                i += 1;
                if i < tokens.len() && tokens[i].kind == TokenKind::Symbol('!') {
                    i += 1;
                }
                if i < tokens.len() && tokens[i].kind == TokenKind::Symbol('[') {
                    let mut depth = 1;
                    i += 1;
                    while i < tokens.len() && depth > 0 {
                        if tokens[i].kind == TokenKind::Symbol('[') {
                            depth += 1;
                        } else if tokens[i].kind == TokenKind::Symbol(']') {
                            depth -= 1;
                        }
                        i += 1;
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
                if extern_depth.is_some_and(|d| scope.depth - 1 <= d) {
                    extern_depth = None;
                }
                scope.on_close_delimiter(tok.end as usize, &mut facts);
                i += 1;
                continue;
            }
            TokenKind::Symbol(';') => {
                scope.on_statement_end(tok.end as usize, &mut facts);
                i += 1;
                continue;
            }
            TokenKind::Symbol('.') => {
                scope.on_receiver();
                i += 1;
                continue;
            }
            TokenKind::DoubleSymbol("::") => {
                scope.on_receiver();
                i += 1;
                continue;
            }
            TokenKind::Ident(ident) => {
                // `extern "C" { fn f(); }` declares what another object file
                // defines. Counting those as definitions gives tier 3 a local
                // target for a name this tree does not implement.
                if *ident == "extern" {
                    extern_depth = Some(scope.depth);
                    i += 1;
                    continue;
                }
                if matches!(
                    *ident,
                    "pub" | "async" | "unsafe" | "default" | "mut" | "ref" | "move"
                ) {
                    if *ident == "pub"
                        && i + 1 < tokens.len()
                        && tokens[i + 1].kind == TokenKind::Symbol('(')
                    {
                        i += 2;
                        while i < tokens.len() && tokens[i].kind != TokenKind::Symbol(')') {
                            i += 1;
                        }
                    }
                    i += 1;
                    continue;
                }

                match *ident {
                    "fn" => {
                        let start_byte = tok.start;
                        if i + 1 < tokens.len() {
                            if let TokenKind::Ident(fn_name) = tokens[i + 1].kind {
                                if extern_depth.is_some_and(|d| scope.depth > d) {
                                    i += 2;
                                    continue;
                                }
                                scope.open_definition(
                                    fn_name,
                                    start_byte as usize,
                                    true,
                                    &mut facts,
                                );
                                scope.on_word(fn_name);
                                i += 2;
                                continue;
                            }
                        }
                    }
                    "macro_rules" => {
                        let start_byte = tok.start;
                        if i + 2 < tokens.len() && tokens[i + 1].kind == TokenKind::Symbol('!') {
                            if let TokenKind::Ident(macro_name) = tokens[i + 2].kind {
                                scope.open_definition(
                                    macro_name,
                                    start_byte as usize,
                                    false,
                                    &mut facts,
                                );
                                scope.on_word(macro_name);
                                i += 3;
                                continue;
                            }
                        }
                    }
                    "struct" | "enum" | "trait" | "union" | "mod" | "type" | "const" | "static" => {
                        let start_byte = tok.start;
                        if i + 1 < tokens.len() {
                            if let TokenKind::Ident(type_name) = tokens[i + 1].kind {
                                // A constant owns the calls in its initialiser:
                                // `const LIMIT: usize = compute_limit();` is not
                                // a module-level call. Its scope ends at the `;`,
                                // since it opens no brace of its own.
                                if matches!(*ident, "const" | "static") {
                                    scope.open_statement_definition(
                                        type_name,
                                        start_byte as usize,
                                        &mut facts,
                                    );
                                    scope.on_word(type_name);
                                    i += 2;
                                    continue;
                                }
                                let opens_body =
                                    matches!(*ident, "struct" | "enum" | "trait" | "union" | "mod");
                                scope.open_definition_with_body_docs(
                                    type_name,
                                    start_byte as usize,
                                    opens_body,
                                    false,
                                    &mut facts,
                                );
                                scope.on_word(type_name);
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
