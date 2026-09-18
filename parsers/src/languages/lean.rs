//! Language-specific syntax; lexical rules come from the registry.

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
            TokenKind::Ident(ident) => {
                let lower = ident.to_ascii_lowercase();
                match lower.as_str() {
                    "theorem" | "lemma" | "def" | "definition" | "inductive" | "structure" | "axiom" | "opaque" => {
                        let start_byte = tok.start;
                        if i + 1 < tokens.len() {
                            if let TokenKind::Ident(name) = tokens[i + 1].kind {
                                scope.close_definitions_at_or_above(0, start_byte as usize, &mut facts);
                                let is_fn = matches!(lower.as_str(), "theorem" | "lemma" | "def" | "definition");
                                scope.open_definition_with_body_docs(name, start_byte as usize, true, is_fn, &mut facts);
                                scope.on_word(name);
                                i += 2;
                                continue;
                            }
                        }
                    }
                    _ => {
                        if scope.had_receiver && calls.allows(ident) {
                            scope.record_call(ident, &mut facts);
                        }
                        scope.on_word(ident);
                    }
                }
            }
            TokenKind::Symbol('.') => {
                scope.on_receiver();
            }
            _ => {
                if !matches!(tok.kind, TokenKind::StringLit(_) | TokenKind::Number(_)) {
                    scope.had_receiver = false;
                }
            }
        }
        i += 1;
    }

    scope.finish(src.len(), &mut facts);
    facts
}
