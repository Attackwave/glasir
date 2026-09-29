//! Language-specific syntax; configurable lexical and call rules are supplied by the registry.

use crate::facts::FileFacts;
use crate::lexer::{CommentStyle, Lexer, TokenKind};
use crate::scope::ScopeStack;

pub(crate) fn parse(src: &str, style: CommentStyle<'_>, calls: &crate::rules::Calls) -> FileFacts {
    let mut facts = FileFacts::new();
    let mut lexer = Lexer::new(src, style);
    let tokens = lexer.collect_all_tokens();

    let mut i = 0;
    let mut scope = ScopeStack::new();
    let mut expression_start = true;
    let mut loop_header = false;

    while i < tokens.len() {
        let tok = &tokens[i];
        let starts_expression = expression_start;
        expression_start = false;
        match &tok.kind {
            TokenKind::DocComment(text)
            | TokenKind::LineComment(text)
            | TokenKind::BlockComment(text) => {
                scope.push_comment(text);
                expression_start = starts_expression;
                i += 1;
                continue;
            }
            TokenKind::Newline | TokenKind::Symbol(';') => {
                if scope.open.last().is_some_and(|o| o.statement_scoped) {
                    scope.on_statement_end(tok.end as usize, &mut facts);
                }
                expression_start = true;
                loop_header = false;
                i += 1;
                continue;
            }
            TokenKind::Symbol('{') => {
                scope.on_open_delimiter();
                expression_start = true;
            }
            TokenKind::Symbol('}') => scope.on_close_delimiter(tok.end as usize, &mut facts),
            TokenKind::Symbol('=') | TokenKind::Symbol('(') | TokenKind::Symbol(',') => {
                expression_start = true;
                scope.had_receiver = false;
            }
            TokenKind::Symbol('.')
            | TokenKind::DoubleSymbol("&.")
            | TokenKind::DoubleSymbol("::") => {
                scope.on_receiver();
                i += 1;
                continue;
            }
            TokenKind::Ident(ident) => {
                // `MAX_RETRIES = 3` — Ruby marks a constant by casing alone.
                if crate::scope::is_screaming_case(ident)
                    && i + 1 < tokens.len()
                    && tokens[i + 1].kind == TokenKind::Symbol('=')
                {
                    scope.open_statement_definition(*ident, tok.start as usize, &mut facts);
                    scope.on_word(ident);
                    i += 1;
                    continue;
                }

                match *ident {
                    "class" | "module" => {
                        let start_byte = tok.start;
                        let mut j = i + 1;
                        if j < tokens.len() && tokens[j].kind == TokenKind::DoubleSymbol("<<") {
                            j += 1;
                            // `class << self` reopens the singleton class: it
                            // groups the methods that follow but defines no
                            // name of its own. Taking `self` as one put a
                            // definition called `self` in every Ruby file that
                            // uses the form.
                            if matches!(
                                tokens.get(j).map(|t| &t.kind),
                                Some(TokenKind::Ident("self"))
                            ) {
                                scope.on_open_delimiter();
                                i = j + 1;
                                continue;
                            }
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
                                scope.on_open_delimiter();
                                scope.on_word(name);
                                i = j + 1;
                                continue;
                            }
                        }
                        scope.on_open_delimiter();
                    }
                    "def" => {
                        let start_byte = tok.start;
                        let mut j = i + 1;
                        let mut fn_name = String::new();
                        if j < tokens.len() && matches!(tokens[j].kind, TokenKind::Ident("self")) {
                            if j + 2 < tokens.len() && tokens[j + 1].kind == TokenKind::Symbol('.')
                            {
                                if let TokenKind::Ident(n) = tokens[j + 2].kind {
                                    fn_name = n.to_string();
                                    j += 3;
                                }
                            }
                        } else if j < tokens.len() {
                            if let TokenKind::Ident(n) = tokens[j].kind {
                                fn_name = n.to_string();
                                j += 1;
                            }
                        }
                        if !fn_name.is_empty() {
                            scope.open_definition_with_body_docs(
                                &fn_name,
                                start_byte as usize,
                                true,
                                true,
                                &mut facts,
                            );
                            scope.on_open_delimiter();
                            scope.on_word(&fn_name);
                            i = j;
                            continue;
                        }
                        scope.on_open_delimiter();
                    }
                    "if" | "unless" | "while" | "until" => {
                        if starts_expression {
                            scope.on_open_delimiter();
                            loop_header = matches!(*ident, "while" | "until");
                        }
                    }
                    "for" => {
                        scope.on_open_delimiter();
                        loop_header = true;
                    }
                    "do" => {
                        if !loop_header {
                            scope.on_open_delimiter();
                        }
                        loop_header = false;
                    }
                    "begin" | "case" => scope.on_open_delimiter(),
                    "end" => {
                        scope.on_close_delimiter(tok.end as usize, &mut facts);
                    }
                    _ => {
                        let mut j = i + 1;
                        while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                            j += 1;
                        }
                        if j < tokens.len()
                            && (tokens[j].kind == TokenKind::Symbol('(')
                                || (scope.had_receiver && !ident.starts_with(char::is_uppercase)))
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
