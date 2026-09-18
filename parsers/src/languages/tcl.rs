//! Language-specific syntax; lexical and call rules come from the registry.
//!
//! Tcl calls a command as `[refuse $owner]` — the name is the *first word*
//! inside a bracket, with no parenthesis anywhere. The generic scanner tests
//! for `name (`, which is why this language needs a module: measured with a
//! `[generic]` table the fixture found its definitions and recorded **zero**
//! edges, so every call in it was lost silently.

use crate::facts::FileFacts;
use crate::lexer::{CommentStyle, Lexer, TokenKind};
use crate::scope::ScopeStack;

pub(crate) fn parse(src: &str, style: CommentStyle<'_>, calls: &crate::rules::Calls) -> FileFacts {
    let mut facts = FileFacts::new();
    let mut lexer = Lexer::new(src, style);
    let tokens = lexer.collect_all_tokens();

    let mut i = 0;
    let mut scope = ScopeStack::new();
    // True when the previous significant token opened a command position: the
    // start of a bracket, or a statement boundary. Only there is an identifier
    // the name of a command rather than one of its arguments.
    let mut command_position = true;

    while i < tokens.len() {
        let tok = &tokens[i];
        match &tok.kind {
            TokenKind::DocComment(text) | TokenKind::LineComment(text) => {
                scope.push_comment(text);
                i += 1;
                continue;
            }
            TokenKind::Newline => {
                command_position = true;
                i += 1;
                continue;
            }
            TokenKind::Symbol('{') => {
                scope.on_open_delimiter();
                command_position = true;
                i += 1;
                continue;
            }
            TokenKind::Symbol('}') => {
                scope.on_close_delimiter(tok.end as usize, &mut facts);
                command_position = true;
                i += 1;
                continue;
            }
            TokenKind::Symbol('[') | TokenKind::Symbol(';') => {
                command_position = true;
                i += 1;
                continue;
            }
            TokenKind::Symbol(']') => {
                command_position = false;
                i += 1;
                continue;
            }
            TokenKind::Ident(ident) => {
                // `proc name {args} {body}` — the name is the word after the
                // keyword, and the body's brace opens the scope.
                if *ident == "proc" {
                    if let Some(TokenKind::Ident(name)) = tokens.get(i + 1).map(|t| &t.kind) {
                        scope.open_definition(*name, tok.start as usize, true, &mut facts);
                        scope.on_word(name);
                        // Skip the parameter list, which is a brace group of
                        // its own: `proc charge {owner amount} {body}`. Left
                        // to the loop, its closing brace would end the
                        // definition and every call in the body would fall to
                        // `<module>` — measured, 57% before this. Sixth
                        // occurrence of the shape Lua, Fortran, Julia, Scheme
                        // and HCL each needed.
                        let mut j = i + 2;
                        while j < tokens.len() && tokens[j].kind != TokenKind::Symbol('{') {
                            j += 1;
                        }
                        let mut depth = 0usize;
                        while j < tokens.len() {
                            match tokens[j].kind {
                                TokenKind::Symbol('{') => depth += 1,
                                TokenKind::Symbol('}') => {
                                    depth -= 1;
                                    if depth == 0 {
                                        break;
                                    }
                                }
                                _ => {}
                            }
                            j += 1;
                        }
                        i = j + 1;
                        command_position = true;
                        continue;
                    }
                }
                if command_position && calls.allows(ident) {
                    scope.record_call(ident, &mut facts);
                }
                scope.on_word(ident);
                command_position = false;
            }
            TokenKind::Number(num) => {
                scope.on_word(num);
                command_position = false;
            }
            _ => {
                if !matches!(tok.kind, TokenKind::StringLit(_)) {
                    scope.had_receiver = false;
                }
                command_position = false;
            }
        }
        i += 1;
    }

    scope.finish(src.len(), &mut facts);
    facts
}
