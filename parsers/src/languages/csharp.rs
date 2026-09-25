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
    // Where the modifiers before a declaration begin, so its range, and the
    // snippet handed back for it, starts at `public static` and not at the name.
    let mut declared_at: Option<u32> = None;

    while i < tokens.len() {
        let tok = &tokens[i];
        match &tok.kind {
            TokenKind::DocComment(text)
            | TokenKind::LineComment(text)
            | TokenKind::BlockComment(text) => {
                scope.push_comment(text);
                i += 1;
                continue;
            }
            TokenKind::Newline => {
                i += 1;
                continue;
            }
            TokenKind::Symbol('[') => {
                let mut depth = 1;
                i += 1;
                while i < tokens.len() && depth > 0 {
                    if tokens[i].kind == TokenKind::Symbol('[') {
                        depth += 1;
                    } else if tokens[i].kind == TokenKind::Symbol(']') {
                        depth -= 1;
                    }
                    i += 1;
                }
                continue;
            }
            // A statement ends a definition that has no braces of its own: a
            // `const`, an expression-bodied member. Without it both stayed
            // open to the end of the class and took every call after them.
            TokenKind::Symbol(';') => {
                declared_at = None;
                scope.on_statement_end(tok.end as usize, &mut facts);
                scope.had_receiver = false;
                i += 1;
                continue;
            }
            TokenKind::Symbol('{') => {
                declared_at = None;
                scope.on_open_delimiter();
                i += 1;
                continue;
            }
            TokenKind::Symbol('}') => {
                declared_at = None;
                scope.on_close_delimiter(tok.end as usize, &mut facts);
                i += 1;
                continue;
            }
            TokenKind::Symbol('.') | TokenKind::DoubleSymbol("?.") => {
                scope.on_receiver();
                i += 1;
                continue;
            }
            TokenKind::Ident(ident) => {
                if matches!(
                    *ident,
                    "public"
                        | "protected"
                        | "private"
                        | "internal"
                        | "static"
                        | "readonly"
                        | "volatile"
                        | "virtual"
                        | "override"
                        | "abstract"
                        | "sealed"
                        | "async"
                        | "extern"
                        | "unsafe"
                        | "partial"
                        | "required"
                ) {
                    declared_at.get_or_insert(tok.start);
                    i += 1;
                    continue;
                }

                match *ident {
                    "namespace" => {
                        let start_byte = tok.start;
                        let mut j = i + 1;
                        let mut ns_name = String::new();
                        while j < tokens.len()
                            && tokens[j].kind != TokenKind::Symbol(';')
                            && tokens[j].kind != TokenKind::Symbol('{')
                            && tokens[j].kind != TokenKind::Newline
                        {
                            if let TokenKind::Ident(part) = tokens[j].kind {
                                ns_name.push_str(part);
                            } else if let TokenKind::Symbol('.') = tokens[j].kind {
                                ns_name.push('.');
                            }
                            j += 1;
                        }
                        if !ns_name.is_empty() {
                            let opens_body =
                                j < tokens.len() && tokens[j].kind == TokenKind::Symbol('{');
                            scope.open_definition_with_body_docs(
                                &ns_name,
                                start_byte as usize,
                                opens_body,
                                false,
                                &mut facts,
                            );
                            scope.on_word(&ns_name);
                            i = j;
                            continue;
                        }
                    }
                    "const" => {
                        let mut k = i;
                        let mut ok = false;
                        while k + 1 < tokens.len() {
                            match tokens[k + 1].kind {
                                TokenKind::Symbol('=') => {
                                    ok = true;
                                    break;
                                }
                                TokenKind::Symbol(';') | TokenKind::Symbol('(') => break,
                                _ => k += 1,
                            }
                        }
                        if ok {
                            if let TokenKind::Ident(name) = tokens[k].kind {
                                scope.open_statement_definition(
                                    name,
                                    declared_at.take().unwrap_or(tok.start) as usize,
                                    &mut facts,
                                );
                                scope.on_word(name);
                                i = k + 1;
                                continue;
                            }
                        }
                    }
                    "class" | "struct" | "interface" | "enum" | "record" => {
                        let start_byte = declared_at.take().unwrap_or(tok.start);
                        let mut j = i + 1;
                        if j < tokens.len()
                            && matches!(tokens[j].kind, TokenKind::Ident("class" | "struct"))
                        {
                            j += 1;
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
                                scope.on_word(name);
                                i = j + 1;
                                continue;
                            }
                        }
                    }
                    _ => {
                        let mut j = skip_newlines(&tokens, i + 1);
                        // `Run<TResult>(` is a method as much as `Run(`.
                        if let Some(after) = skip_type_arguments(&tokens, j) {
                            j = skip_newlines(&tokens, after);
                        }
                        let call_paren = j;
                        let mut is_method_def = false;
                        let mut expression_bodied = false;
                        if !crate::scope::is_control_keyword(ident)
                            && j < tokens.len()
                            && tokens[j].kind == TokenKind::Symbol('(')
                        {
                            let mut k = j + 1;
                            let mut paren_depth = 1;
                            while k < tokens.len() && paren_depth > 0 {
                                if tokens[k].kind == TokenKind::Symbol('(') {
                                    paren_depth += 1;
                                } else if tokens[k].kind == TokenKind::Symbol(')') {
                                    paren_depth -= 1;
                                }
                                k += 1;
                            }
                            // Constraints and a constructor's `: base(...)` sit
                            // between the parameters and the body.
                            while k < tokens.len()
                                && (tokens[k].kind == TokenKind::Newline
                                    || matches!(
                                        tokens[k].kind,
                                        TokenKind::Ident(_)
                                            | TokenKind::Symbol(':')
                                            | TokenKind::Symbol(',')
                                    ))
                            {
                                k += 1;
                            }
                            if k < tokens.len() && scope.depth > 0 {
                                match tokens[k].kind {
                                    TokenKind::Symbol('{') => is_method_def = true,
                                    // `Run(int x) => x > 0;`: the body is one
                                    // expression and ends at the semicolon.
                                    TokenKind::DoubleSymbol("=>") => {
                                        is_method_def = true;
                                        expression_bodied = true;
                                    }
                                    _ => {}
                                }
                            }
                        }

                        if is_method_def {
                            let start_byte = declared_at.take().unwrap_or(tok.start) as usize;
                            if expression_bodied {
                                scope.open_statement_definition(*ident, start_byte, &mut facts);
                            } else {
                                scope.open_definition(*ident, start_byte, true, &mut facts);
                            }
                            scope.on_word(ident);
                            i += 1;
                            continue;
                        }

                        let j = call_paren;
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

fn skip_newlines(tokens: &[crate::lexer::Token<'_>], mut j: usize) -> usize {
    while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
        j += 1;
    }
    j
}

/// The index after `<...>` when `j` opens a type argument list, `None` when
/// the `<` is a comparison. Only names, dots, commas, `?` and brackets may sit
/// inside, which `a < b` fails as soon as it reaches an operator or a paren.
fn skip_type_arguments(tokens: &[crate::lexer::Token<'_>], j: usize) -> Option<usize> {
    if tokens.get(j)?.kind != TokenKind::Symbol('<') {
        return None;
    }
    let mut depth = 0i32;
    let mut k = j;
    while let Some(t) = tokens.get(k) {
        match t.kind {
            TokenKind::Symbol('<') => depth += 1,
            TokenKind::Symbol('>') => depth -= 1,
            TokenKind::DoubleSymbol(">>") => depth -= 2,
            TokenKind::Ident(_)
            | TokenKind::Symbol('.' | ',' | '?' | '[' | ']')
            | TokenKind::Newline => {}
            _ => return None,
        }
        k += 1;
        if depth <= 0 {
            return (depth == 0).then_some(k);
        }
    }
    None
}
