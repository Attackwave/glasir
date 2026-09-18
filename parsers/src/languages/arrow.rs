//! Language-specific syntax; lexical and call rules come from the registry.
//!
//! A diagram language states its edges directly: `charge --> refuse`. There is
//! no call syntax to match, so a `[generic]` table finds definitions and no
//! relationships at all — which for a language whose entire content *is*
//! relationships leaves nothing worth indexing.
//!
//! Every node named on either side of an arrow is a definition, and the arrow
//! itself is the edge.

use crate::facts::FileFacts;
use crate::lexer::{CommentStyle, Lexer, TokenKind};

pub(crate) fn parse(src: &str, style: CommentStyle<'_>, calls: &crate::rules::Calls) -> FileFacts {
    let mut facts = FileFacts::new();
    let mut lexer = Lexer::new(src, style);
    let tokens = lexer.collect_all_tokens();

    // The last identifier seen on this line, which an arrow refers back to.
    let mut left: Option<(String, u32, u32)> = None;
    let mut i = 0;

    while i < tokens.len() {
        match &tokens[i].kind {
            TokenKind::Newline => {
                left = None;
                i += 1;
                continue;
            }
            TokenKind::Ident(ident) => {
                // A shape label (`graph`, `flowchart`, `TD`) is syntax, not a
                // node.
                if !calls.allows(ident) {
                    i += 1;
                    continue;
                }
                if !facts.defines.contains(&(*ident).to_string()) {
                    facts.add_definition(
                        *ident,
                        (tokens[i].start, tokens[i].end),
                        None,
                    );
                }
                left = Some(((*ident).to_string(), tokens[i].start, tokens[i].end));
            }
            // `-->`, `---`, `==>`: the lexer splits them, so any run of dashes
            // or equals between two identifiers is an edge.
            TokenKind::Symbol('-') | TokenKind::Symbol('>') | TokenKind::Symbol('=')
            | TokenKind::DoubleSymbol("--") | TokenKind::DoubleSymbol("->")
            | TokenKind::DoubleSymbol("==") | TokenKind::DoubleSymbol("=>") => {
                let mut j = i + 1;
                while j < tokens.len()
                    && matches!(
                        tokens[j].kind,
                        TokenKind::Symbol('-')
                            | TokenKind::Symbol('>')
                            | TokenKind::Symbol('=')
                            | TokenKind::DoubleSymbol("--")
                            | TokenKind::DoubleSymbol("->")
                            | TokenKind::DoubleSymbol("==")
                            | TokenKind::DoubleSymbol("=>")
                    )
                {
                    j += 1;
                }
                if let (Some((from, _, _)), Some(TokenKind::Ident(to))) =
                    (left.clone(), tokens.get(j).map(|t| &t.kind))
                {
                    if calls.allows(to) {
                        if !facts.defines.contains(&(*to).to_string()) {
                            facts.add_definition(*to, (tokens[j].start, tokens[j].end), None);
                        }
                        facts.add_call(from, (*to).to_string(), false);
                        left = Some(((*to).to_string(), tokens[j].start, tokens[j].end));
                    }
                }
                i = j + 1;
                continue;
            }
            _ => {}
        }
        i += 1;
    }

    facts
}
