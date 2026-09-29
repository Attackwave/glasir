//! Language-specific syntax; lexical and call rules come from the registry.
//!
//! PL/SQL is case-insensitive and written both ways in practice: Oracle's own
//! documentation uses `FUNCTION`, a modern codebase often uses `function`, and
//! a single file mixes them. A `[generic]` table matches a keyword literally,
//! so measured with one the fixture — written the conventional upper-case way
//! — yielded zero definitions and put every call on `<module>`.
//!
//! Nothing else here differs from a keyword table, so only the comparison is
//! folded to lower case.

use crate::facts::FileFacts;
use crate::lexer::{CommentStyle, Lexer, TokenKind};
use crate::scope::ScopeStack;

/// Opens a body closed by `END`.
const DEFINES: [&str; 4] = ["procedure", "function", "package", "trigger"];

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
            TokenKind::Symbol('.') => {
                scope.on_receiver();
                i += 1;
                continue;
            }
            TokenKind::Ident(ident) => {
                let lower = ident.to_ascii_lowercase();

                // `END` closes whatever `BEGIN` or a declaration opened. The
                // optional name after it (`END ledger;`) is skipped by the
                // loop as an ordinary word.
                if lower == "end" {
                    scope.on_close_delimiter(tok.end as usize, &mut facts);
                    i += 1;
                    continue;
                }
                // `IS` / `AS` opens a routine's declaration section and
                // `BEGIN` opens its body, but both are closed by the *same*
                // `END` — so opening a level at each would leave one unclosed
                // per routine. `IS`/`AS` is the one that pairs with `END`,
                // because a package has one and no `BEGIN` at all; `BEGIN` is
                // therefore not a level of its own here.
                if lower == "begin" {
                    i += 1;
                    continue;
                }
                if (lower == "is" || lower == "as") && scope.had_receiver {
                    scope.had_receiver = false;
                }

                if DEFINES.contains(&lower.as_str()) {
                    // `PACKAGE BODY ledger` — the word `BODY` sits between the
                    // keyword and the name.
                    let mut j = i + 1;
                    if let Some(TokenKind::Ident(w)) = tokens.get(j).map(|t| &t.kind) {
                        if w.eq_ignore_ascii_case("body") {
                            j += 1;
                        }
                    }
                    if let Some(TokenKind::Ident(name)) = tokens.get(j).map(|t| &t.kind) {
                        scope.open_definition(*name, tok.start as usize, true, &mut facts);
                        scope.on_word(name);
                        // Skip the parameter list so its closing parenthesis
                        // cannot end the definition, and let the `BEGIN` that
                        // follows open the body. Without this the declaration's
                        // own `)` closed the routine and a quarter of the
                        // references fell to `<module>` — seventh occurrence of
                        // the shape Lua, Fortran, Julia, Scheme, HCL and Tcl
                        // each needed.
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
                        // The routine's own level: opened here rather than at
                        // `IS`, so a `RETURN NUMBER IS` between the parameter
                        // list and the keyword cannot be missed.
                        scope.on_open_delimiter();
                        i = k;
                        continue;
                    }
                }

                let mut j = i + 1;
                while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                    j += 1;
                }
                if tokens.get(j).map(|t| &t.kind) == Some(&TokenKind::Symbol('('))
                    && calls.allows(&lower)
                {
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
