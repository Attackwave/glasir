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
            TokenKind::Symbol('$') | TokenKind::DoubleSymbol("::") => {
                scope.on_receiver();
                i += 1;
                continue;
            }
            TokenKind::Ident(ident) => {
                let mut j = i + 1;
                while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                    j += 1;
                }
                if j < tokens.len() {
                    let is_assign = matches!(&tokens[j].kind, TokenKind::DoubleSymbol("<-") | TokenKind::Symbol('='));
                    if is_assign {
                        let mut k = j + 1;
                        while k < tokens.len() && tokens[k].kind == TokenKind::Newline {
                            k += 1;
                        }
                        if k < tokens.len() && tokens[k].kind == TokenKind::Ident("function") {
                            let start_byte = tok.start;
                            scope.open_definition(*ident, start_byte as usize, true, &mut facts);
                            scope.on_word(ident);
                            i = k + 1;
                            continue;
                        }
                    }
                }

                // Check if this is a function call: `name(`
                let mut j = i + 1;
                while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                    j += 1;
                }
                if j < tokens.len()
                    && tokens[j].kind == TokenKind::Symbol('(')
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
