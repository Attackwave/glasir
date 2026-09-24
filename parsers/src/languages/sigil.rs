//! Language-specific syntax; lexical and call rules come from the registry.
//!
//! LLVM IR names every global with a sigil — `define i32 @charge(...)` and
//! `call i32 @refuse(...)` — and the lexer emits `@` as a symbol of its own,
//! so the name is never adjacent to the keyword. A `[generic]` table matches
//! `keyword name`, which is why this needs a module.
//!
//! The same shape covers Wolfram, where a definition is `name[x_] := body`
//! and the delimiter is a bracket rather than a sigil: both put something
//! between the name and what identifies it as a definition.

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
            TokenKind::DocComment(text) | TokenKind::LineComment(text) => {
                scope.push_comment(text);
                i += 1;
                continue;
            }
            TokenKind::Newline => {
                i += 1;
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
            TokenKind::Ident(ident) => {
                // `define … @name(` opens a function; `declare` is a prototype
                // with no body and is deliberately not a definition.
                if *ident == "define" {
                    let mut j = i + 1;
                    while j < tokens.len() {
                        if tokens[j].kind == TokenKind::Symbol('@') {
                            if let Some(TokenKind::Ident(name)) = tokens.get(j + 1).map(|t| &t.kind)
                            {
                                scope.open_definition(*name, tok.start as usize, true, &mut facts);
                                scope.on_word(name);
                                i = j + 2;
                                break;
                            }
                        }
                        if tokens[j].kind == TokenKind::Newline {
                            break;
                        }
                        j += 1;
                    }
                    if i == j + 2 {
                        continue;
                    }
                }

                // Wolfram: `name[x_] := body`. The bracket holds the pattern,
                // and `:=` is what makes it a definition rather than a call.
                if tokens.get(i + 1).map(|t| &t.kind) == Some(&TokenKind::Symbol('[')) {
                    let mut depth = 0usize;
                    let mut j = i + 1;
                    while j < tokens.len() {
                        match tokens[j].kind {
                            TokenKind::Symbol('[') => depth += 1,
                            TokenKind::Symbol(']') => {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            _ => {}
                        }
                        j += 1;
                    }
                    if tokens.get(j + 1).map(|t| &t.kind) == Some(&TokenKind::DoubleSymbol(":=")) {
                        scope.open_definition(*ident, tok.start as usize, true, &mut facts);
                        scope.on_word(ident);
                        scope.on_open_delimiter();
                        i = j + 2;
                        continue;
                    }
                    // No `:=` after the bracket: this is a call, `notify[x]`.
                    if calls.allows(ident) {
                        scope.record_call(ident, &mut facts);
                    }
                    scope.on_word(ident);
                    i += 1;
                    continue;
                }

                let mut j = i + 1;
                while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                    j += 1;
                }
                if tokens.get(j).map(|t| &t.kind) == Some(&TokenKind::Symbol('('))
                    && calls.allows(ident)
                {
                    scope.record_call(ident, &mut facts);
                }
                scope.on_word(ident);
            }
            // `@name(` in a call position: the callee is the word after it.
            TokenKind::Symbol('@') => {
                if let Some(TokenKind::Ident(name)) = tokens.get(i + 1).map(|t| &t.kind) {
                    if tokens.get(i + 2).map(|t| &t.kind) == Some(&TokenKind::Symbol('('))
                        && calls.allows(name)
                    {
                        scope.record_call(name, &mut facts);
                    }
                    scope.on_word(name);
                    i += 2;
                    continue;
                }
                i += 1;
                continue;
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
