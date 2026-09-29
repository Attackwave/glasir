//! Language-specific syntax; lexical and call rules come from the registry.
//!
//! Meson and Jsonnet both define by assignment — `name = function(...)` and
//! `name:: function(...)` — so the name precedes the shape that makes it a
//! definition, exactly as R writes `name <- function(`. A `[generic]` table
//! matches `keyword name`, which is the opposite order.
//!
//! One module for both: the only difference is the assignment token, and a
//! second copy of this loop would drift from this one.

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
            TokenKind::Symbol('{') | TokenKind::Symbol('(') => {
                scope.on_open_delimiter();
                i += 1;
                continue;
            }
            TokenKind::Symbol('}') | TokenKind::Symbol(')') => {
                scope.on_close_delimiter(tok.end as usize, &mut facts);
                i += 1;
                continue;
            }
            // Magma closes a body with `end function;` rather than a brace,
            // and Pine closes one by dedenting — neither is a delimiter this
            // module counts. Measured, Magma read 100% on `<module>` with its
            // definitions found: the bodies never closed, so every later call
            // belonged to the first function.
            TokenKind::Ident("end") => {
                scope.on_close_delimiter(tok.end as usize, &mut facts);
                i += 1;
                continue;
            }
            TokenKind::Symbol('.') => {
                scope.on_receiver();
                i += 1;
                continue;
            }
            // Typst prefixes a binding with `#`: `#let charge(…) = …`. The
            // sigil is its own token, so the `let` behind it is what the
            // assignment test must see — skipping it here costs Meson and
            // Jsonnet nothing, since neither writes one.
            TokenKind::Symbol('#') => {
                i += 1;
                continue;
            }
            TokenKind::Ident(ident) => {
                // `let name(args) = …` / `let name = …`: Typst and ReScript
                // both put a keyword before the name that Meson does not.
                if matches!(*ident, "let" | "const") {
                    i += 1;
                    continue;
                }
                let mut j = i + 1;
                while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                    j += 1;
                }
                // Typst and ReScript write `name(args) = body`, so the token
                // that marks the assignment sits *after* the parameter list.
                // Looking only at the next token measured Typst at 45
                // definitions per 1,000 lines and 86% on `<module>`.
                if tokens.get(j).map(|t| &t.kind) == Some(&TokenKind::Symbol('(')) {
                    let mut depth = 0usize;
                    let mut k = j;
                    while k < tokens.len() {
                        match tokens[k].kind {
                            TokenKind::Symbol('(') => depth += 1,
                            TokenKind::Symbol(')') => {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            _ => {}
                        }
                        k += 1;
                    }
                    // Pine writes `charge(owner, amount) =>`, Magma
                    // `charge := function(…)`; the token after the parameter
                    // list is `=`, `=>` or `:=` depending on the language.
                    if matches!(
                        tokens.get(k + 1).map(|t| &t.kind),
                        Some(TokenKind::Symbol('='))
                            | Some(TokenKind::DoubleSymbol("=>"))
                            | Some(TokenKind::DoubleSymbol(":="))
                    ) {
                        j = k + 1;
                    }
                }
                // `name = …` (Meson) or `name:: …` / `name: …` (Jsonnet).
                let assigns = matches!(
                    tokens.get(j).map(|t| &t.kind),
                    Some(TokenKind::Symbol('='))
                        | Some(TokenKind::DoubleSymbol("::"))
                        | Some(TokenKind::DoubleSymbol(":="))
                        | Some(TokenKind::DoubleSymbol("=>"))
                );
                if assigns {
                    scope.open_definition(*ident, tok.start as usize, true, &mut facts);
                    scope.on_word(ident);
                    // ReScript writes `let charge = (owner, amount) => {…}`:
                    // the parameter list sits *after* the assignment and its
                    // closing paren would end the definition before the body
                    // is reached — measured, 75% of references on `<module>`.
                    // Meson and Jsonnet have no such list, so stepping over
                    // one when it is there serves all three. Ninth occurrence
                    // of the two-halves scope shape in this project.
                    let mut k = j + 1;
                    if tokens.get(k).map(|t| &t.kind) == Some(&TokenKind::Symbol('(')) {
                        let mut depth = 0usize;
                        while k < tokens.len() {
                            match tokens[k].kind {
                                TokenKind::Symbol('(') => depth += 1,
                                TokenKind::Symbol(')') => {
                                    depth -= 1;
                                    if depth == 0 {
                                        break;
                                    }
                                }
                                _ => {}
                            }
                            k += 1;
                        }
                        k += 1;
                    }
                    // The body's own level, opened whether or not a parameter
                    // list was there to skip. Without it a function whose body
                    // is a bare `if … {…} else {…}` — legal ReScript — ends at
                    // the `if`'s first closing brace; and Magma, which writes
                    // `charge := function(owner)` with the list *after* the
                    // keyword rather than after the name, never opened one at
                    // all: 100% on `<module>` with every definition found.
                    // Both halves are needed and neither works by itself,
                    // exactly as Lua, Fortran, Julia and Tcl each measured.
                    scope.on_open_delimiter();
                    i = k;
                    continue;
                }

                // Nickel applies a function by juxtaposition — `warn_owner
                // owner` — like every ML. Testing only for `(` measured 538
                // definitions per 1,000 lines and **zero** edges there, while
                // Meson, Jsonnet, Typst, ReScript, Magma and Pine all write
                // the parenthesis and are unaffected either way.
                let applied = tokens.get(j).is_some_and(|t| {
                    matches!(
                        &t.kind,
                        TokenKind::Symbol('(') | TokenKind::Ident(_) | TokenKind::StringLit(_)
                    )
                });
                if applied && calls.allows(ident) {
                    scope.record_call(ident, &mut facts);
                }
                scope.on_word(ident);
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
