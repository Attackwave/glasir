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
            TokenKind::Symbol('.') | TokenKind::Symbol(':') => {
                scope.on_receiver();
                i += 1;
                continue;
            }
            TokenKind::Ident(ident) => {
                match *ident {
                    // `local MAX_RETRIES = 3` at file scope. Lua marks a
                    // module's tunables this way; a local inside a function is
                    // not one, hence the depth test.
                    "local" if scope.depth == 0 => {
                        if i + 1 < tokens.len() {
                            if let TokenKind::Ident(name) = tokens[i + 1].kind {
                                if i + 2 < tokens.len() && tokens[i + 2].kind == TokenKind::Symbol('=') {
                                    scope.open_statement_definition(name, tok.start as usize, &mut facts);
                                    scope.on_word(name);
                                    i += 2;
                                    continue;
                                }
                            }
                        }
                    }
                    "function" => {
                        let start_byte = tok.start;
                        let mut j = i + 1;
                        let mut fn_name = String::new();
                        while j < tokens.len() {
                            if let TokenKind::Ident(part) = tokens[j].kind {
                                fn_name.push_str(part);
                                if j + 1 < tokens.len() && (tokens[j + 1].kind == TokenKind::Symbol('.') || tokens[j + 1].kind == TokenKind::Symbol(':')) {
                                    if tokens[j + 1].kind == TokenKind::Symbol('.') {
                                        fn_name.push('.');
                                    } else {
                                        fn_name.push(':');
                                    }
                                    j += 2;
                                    continue;
                                }
                                j += 1;
                                break;
                            } else {
                                break;
                            }
                        }
                        if !fn_name.is_empty() {
                            scope.open_definition(&fn_name, start_byte as usize, true, &mut facts);
                            // The body opens a level of its own, closed by the
                            // matching `end`. `open_definition` records the
                            // depth it was called at and does not raise it, so
                            // without this the *first* `end` in the body — an
                            // `if`'s — closed the function.
                            scope.on_open_delimiter();
                            scope.on_word(&fn_name);
                            i = j;
                            continue;
                        }
                    }
                    // Lua closes `if`, `for`, `while` and `do` with the same
                    // `end` a function uses, so a block that opens nothing
                    // makes its `end` close the enclosing definition — measured
                    // on the fixture, `return commit_entry(...)` after an
                    // `if … end` was attributed to `<module>`.
                    "if" | "for" | "while" | "do" => {
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
                            && (tokens[j].kind == TokenKind::Symbol('(') || matches!(tokens[j].kind, TokenKind::StringLit(_) | TokenKind::Symbol('{')))
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
