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
    // A newline ends a Python statement, but not inside brackets: a constant
    // such as `STOP = frozenset({` runs to its closing brace.
    let mut brackets = 0i32;

    while i < tokens.len() {
        let tok = &tokens[i];
        match tok.kind {
            TokenKind::Symbol('(' | '[' | '{') => brackets += 1,
            TokenKind::Symbol(')' | ']' | '}') => brackets = (brackets - 1).max(0),
            _ => {}
        }
        let line_start_pos = src[..tok.start as usize]
            .rfind('\n')
            .map(|p| p + 1)
            .unwrap_or(0);
        let current_line_indent = (tok.start as usize).saturating_sub(line_start_pos) as i32;

        match &tok.kind {
            TokenKind::DocComment(text) | TokenKind::LineComment(text) => {
                while scope
                    .open
                    .last()
                    .is_some_and(|o| o.depth > current_line_indent)
                {
                    scope.on_close_delimiter(tok.start as usize, &mut facts);
                }
                scope.depth = current_line_indent;
                scope.push_comment(text);
                i += 1;
                continue;
            }
            TokenKind::Newline => {
                // Without this a module constant stayed open to the end of
                // the file: its range, and the snippet for it, took in every
                // definition after it.
                if brackets == 0 && scope.open.last().is_some_and(|o| o.statement_scoped) {
                    scope.on_statement_end(tok.start as usize, &mut facts);
                }
                i += 1;
                continue;
            }
            TokenKind::StringLit(s) => {
                if s.starts_with("\"\"\"") || s.starts_with("'''") {
                    // The indent has to be applied first, exactly as the
                    // comment arm does: a docstring sits *inside* the
                    // function it documents, and `push_comment` decides by
                    // depth whether it belongs to the definition around it.
                    scope.depth = current_line_indent;
                    scope.push_comment(s);
                }
                i += 1;
                continue;
            }
            TokenKind::Symbol('@') => {
                while i < tokens.len() && tokens[i].kind != TokenKind::Newline {
                    i += 1;
                }
                continue;
            }
            TokenKind::Symbol('.') => {
                scope.on_receiver();
                i += 1;
                continue;
            }
            TokenKind::Ident(ident) => {
                while scope
                    .open
                    .last()
                    .is_some_and(|o| o.depth > current_line_indent)
                {
                    scope.on_close_delimiter(tok.start as usize, &mut facts);
                }
                scope.depth = current_line_indent;

                let mut cur_ident = *ident;
                if cur_ident == "async"
                    && i + 1 < tokens.len()
                    && tokens[i + 1].kind == TokenKind::Ident("def")
                {
                    i += 1;
                    cur_ident = "def";
                }

                // `MAX_RETRIES = 250` — Python has no keyword for a constant,
                // so the casing is what marks one. Only at module level: a
                // local in a function is not a tunable anyone asks about.
                if current_line_indent == 0
                    && crate::scope::is_screaming_case(cur_ident)
                    && i + 1 < tokens.len()
                    && tokens[i + 1].kind == TokenKind::Symbol('=')
                {
                    scope.open_statement_definition(cur_ident, tok.start as usize, &mut facts);
                    scope.on_word(cur_ident);
                    i += 1;
                    continue;
                }

                match cur_ident {
                    "def" => {
                        let start_byte = tok.start;
                        if i + 1 < tokens.len() {
                            if let TokenKind::Ident(name) = tokens[i + 1].kind {
                                scope.open_definition_with_body_docs(
                                    name,
                                    start_byte as usize,
                                    true,
                                    true,
                                    &mut facts,
                                );
                                if let Some(last) = scope.open.last_mut() {
                                    last.depth = current_line_indent + 1;
                                }
                                scope.on_word(name);
                                i += 2;
                                continue;
                            }
                        }
                    }
                    "class" => {
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
                                if let Some(last) = scope.open.last_mut() {
                                    last.depth = current_line_indent + 1;
                                }
                                scope.on_word(name);
                                i += 2;
                                continue;
                            }
                        }
                    }
                    _ => {
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
