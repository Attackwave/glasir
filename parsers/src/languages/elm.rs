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
            TokenKind::DocComment(text)
            | TokenKind::LineComment(text)
            | TokenKind::BlockComment(text) => {
                scope.push_comment(text);
                i += 1;
                continue;
            }
            TokenKind::Newline => {
                i += 1;
                continue;
            }
            TokenKind::Ident(ident) => {
                match *ident {
                    "module" => {
                        let start_byte = tok.start;
                        if i + 1 < tokens.len() {
                            if let TokenKind::Ident(name) = tokens[i + 1].kind {
                                scope.open_definition_with_body_docs(
                                    name,
                                    start_byte as usize,
                                    true,
                                    false,
                                    &mut facts,
                                );
                                scope.on_word(name);
                                i += 2;
                                continue;
                            }
                        }
                    }
                    "type" => {
                        let start_byte = tok.start;
                        let mut j = i + 1;
                        if j < tokens.len() && tokens[j].kind == TokenKind::Ident("alias") {
                            j += 1;
                        }
                        if j < tokens.len() {
                            if let TokenKind::Ident(name) = tokens[j].kind {
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
                    _ => {
                        let mut j = i + 1;
                        while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                            j += 1;
                        }
                        if j < tokens.len() && tokens[j].kind == TokenKind::Symbol(':') {
                            // Type annotation: `name : Type -> Type`
                            let mut k = j + 1;
                            while k < tokens.len() && tokens[k].kind != TokenKind::Newline {
                                k += 1;
                            }
                            let start_byte = tok.start;
                            scope.close_definitions_at_or_above(0, start_byte as usize, &mut facts);
                            scope.open_definition_with_body_docs(
                                *ident,
                                start_byte as usize,
                                true,
                                true,
                                &mut facts,
                            );
                            scope.on_word(ident);
                            i = k;
                            continue;
                        } else if j < tokens.len()
                            && tokens[j].kind == TokenKind::Symbol('=')
                            && !scope.open.iter().any(|o| o.name == *ident)
                        {
                            // Function definition without annotation: `name args = ...`
                            let start_byte = tok.start;
                            scope.close_definitions_at_or_above(0, start_byte as usize, &mut facts);
                            scope.open_definition_with_body_docs(
                                *ident,
                                start_byte as usize,
                                true,
                                true,
                                &mut facts,
                            );
                            scope.on_word(ident);
                            i = j;
                            continue;
                        } else {
                            if calls.allows(ident) {
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
