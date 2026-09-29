//! Language-specific syntax; lexical and call rules come from the registry.
//!
//! A description format states its dependencies as *references*, each with its
//! own marker: a device tree writes `<&gic>`, a linker script `*(.text)`,
//! TableGen `def X : Base<…>`, RON `Charge(fallback: Refuse)` and a Go module
//! `require ledger/refuse`. None is a call, so a `[generic]` table finds the
//! declarations and no edges — and for a format whose whole content is how
//! parts refer to each other, that is the CSS failure again: it looks
//! extracted while answering nothing.
//!
//! One module, because the shape is the same in each: a declaration owns
//! whatever references follow it until the next one.

use crate::facts::FileFacts;
use crate::lexer::{CommentStyle, Lexer, TokenKind};
use crate::scope::ScopeStack;

/// A name that could head a struct literal: capitalised, as every RON and
/// TableGen declaration is.
fn declarable_head(ident: &str) -> bool {
    ident.chars().next().is_some_and(|c| c.is_uppercase())
}

/// Keywords after which the next name is a reference rather than a definition.
const REFERS: [&str; 6] = [
    "require", "replace", "import", "extends", "amends", "include",
];

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
            // `*(.text.warn_owner)` in a linker script: the section it pulls in
            // is named inside the parenthesis after a wildcard, and the
            // leading dot is part of the name. Without this the format
            // measured 176 definitions per 1,000 lines and **zero** edges —
            // for a file whose entire purpose is saying which section goes
            // where.
            TokenKind::Symbol('*') => {
                if tokens.get(i + 1).map(|t| &t.kind) == Some(&TokenKind::Symbol('(')) {
                    let mut j = i + 2;
                    if tokens.get(j).map(|t| &t.kind) == Some(&TokenKind::Symbol('.')) {
                        j += 1;
                    }
                    if let Some(TokenKind::Ident(name)) = tokens.get(j).map(|t| &t.kind) {
                        if calls.allows(name) {
                            scope.record_call(name, &mut facts);
                        }
                        scope.on_word(name);
                        i = j + 1;
                        continue;
                    }
                }
                i += 1;
                continue;
            }
            // A section header is `.charge : {`, so the dot opens a name the
            // wildcard above refers back to.
            TokenKind::Symbol('.') => {
                if let Some(TokenKind::Ident(name)) = tokens.get(i + 1).map(|t| &t.kind) {
                    if tokens.get(i + 2).map(|t| &t.kind) == Some(&TokenKind::Symbol(':')) {
                        scope.open_definition(*name, tok.start as usize, true, &mut facts);
                        scope.on_word(name);
                        i += 3;
                        continue;
                    }
                }
                i += 1;
                continue;
            }
            // `<&gic>` in a device tree, and `&label` anywhere: a phandle is a
            // reference to a node declared elsewhere.
            TokenKind::Symbol('&') => {
                if let Some(TokenKind::Ident(name)) = tokens.get(i + 1).map(|t| &t.kind) {
                    if calls.allows(name) {
                        scope.record_call(name, &mut facts);
                    }
                    scope.on_word(name);
                    i += 2;
                    continue;
                }
                i += 1;
                continue;
            }
            TokenKind::Ident(ident) => {
                // `label: node {` — a device tree labels a node before its
                // name, and the label is what a phandle refers to.
                if tokens.get(i + 1).map(|t| &t.kind) == Some(&TokenKind::Symbol(':')) {
                    // `def X : Base<…>` is the opposite: the name comes first
                    // and what follows the colon is the parent it inherits.
                    let inherits = i > 0
                        && matches!(
                            tokens[i - 1].kind,
                            TokenKind::Ident("def") | TokenKind::Ident("class")
                        );
                    if !inherits {
                        scope.open_definition(*ident, tok.start as usize, true, &mut facts);
                        scope.on_word(ident);
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

                // `def Charge : Action<…>` and `module ledger/charge`.
                if matches!(*ident, "def" | "class" | "module" | "function") {
                    if let Some(TokenKind::Ident(name)) = tokens.get(i + 1).map(|t| &t.kind) {
                        scope.open_definition(*name, tok.start as usize, true, &mut facts);
                        scope.on_word(name);
                        // Whatever follows a colon here is a parent.
                        if tokens.get(i + 2).map(|t| &t.kind) == Some(&TokenKind::Symbol(':')) {
                            if let Some(TokenKind::Ident(base)) = tokens.get(i + 3).map(|t| &t.kind)
                            {
                                if calls.allows(base) {
                                    scope.record_call(base, &mut facts);
                                }
                            }
                        }
                        i += 2;
                        continue;
                    }
                }

                // `Charge(` opens a struct literal, and RON's structure is
                // what a reader asks about — which of them names which. Only
                // at the head of a call, so a bare capitalised word in a value
                // stays a reference rather than becoming a second definition
                // of the same name.
                let opens_struct = declarable_head(ident)
                    && tokens.get(i + 1).map(|t| &t.kind) == Some(&TokenKind::Symbol('('))
                    && !matches!(
                        tokens.get(i.wrapping_sub(1)).map(|t| &t.kind),
                        Some(TokenKind::Symbol(':')) | Some(TokenKind::Symbol('='))
                    );
                if opens_struct && !facts.defines.contains(&(*ident).to_string()) {
                    scope.open_definition(*ident, tok.start as usize, true, &mut facts);
                    scope.on_word(ident);
                    scope.on_open_delimiter();
                    i += 2;
                    continue;
                }

                // A capitalised name in a value position is a reference to
                // another declaration: RON's `fallback: Refuse`, TableGen's
                // `Action fallback = Refuse`.
                let in_value = i > 0
                    && matches!(
                        tokens[i - 1].kind,
                        TokenKind::Symbol(':') | TokenKind::Symbol('=') | TokenKind::Symbol(',')
                    );
                let declarable = ident.chars().next().is_some_and(|c| c.is_uppercase());
                let called = tokens.get(i + 1).map(|t| &t.kind) == Some(&TokenKind::Symbol('('));
                if (in_value && declarable || called) && calls.allows(ident) {
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
