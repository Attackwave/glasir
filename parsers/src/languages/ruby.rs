//! Language-specific syntax; configurable lexical and call rules are supplied by the registry.

use crate::facts::FileFacts;
use crate::lexer::{is_atom_or_member, is_key, CommentStyle, Lexer, Token, TokenKind};
use crate::scope::ScopeStack;

/// `def verb = "GET"` and `def call(req) = go(req)` have no `end`. The token at
/// `i` follows the name; `def name=(value)` is a setter and has one.
fn is_endless(tokens: &[Token<'_>], i: usize) -> bool {
    let mut k = i;
    if tokens
        .get(k)
        .is_some_and(|t| t.kind == TokenKind::Symbol('('))
    {
        let mut depth = 0;
        while let Some(t) = tokens.get(k) {
            match t.kind {
                TokenKind::Symbol('(') => depth += 1,
                TokenKind::Symbol(')') => depth -= 1,
                _ => {}
            }
            k += 1;
            if depth == 0 {
                break;
            }
        }
    }
    tokens
        .get(k)
        .is_some_and(|t| t.kind == TokenKind::Symbol('=') && (k > i || t.start > tokens[k - 1].end))
}

pub(crate) fn parse(src: &str, style: CommentStyle<'_>, calls: &crate::rules::Calls) -> FileFacts {
    let mut facts = FileFacts::new();
    let mut lexer = Lexer::new(src, style);
    crate::lexer::configure(&mut lexer, crate::Language::Ruby);
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
            // `:name=` and `:[]=` are symbols, and what follows them no value.
            TokenKind::Symbol('=') if crate::lexer::ends_a_setter_symbol(&tokens, i) => {}
            TokenKind::Symbol('=' | '(' | ',') | TokenKind::DoubleSymbol("||" | "&&" | "<<") => {
                expression_start = true;
                scope.had_receiver = false;
            }
            TokenKind::Symbol('.')
            | TokenKind::DoubleSymbol("&.")
            | TokenKind::DoubleSymbol("::") => {
                scope.on_receiver();
                // `Journey::Router::Utils.escape` names more than its last word.
                let mut chain: Vec<&str> = Vec::new();
                let mut k = i;
                while let Some(TokenKind::Ident(w)) = k.checked_sub(1).map(|p| &tokens[p].kind) {
                    if !w.starts_with(|c: char| c.is_uppercase()) {
                        break;
                    }
                    chain.push(w);
                    if k < 2 || tokens[k - 2].kind != TokenKind::DoubleSymbol("::") {
                        break;
                    }
                    k -= 2;
                }
                if chain.len() > 1 {
                    chain.reverse();
                    scope.module_receiver = Some(chain.join("::"));
                }
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

                // `self.class`, `:end`, `if:` are no keywords.
                let word = if is_key(&tokens, i) || is_atom_or_member(&tokens, i) {
                    ""
                } else {
                    *ident
                };
                match word {
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
                                // `def object.name`: the method is `name`.
                                if let (Some(TokenKind::Symbol('.')), Some(TokenKind::Ident(m))) = (
                                    tokens.get(j).map(|t| &t.kind),
                                    tokens.get(j + 1).map(|t| &t.kind),
                                ) {
                                    fn_name = (*m).to_string();
                                    j += 2;
                                }
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
                            if is_endless(&tokens, j) {
                                scope.body_is_one_statement();
                            } else {
                                scope.on_open_delimiter();
                            }
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
