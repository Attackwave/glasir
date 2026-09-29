//! Language-specific syntax; lexical and call rules come from the registry.
//!
//! CMake names a definition *inside* the parenthesis — `function(charge owner
//! amount)` — so the keyword and the name are separated by a delimiter rather
//! than adjacent. A `[generic]` table matches `keyword name`, which is why this
//! language needs a module: measured with one, the fixture yielded zero
//! definitions and put every call on `<module>`.
//!
//! The body is closed by a matching `endfunction`/`endmacro` rather than by a
//! brace, and every call is also written `name(...)`, so the two shapes are
//! told apart by the opening keyword alone.

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
            TokenKind::Ident(ident) => {
                let opens = matches!(*ident, "function" | "macro");
                let closes = matches!(*ident, "endfunction" | "endmacro");

                if closes {
                    scope.on_close_delimiter(tok.end as usize, &mut facts);
                    i += 1;
                    continue;
                }

                // The token after the keyword must be `(`, and the one after
                // that is the name. Newlines are skipped so a definition whose
                // parenthesis starts on the next line still counts.
                let mut j = i + 1;
                while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                    j += 1;
                }
                let has_paren = j < tokens.len() && tokens[j].kind == TokenKind::Symbol('(');

                if opens && has_paren {
                    if let Some(TokenKind::Ident(name)) = tokens.get(j + 1).map(|t| &t.kind) {
                        // The definition opens one level below the parenthesis
                        // it is named in, or the closing `)` of its own
                        // parameter list would end it — the same two-halves
                        // shape Lua, Fortran and Julia each needed.
                        scope.open_definition(*name, tok.start as usize, true, &mut facts);
                        scope.on_word(name);
                        scope.on_open_delimiter();
                        i = j + 2;
                        continue;
                    }
                }

                if has_paren && calls.allows(ident) {
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
