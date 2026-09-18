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
    // `static final` seen, so the next `<type> <NAME> =` is a constant.
    let mut saw_static = false;
    let mut saw_final = false;

    while i < tokens.len() {
        let tok = &tokens[i];
        match &tok.kind {
            TokenKind::DocComment(text) | TokenKind::LineComment(text) | TokenKind::BlockComment(text) => {
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
                if i < tokens.len() && matches!(tokens[i].kind, TokenKind::Ident("interface")) {
                    let start_byte = tok.start;
                    if i + 1 < tokens.len() {
                        if let TokenKind::Ident(name) = tokens[i + 1].kind {
                            scope.open_definition_with_body_docs(name, start_byte as usize, true, false, &mut facts);
                            scope.on_word(name);
                            i += 2;
                            continue;
                        }
                    }
                }
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
                if matches!(
                    *ident,
                    "public"
                        | "protected"
                        | "private"
                        | "static"
                        | "final"
                        | "abstract"
                        | "synchronized"
                        | "native"
                        | "strictfp"
                        | "transient"
                        | "volatile"
                        | "default"
                        | "sealed"
                        | "non-sealed"
                ) {
                    // Java writes a constant as `static final int MAX = 3;` —
                    // there is no keyword of its own, so the modifier pair is
                    // what marks one.
                    if *ident == "static" {
                        saw_static = true;
                    } else if *ident == "final" && saw_static {
                        saw_final = true;
                    }
                    i += 1;
                    continue;
                }

                if saw_final {
                    // `<type> <NAME> =` — the name is the token before the `=`.
                    let mut k = i;
                    let mut ok = false;
                    while k + 1 < tokens.len() {
                        match tokens[k + 1].kind {
                            TokenKind::Symbol('=') => {
                                ok = true;
                                break;
                            }
                            TokenKind::Symbol(';') | TokenKind::Symbol('(') => break,
                            _ => k += 1,
                        }
                    }
                    saw_static = false;
                    saw_final = false;
                    if ok {
                        if let TokenKind::Ident(name) = tokens[k].kind {
                            scope.open_statement_definition(name, tok.start as usize, &mut facts);
                            scope.on_word(name);
                            i = k + 1;
                            continue;
                        }
                    }
                }

                match *ident {
                    "class" | "interface" | "enum" | "record" => {
                        let start_byte = tok.start;
                        if i + 1 < tokens.len() {
                            if let TokenKind::Ident(name) = tokens[i + 1].kind {
                                scope.open_definition_with_body_docs(name, start_byte as usize, true, false, &mut facts);
                                scope.on_word(name);
                                i += 2;
                                continue;
                            }
                        }
                    }
                    "package" => {
                        let start_byte = tok.start;
                        let mut j = i + 1;
                        let mut pkg_name = String::new();
                        while j < tokens.len() && tokens[j].kind != TokenKind::Symbol(';') && tokens[j].kind != TokenKind::Newline {
                            if let TokenKind::Ident(part) = tokens[j].kind {
                                pkg_name.push_str(part);
                            } else if let TokenKind::Symbol('.') = tokens[j].kind {
                                pkg_name.push('.');
                            }
                            j += 1;
                        }
                        if !pkg_name.is_empty() {
                            scope.open_definition_with_body_docs(&pkg_name, start_byte as usize, false, false, &mut facts);
                            scope.on_word(&pkg_name);
                            i = j;
                            continue;
                        }
                    }
                    _ => {
                        let mut j = i + 1;
                        while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                            j += 1;
                        }
                        let mut is_method_def = false;
                        if !crate::scope::is_control_keyword(ident)
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
                            while k < tokens.len() && (tokens[k].kind == TokenKind::Newline || matches!(tokens[k].kind, TokenKind::Ident(_) | TokenKind::Symbol(','))) {
                                k += 1;
                            }
                            if k < tokens.len() && tokens[k].kind == TokenKind::Symbol('{') && scope.depth > 0 {
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
