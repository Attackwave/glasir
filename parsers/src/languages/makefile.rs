//! Language-specific syntax; lexical and call rules come from the registry.
//!
//! Make defines by *shape*: a target is a name at the start of a line followed
//! by a colon, and a variable is a name followed by `=`. Neither is a keyword,
//! so there is nothing for a `[generic]` table to match — measured with one,
//! the fixture yielded zero definitions.
//!
//! A recipe line is a command rather than a call in the usual sense, but the
//! first word of it is what the target depends on operationally, which is the
//! edge worth having: `charge:` running `check_limit` is exactly the "who
//! reaches what" question the graph answers.

use crate::facts::FileFacts;
use crate::lexer::{CommentStyle, Lexer, TokenKind};
use crate::scope::ScopeStack;

pub(crate) fn parse(src: &str, style: CommentStyle<'_>, calls: &crate::rules::Calls) -> FileFacts {
    let mut facts = FileFacts::new();
    let mut lexer = Lexer::new(src, style);
    let tokens = lexer.collect_all_tokens();

    let mut i = 0;
    let mut scope = ScopeStack::new();
    // A target's recipe is the indented block under it; the definition stays
    // open until the next line that starts in column zero.
    let mut line_start = true;

    while i < tokens.len() {
        let tok = &tokens[i];
        match &tok.kind {
            TokenKind::DocComment(text) | TokenKind::LineComment(text) => {
                scope.push_comment(text);
                i += 1;
                continue;
            }
            TokenKind::Newline => {
                line_start = true;
                i += 1;
                continue;
            }
            TokenKind::Ident(ident) => {
                let at_column_zero = line_start
                    && src.as_bytes().get(tok.start as usize).is_some()
                    && (tok.start == 0 || src.as_bytes()[tok.start as usize - 1] == b'\n');

                if at_column_zero {
                    let j = i + 1;
                    let next = tokens.get(j).map(|t| &t.kind);
                    // `target:` opens a recipe; `VAR =` is a statement-scoped
                    // definition whose value may hold calls.
                    // `target:`, `VAR =`, and Just's `VAR :=`. The last one
                    // became a single token when `:=` was added to the lexer
                    // for Wolfram, and testing only the two single symbols
                    // then missed it — measured, Just went 0% to 12% on
                    // `<module>` until this arm was added.
                    if next == Some(&TokenKind::Symbol(':'))
                        || next == Some(&TokenKind::Symbol('='))
                        || next == Some(&TokenKind::DoubleSymbol(":="))
                    {
                        // Close whatever the previous target opened before
                        // starting the next: a recipe has no end marker, only
                        // the next column-zero name.
                        scope.on_close_delimiter(tok.start as usize, &mut facts);
                        scope.open_definition(*ident, tok.start as usize, true, &mut facts);
                        scope.on_word(ident);
                        scope.on_open_delimiter();
                        i = j + 1;
                        line_start = false;
                        continue;
                    }
                }

                if calls.allows(ident) {
                    scope.record_call(ident, &mut facts);
                }
                scope.on_word(ident);
                line_start = false;
            }
            TokenKind::Number(num) => {
                scope.on_word(num);
                line_start = false;
            }
            _ => {
                if !matches!(tok.kind, TokenKind::StringLit(_)) {
                    scope.had_receiver = false;
                }
                line_start = false;
            }
        }
        i += 1;
    }

    scope.finish(src.len(), &mut facts);
    facts
}
