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
            TokenKind::DocComment(text) | TokenKind::LineComment(text) | TokenKind::BlockComment(text) => {
                scope.push_comment(text);
                i += 1;
                continue;
            }
            TokenKind::Newline => {
                i += 1;
                continue;
            }
            TokenKind::Symbol('#') => {
                i += 1;
                if i < tokens.len() && matches!(tokens[i].kind, TokenKind::Ident("define")) {
                    let start_byte = tok.start;
                    if i + 1 < tokens.len() {
                        if let TokenKind::Ident(macro_name) = tokens[i + 1].kind {
                            scope.open_definition_with_body_docs(macro_name, start_byte as usize, false, false, &mut facts);
                            scope.on_word(macro_name);
                            i += 2;
                            continue;
                        }
                    }
                }
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
            TokenKind::Symbol('.') | TokenKind::DoubleSymbol("->") | TokenKind::DoubleSymbol("::") => {
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
                        | "inline"
                        | "virtual"
                        | "explicit"
                        | "friend"
                        | "constexpr"
                        | "consteval"
                        | "constinit"
                        | "const"
                        | "volatile"
                        | "mutable"
                        | "noexcept"
                        | "override"
                        | "final"
                        | "extern"
                        | "template"
                        | "typename"
                        | "auto"
                ) {
                    // `const int kMaxRetries = 3;` — the type sits between the
                    // keyword and the name, so the name is the token before `=`.
                    // Only at file scope: a local `const` is not a tunable.
                    if matches!(*ident, "const" | "constexpr") && scope.depth == 0 {
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
                        if ok {
                            if let TokenKind::Ident(name) = tokens[k].kind {
                                scope.open_statement_definition(name, tok.start as usize, &mut facts);
                                scope.on_word(name);
                                i = k + 1;
                                continue;
                            }
                        }
                    }
                    i += 1;
                    continue;
                }

                match *ident {
                    "class" | "struct" | "union" | "enum" | "namespace" | "concept" => {
                        let start_byte = tok.start;
                        let mut j = i + 1;
                        if j < tokens.len() && matches!(tokens[j].kind, TokenKind::Ident("class" | "struct")) {
                            j += 1;
                        }
                        if j < tokens.len() {
                            if let TokenKind::Ident(name) = tokens[j].kind {
                                let opens_body = matches!(*ident, "class" | "struct" | "union" | "enum" | "namespace");
                                scope.open_definition_with_body_docs(name, start_byte as usize, opens_body, false, &mut facts);
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
                        let mut is_fn_def = false;
                        if j < tokens.len() && tokens[j].kind == TokenKind::Symbol('(') {
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
                            while k < tokens.len() && (tokens[k].kind == TokenKind::Newline || matches!(tokens[k].kind, TokenKind::Ident(_) | TokenKind::Symbol(':') | TokenKind::DoubleSymbol("->") | TokenKind::Symbol('&') | TokenKind::Symbol('*'))) {
                                k += 1;
                            }
                            if k < tokens.len() && tokens[k].kind == TokenKind::Symbol('{') {
                                is_fn_def = true;
                            }
                        }

                        if is_fn_def {
                            let start_byte = tok.start;
                            scope.open_definition_with_body_docs(*ident, start_byte as usize, true, true, &mut facts);
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
