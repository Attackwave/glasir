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
            TokenKind::StringLit(s) => {
                if s.starts_with("\"\"\"") {
                    scope.push_comment(s);
                }
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
            TokenKind::Ident("mutable") | TokenKind::Ident("abstract") | TokenKind::Ident("primitive") => {
                i += 1;
                continue;
            }
            TokenKind::Ident(ident) => {
                // `pay(cart) = charge(cart.total)` is Julia's short form, and
                // it is as common as `function … end`: measured on 200 files of
                // the language's own base library, 3,939 against 4,037. Without
                // it half the definitions are missing and their bodies' calls
                // land on `<module>` — that rate read 59% before this.
                //
                // The test is a name, a balanced parenthesis group, then a
                // single `=`. `==` is a comparison and `(a, b) = f()` is
                // destructuring, so both are left alone.
                if scope.depth == 0 && i + 1 < tokens.len() && tokens[i + 1].kind == TokenKind::Symbol('(') {
                    let mut k = i + 1;
                    let mut paren = 0i32;
                    while k < tokens.len() {
                        match tokens[k].kind {
                            TokenKind::Symbol('(') => paren += 1,
                            TokenKind::Symbol(')') => {
                                paren -= 1;
                                if paren == 0 { break; }
                            }
                            TokenKind::Newline => break,
                            _ => {}
                        }
                        k += 1;
                    }
                    if paren == 0
                        && k + 1 < tokens.len()
                        && tokens[k + 1].kind == TokenKind::Symbol('=')
                        && !matches!(tokens.get(k + 2).map(|t| &t.kind), Some(TokenKind::Symbol('=')))
                    {
                        scope.open_statement_definition(*ident, tok.start as usize, &mut facts);
                        scope.on_word(ident);
                        i = k + 2;
                        continue;
                    }
                }

                match *ident {
                    "function" | "macro" => {
                        let start_byte = tok.start;
                        if i + 1 < tokens.len() {
                            if let TokenKind::Ident(fn_name) = tokens[i + 1].kind {
                                scope.open_definition(fn_name, start_byte as usize, true, &mut facts);
                                // The body opens a level its `end` closes.
                                // `open_definition` records the depth it was
                                // called at and does not raise it, so without
                                // this the first `end` *inside* the body — an
                                // `if`'s — closed the function and every later
                                // call fell to `<module>`.
                                scope.on_open_delimiter();
                                if let Some(last) = scope.open.last_mut() {
                                    last.depth = scope.depth - 1;
                                }
                                if let Some(last) = scope.enclosing.last_mut() {
                                    last.1 = scope.depth - 1;
                                }
                                scope.on_word(fn_name);
                                i += 2;
                                continue;
                            }
                        }
                    }
                    "struct" | "module" | "type" | "const" => {
                        let start_byte = tok.start;
                        if i + 1 < tokens.len() {
                            if let TokenKind::Ident(name) = tokens[i + 1].kind {
                                let opens_body = matches!(*ident, "struct" | "module");
                                scope.open_definition(name, start_byte as usize, opens_body, &mut facts);
                                scope.on_word(name);
                                i += 2;
                                continue;
                            }
                        }
                    }
                    // Every one of these is closed by `end` in Julia; there
                    // is no one-line form that omits it.
                    "if" | "for" | "while" | "try" | "begin" | "let" | "quote" | "do" => {
                        scope.on_open_delimiter();
                    }
                    "end" => {
                        scope.on_close_delimiter(tok.end as usize, &mut facts);
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
