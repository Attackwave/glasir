//! Language-specific syntax; lexical and call rules come from the registry.
//!
//! CFML declares a function as an XML tag whose *attribute* carries the name:
//! `<cffunction name="charge">`. The name is neither adjacent to a keyword nor
//! at a fixed offset — it follows whichever attribute happens to be called
//! `name` — so a `[generic]` table cannot reach it.
//!
//! TLA+ shares this module for a different reason with the same consequence:
//! `Charge(owner, amount) == body` puts the name *before* its parameter list
//! and marks the definition with `==` after it, so what identifies a
//! definition sits on the far side of a delimiter.

use crate::facts::FileFacts;
use crate::lexer::{CommentStyle, Lexer, TokenKind};
use crate::scope::ScopeStack;

/// CFML tags that open a definition.
const TAGS: [&str; 2] = ["cffunction", "cfcomponent"];

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
                // `</cffunction>` closes one. The `/` arrives as its own
                // symbol, so the test is the previous token.
                let closing = i > 0 && tokens[i - 1].kind == TokenKind::Symbol('/');
                if TAGS.contains(ident) {
                    if closing {
                        scope.on_close_delimiter(tok.end as usize, &mut facts);
                        i += 1;
                        continue;
                    }
                    // Scan the tag's attributes for `name="…"`.
                    let mut j = i + 1;
                    while j + 2 < tokens.len() {
                        if tokens[j].kind == TokenKind::Symbol('>') {
                            break;
                        }
                        if let TokenKind::Ident("name") = tokens[j].kind {
                            if tokens[j + 1].kind == TokenKind::Symbol('=') {
                                if let Some(TokenKind::StringLit(raw)) =
                                    tokens.get(j + 2).map(|t| &t.kind)
                                {
                                    let clean = raw.trim_matches('"').trim_matches('\'');
                                    if !clean.is_empty() {
                                        scope.open_definition(
                                            clean,
                                            tok.start as usize,
                                            true,
                                            &mut facts,
                                        );
                                        scope.on_word(clean);
                                        scope.on_open_delimiter();
                                    }
                                    break;
                                }
                            }
                        }
                        j += 1;
                    }
                    i = j + 1;
                    continue;
                }

                // TLA+: `Name(args) == body`, or `Name == body` for a constant.
                let mut j = i + 1;
                if tokens.get(j).map(|t| &t.kind) == Some(&TokenKind::Symbol('(')) {
                    let mut depth = 0usize;
                    while j < tokens.len() {
                        match tokens[j].kind {
                            TokenKind::Symbol('(') => depth += 1,
                            TokenKind::Symbol(')') => {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            _ => {}
                        }
                        j += 1;
                    }
                    j += 1;
                }
                if tokens.get(j).map(|t| &t.kind) == Some(&TokenKind::DoubleSymbol("==")) {
                    // A definition ends where the next one begins, which is the
                    // only boundary this language offers.
                    scope.on_close_delimiter(tok.start as usize, &mut facts);
                    scope.open_definition(*ident, tok.start as usize, true, &mut facts);
                    scope.on_word(ident);
                    scope.on_open_delimiter();
                    i = j + 1;
                    continue;
                }

                let mut k = i + 1;
                while k < tokens.len() && tokens[k].kind == TokenKind::Newline {
                    k += 1;
                }
                if tokens.get(k).map(|t| &t.kind) == Some(&TokenKind::Symbol('('))
                    && calls.allows(ident)
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
