//! Language-specific syntax; lexical and call rules come from the registry.
//!
//! Template languages name a block inside a delimiter rather than with a
//! keyword at statement level: Go writes `{{define "charge"}}`, Liquid writes
//! `{% assign charge = %}` and `{% capture charge %}`. The name is a *string*
//! in the first case and a bare word in the second, and both sit behind a
//! two-character opener — nothing a keyword table can match, measured as zero
//! definitions.
//!
//! One module for both: the openers differ, the shape does not.

use crate::facts::FileFacts;
use crate::lexer::{CommentStyle, Lexer, TokenKind};
use crate::scope::ScopeStack;

/// Words that open a named block, in either dialect.
const DEFINES: [&str; 6] = ["define", "block", "assign", "capture", "macro", "section"];
/// Words that close one.
const CLOSES: [&str; 5] = ["end", "endblock", "endcapture", "endmacro", "endsection"];

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
                if CLOSES.contains(ident) {
                    scope.on_close_delimiter(tok.end as usize, &mut facts);
                    i += 1;
                    continue;
                }
                if DEFINES.contains(ident) {
                    // The name is either a quoted string (Go) or the next bare
                    // word (Liquid, Jinja).
                    // Blade writes `@section('name')`, parenthesised, where Go
                    // and Liquid put the name directly after the word. Step
                    // over the opening paren so one lookup serves all three.
                    let mut at = i + 1;
                    if tokens.get(at).map(|t| &t.kind) == Some(&TokenKind::Symbol('(')) {
                        at += 1;
                    }
                    match tokens.get(at).map(|t| &t.kind) {
                        Some(TokenKind::StringLit(name)) => {
                            let clean = name.trim_matches('"').trim_matches('\'');
                            if !clean.is_empty() {
                                scope.open_definition(
                                    clean,
                                    tok.start as usize,
                                    true,
                                    &mut facts,
                                );
                                scope.on_word(clean);
                                scope.on_open_delimiter();
                                i = at + 1;
                                continue;
                            }
                        }
                        Some(TokenKind::Ident(name)) => {
                            scope.open_definition(*name, tok.start as usize, true, &mut facts);
                            scope.on_word(name);
                            scope.on_open_delimiter();
                            i = at + 1;
                            continue;
                        }
                        _ => {}
                    }
                }

                // `{{ template "other" }}` and `{% include 'other' %}` reach
                // another block by name, which is the edge worth having.
                if matches!(*ident, "template" | "include" | "render" | "partial") {
                    let mut at = i + 1;
                    if tokens.get(at).map(|t| &t.kind) == Some(&TokenKind::Symbol('(')) {
                        at += 1;
                    }
                    if let Some(TokenKind::StringLit(name)) = tokens.get(at).map(|t| &t.kind) {
                        let clean = name.trim_matches('"').trim_matches('\'');
                        if !clean.is_empty() && calls.allows(clean) {
                            scope.record_call(clean, &mut facts);
                        }
                        i = at + 1;
                        continue;
                    }
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
