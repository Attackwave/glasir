//! Language-specific syntax; lexical and call rules come from the registry.
//!
//! An interface definition language states its dependencies as *references in
//! a field*, not as calls: Smithy writes `operations: [Charge, Refuse]` and
//! `input: ChargeInput`, WIT writes `import refuse;`. A `[generic]` table finds
//! the declarations and no edges at all — measured, 185 definitions per 1,000
//! lines and **zero** for both, which for a language whose whole purpose is
//! describing how parts connect leaves the connections out.
//!
//! A capitalised or hyphenated name appearing after `:`, inside `[…]`, or
//! after `import`/`export`/`use` is a reference to another declaration.

use crate::facts::FileFacts;
use crate::lexer::{CommentStyle, Lexer, TokenKind};
use crate::scope::ScopeStack;

/// Keywords that open a declaration whose body may reference others.
const DEFINES: [&str; 10] = [
    "service",
    "operation",
    "structure",
    "resource",
    "interface",
    "world",
    "record",
    "enum",
    "union",
    "list",
];

/// Keywords after which the next name is a reference.
const REFERS: [&str; 4] = ["import", "export", "use", "apply"];

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
            TokenKind::Ident(ident) => {
                if DEFINES.contains(ident) {
                    if let Some(TokenKind::Ident(name)) = tokens.get(i + 1).map(|t| &t.kind) {
                        scope.open_definition(*name, tok.start as usize, true, &mut facts);
                        scope.on_word(name);
                        i += 2;
                        continue;
                    }
                }
                if REFERS.contains(ident) {
                    if let Some(TokenKind::Ident(name)) = tokens.get(i + 1).map(|t| &t.kind) {
                        if calls.allows(name) {
                            scope.record_call(name, &mut facts);
                        }
                        scope.on_word(name);
                        i += 2;
                        continue;
                    }
                }

                // A field's type: `input: ChargeInput`, or a member of a list
                // `operations: [Charge, Refuse]`. Only a name that could be a
                // declaration counts — a lower-case scalar like `string` is
                // the language's own vocabulary, not a reference.
                let after_colon = i > 0
                    && matches!(
                        tokens[i - 1].kind,
                        TokenKind::Symbol(':') | TokenKind::Symbol('[') | TokenKind::Symbol(',')
                    );
                let declarable = ident
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_uppercase())
                    || ident.contains('-');
                if after_colon && declarable && calls.allows(ident) {
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
