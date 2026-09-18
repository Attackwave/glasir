//! Language-specific syntax; lexical and call rules come from the registry.
//!
//! Objective-C writes a method as `- (NSInteger)charge:(NSInteger)amount`, so
//! the name follows a sign and a parenthesised return type rather than a
//! keyword — and a call is `[receiver name:arg]` rather than `name(`. Neither
//! shape is a keyword list, which is why this language needs a module:
//! measured with a `[generic]` table, the fixture yielded zero definitions and
//! put every reference on `<module>`.

use crate::facts::FileFacts;
use crate::lexer::{CommentStyle, Lexer, TokenKind};
use crate::scope::ScopeStack;

pub(crate) fn parse(src: &str, style: CommentStyle<'_>, calls: &crate::rules::Calls) -> FileFacts {
    let mut facts = FileFacts::new();
    let mut lexer = Lexer::new(src, style);
    let tokens = lexer.collect_all_tokens();

    let mut i = 0;
    let mut scope = ScopeStack::new();
    // Depth of `[` nesting: inside one, an identifier followed by `:` is a
    // message send rather than a method signature.
    let mut bracket = 0usize;

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
            TokenKind::Symbol('[') => {
                bracket += 1;
                i += 1;
                continue;
            }
            TokenKind::Symbol(']') => {
                bracket = bracket.saturating_sub(1);
                i += 1;
                continue;
            }
            TokenKind::Symbol('.') | TokenKind::DoubleSymbol("->") => {
                scope.on_receiver();
                i += 1;
                continue;
            }
            // `- (type)name` or `+ (type)name`: a method signature at file
            // level. The sign alone is not enough — it is also subtraction —
            // so the parenthesised type between sign and name is what decides.
            TokenKind::Symbol('-') | TokenKind::Symbol('+') if bracket == 0 => {
                let mut j = i + 1;
                while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                    j += 1;
                }
                if tokens.get(j).map(|t| &t.kind) == Some(&TokenKind::Symbol('(')) {
                    // Skip the return type to its closing parenthesis.
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
                    if let Some(TokenKind::Ident(name)) = tokens.get(k + 1).map(|t| &t.kind) {
                        scope.open_definition(*name, tok.start as usize, true, &mut facts);
                        scope.on_word(name);
                        i = k + 2;
                        continue;
                    }
                }
                i += 1;
                continue;
            }
            TokenKind::Ident(ident) => {
                // `@implementation Ledger` / `@interface Ledger` — the marker
                // is lexed as `@` plus the word, so the keyword is matched on
                // the word and the preceding `@` is what distinguishes it from
                // an ordinary identifier.
                let at_marked = i > 0 && tokens[i - 1].kind == TokenKind::Symbol('@');
                if at_marked && matches!(*ident, "implementation" | "interface") {
                    if let Some(TokenKind::Ident(name)) = tokens.get(i + 1).map(|t| &t.kind) {
                        scope.open_definition(*name, tok.start as usize, false, &mut facts);
                        scope.on_word(name);
                        i += 2;
                        continue;
                    }
                }
                if at_marked && *ident == "end" {
                    scope.on_close_delimiter(tok.end as usize, &mut facts);
                    i += 1;
                    continue;
                }

                // A message send `[self refuse:amount]`, or an ordinary C call
                // `warnOwner(x)`.
                let mut j = i + 1;
                while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                    j += 1;
                }
                let is_message =
                    bracket > 0 && tokens.get(j).map(|t| &t.kind) == Some(&TokenKind::Symbol(':'));
                let is_c_call =
                    tokens.get(j).map(|t| &t.kind) == Some(&TokenKind::Symbol('('));
                if (is_message || is_c_call) && calls.allows(ident) {
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
