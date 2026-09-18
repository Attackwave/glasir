//! Language-specific syntax; lexical and call rules come from the registry.
//!
//! A Lisp calls as `(refuse owner)` — the name is the first word after an
//! opening parenthesis, never before one. The generic scanner tests for
//! `name (`, which is the mirror image, so measured with a `[generic]` table
//! the fixture recorded **zero** edges while finding every definition.

use crate::facts::FileFacts;
use crate::lexer::{CommentStyle, Lexer, TokenKind};
use crate::scope::ScopeStack;

pub(crate) fn parse(src: &str, style: CommentStyle<'_>, calls: &crate::rules::Calls) -> FileFacts {
    let mut facts = FileFacts::new();
    let mut lexer = Lexer::new(src, style);
    let tokens = lexer.collect_all_tokens();

    let mut i = 0;
    let mut scope = ScopeStack::new();
    // Set by `(`, cleared by the word that follows it: only the first word in
    // a form is the operator being applied.
    let mut head_position = false;

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
            TokenKind::Symbol('(') => {
                scope.on_open_delimiter();
                head_position = true;
                i += 1;
                continue;
            }
            TokenKind::Symbol(')') => {
                scope.on_close_delimiter(tok.end as usize, &mut facts);
                head_position = false;
                i += 1;
                continue;
            }
            TokenKind::Ident(ident) => {
                // `fn` and `lambda` are Fennel's, which shares this module: a Lisp on
                // Lua writes `(fn name [args] ...)` where Janet writes `(defn ...)`.
                let defines = matches!(
                    *ident,
                    "defn" | "defn-" | "defmacro" | "def" | "var" | "fn" | "lambda" | "local"
                        // ChiaLisp's forms, which share this module.
                        | "defun" | "defun-inline" | "defconstant"
                );
                if head_position && defines {
                    if let Some(TokenKind::Ident(name)) = tokens.get(i + 1).map(|t| &t.kind) {
                        // The definition belongs one level *below* the paren
                        // that opened its form, or that form's own closing
                        // paren ends it — the same two-halves shape Lua,
                        // Fortran, Julia and Scheme each needed.
                        scope.open_definition(*name, tok.start as usize, true, &mut facts);
                        scope.on_word(name);
                        // Janet and Fennel write the parameter list in square
                        // brackets, which the paren counter ignores; ChiaLisp
                        // writes `(defun charge (owner amount) …)` in round
                        // ones, and that list's closing paren then ended the
                        // definition — measured, 62% of references on
                        // `<module>`. Stepping over it costs the other two
                        // nothing, since they have no such list to skip.
                        let mut k = i + 2;
                        if tokens.get(k).map(|t| &t.kind) == Some(&TokenKind::Symbol('(')) {
                            let mut depth = 0usize;
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
                            k += 1;
                            // The body's own level, so a form inside it closes
                            // itself rather than the definition. Both halves
                            // are needed: skipping the list alone took 62% to
                            // 33%, not to 0. Tenth occurrence of this shape.
                            scope.on_open_delimiter();
                        }
                        i = k;
                        head_position = false;
                        continue;
                    }
                }
                if head_position && calls.allows(ident) {
                    scope.record_call(ident, &mut facts);
                }
                scope.on_word(ident);
                head_position = false;
            }
            TokenKind::Number(num) => {
                scope.on_word(num);
                head_position = false;
            }
            _ => {
                if !matches!(tok.kind, TokenKind::StringLit(_)) {
                    scope.had_receiver = false;
                }
                head_position = false;
            }
        }
        i += 1;
    }

    scope.finish(src.len(), &mut facts);
    facts
}
