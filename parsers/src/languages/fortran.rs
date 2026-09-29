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
    // Fortran closes with `end function charge`, repeating both keyword and
    // name; without this the closing line opened a second definition of the
    // same symbol — measured on the fixture, every one appeared twice.
    let mut after_end = false;

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
            TokenKind::Symbol('%') | TokenKind::Symbol('.') => {
                // Fortran uses % for derived type component selection, e.g. self%method()
                scope.on_receiver();
                i += 1;
                continue;
            }
            TokenKind::Ident(ident) => {
                let after_end_now = std::mem::take(&mut after_end);
                let id_lower = ident.to_ascii_lowercase();
                match id_lower.as_str() {
                    "subroutine" | "function" | "module" | "program" | "type" if after_end_now => {
                        i += 1;
                        if matches!(tokens.get(i).map(|t| &t.kind), Some(TokenKind::Ident(_))) {
                            i += 1;
                        }
                        continue;
                    }
                    "subroutine" => {
                        let start_byte = tok.start;
                        let mut j = i + 1;
                        while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                            j += 1;
                        }
                        if j < tokens.len() {
                            if let TokenKind::Ident(name) = tokens[j].kind {
                                scope.on_open_delimiter();
                                scope.open_definition_with_body_docs(
                                    name,
                                    start_byte as usize,
                                    true,
                                    true,
                                    &mut facts,
                                );
                                // The body was opened one line above, so the
                                // definition belongs to the level *below* the
                                // current one — otherwise an inner `end if`,
                                // which drops back to exactly this depth,
                                // closes the function with it.
                                if let Some(last) = scope.open.last_mut() {
                                    last.depth = scope.depth - 1;
                                }
                                if let Some(last) = scope.enclosing.last_mut() {
                                    last.1 = scope.depth - 1;
                                }
                                scope.on_word(name);
                                i = j + 1;
                                continue;
                            }
                        }
                    }
                    "function" => {
                        let start_byte = tok.start;
                        let mut j = i + 1;
                        while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                            j += 1;
                        }
                        if j < tokens.len() {
                            if let TokenKind::Ident(name) = tokens[j].kind {
                                scope.on_open_delimiter();
                                scope.open_definition_with_body_docs(
                                    name,
                                    start_byte as usize,
                                    true,
                                    true,
                                    &mut facts,
                                );
                                // The body was opened one line above, so the
                                // definition belongs to the level *below* the
                                // current one — otherwise an inner `end if`,
                                // which drops back to exactly this depth,
                                // closes the function with it.
                                if let Some(last) = scope.open.last_mut() {
                                    last.depth = scope.depth - 1;
                                }
                                if let Some(last) = scope.enclosing.last_mut() {
                                    last.1 = scope.depth - 1;
                                }
                                scope.on_word(name);
                                i = j + 1;
                                continue;
                            }
                        }
                    }
                    "program" | "module" => {
                        let start_byte = tok.start;
                        let mut j = i + 1;
                        while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                            j += 1;
                        }
                        if j < tokens.len() {
                            if let TokenKind::Ident(name) = tokens[j].kind {
                                if !name.eq_ignore_ascii_case("procedure") {
                                    scope.on_open_delimiter();
                                    scope.open_definition_with_body_docs(
                                        name,
                                        start_byte as usize,
                                        true,
                                        false,
                                        &mut facts,
                                    );
                                    scope.on_word(name);
                                    i = j + 1;
                                    continue;
                                }
                            }
                        }
                    }
                    "call" => {
                        let mut j = i + 1;
                        while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                            j += 1;
                        }
                        if j < tokens.len() {
                            if let TokenKind::Ident(callee) = tokens[j].kind {
                                scope.record_call(callee, &mut facts);
                                scope.on_word(callee);
                                i = j + 1;
                                continue;
                            }
                        }
                    }
                    // A block `if` opens the level its `end if` closes.
                    // Without this the `end if` closed the enclosing function
                    // instead, and every call after it was attributed to
                    // `<module>` — measured on the fixture, `commit_entry`.
                    "if" if opens_block(&tokens, i + 1) => {
                        scope.on_open_delimiter();
                    }
                    // `do`, `select` and `associate` always have a closing
                    // `end`; `where` may be a one-line assignment, so it is
                    // treated like `if`.
                    "do" | "select" | "associate" => {
                        scope.on_open_delimiter();
                    }
                    "where" if opens_block(&tokens, i + 1) => {
                        scope.on_open_delimiter();
                    }
                    "end" => {
                        scope.on_close_delimiter(tok.end as usize, &mut facts);
                        after_end = true;
                        i += 1;
                        continue;
                    }
                    _ => {
                        if id_lower.starts_with("end") && id_lower.len() > 3 {
                            scope.on_close_delimiter(tok.end as usize, &mut facts);
                        } else {
                            let mut j = i + 1;
                            while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                                j += 1;
                            }
                            if j < tokens.len()
                                && tokens[j].kind == TokenKind::Symbol('(')
                                && calls.allows(&id_lower)
                            {
                                scope.record_call(ident, &mut facts);
                            }
                            scope.on_word(ident);
                        }
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

/// Whether `then` closes this line, which is what separates a Fortran block
/// `if (…) then … end if` from the one-line form `if (…) return`.
///
/// Only the first matters here: the block form has an `end if` that closes a
/// level, so the level has to be opened. The one-line form has none, and
/// opening one there would leave the depth permanently too deep — the mirror
/// image of the bug this fixes.
fn opens_block(tokens: &[crate::lexer::Token<'_>], from: usize) -> bool {
    tokens[from..]
        .iter()
        .take_while(|t| t.kind != TokenKind::Newline)
        .any(|t| matches!(t.kind, TokenKind::Ident(w) if w.eq_ignore_ascii_case("then")))
}
