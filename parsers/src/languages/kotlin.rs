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
    let mut in_type = false;
    // Where an expression-bodied function ends: its lambdas' braces are not
    // its body, and the first `}` used to close it.
    let mut expression_end: Option<usize> = None;

    while i < tokens.len() {
        let tok = &tokens[i];
        if expression_end == Some(i) {
            expression_end = None;
            scope.on_statement_end(tok.start as usize, &mut facts);
        }
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
            // Kotlin annotations: `@Annotation` or `@file:Annotation` or `@Annotation(...)`
            TokenKind::Symbol('@') => {
                i += 1;
                if i < tokens.len() {
                    if let TokenKind::Ident(_) = tokens[i].kind {
                        i += 1;
                        if i < tokens.len() && tokens[i].kind == TokenKind::Symbol(':') {
                            i += 1;
                            if i < tokens.len() {
                                if let TokenKind::Ident(_) = tokens[i].kind {
                                    i += 1;
                                }
                            }
                        }
                        if i < tokens.len() && tokens[i].kind == TokenKind::Symbol('(') {
                            let mut depth = 1;
                            i += 1;
                            while i < tokens.len() && depth > 0 {
                                if tokens[i].kind == TokenKind::Symbol('(') {
                                    depth += 1;
                                } else if tokens[i].kind == TokenKind::Symbol(')') {
                                    depth -= 1;
                                }
                                i += 1;
                            }
                        }
                    }
                }
                continue;
            }
            TokenKind::Symbol(':') => {
                // `?:` is the elvis operator, and a call follows it.
                in_type = i == 0 || tokens[i - 1].kind != TokenKind::Symbol('?');
                i += 1;
                continue;
            }
            TokenKind::Symbol('{') => {
                in_type = false;
                scope.on_open_delimiter();
                i += 1;
                continue;
            }
            TokenKind::Symbol('}') => {
                in_type = false;
                scope.on_close_delimiter(tok.end as usize, &mut facts);
                i += 1;
                continue;
            }
            TokenKind::Symbol(';') => {
                in_type = false;
                scope.on_statement_end(tok.end as usize, &mut facts);
                i += 1;
                continue;
            }
            TokenKind::Symbol('=') | TokenKind::Symbol(',') => {
                in_type = false;
                i += 1;
                continue;
            }
            TokenKind::Symbol('(') | TokenKind::Symbol(')') => {
                in_type = false;
                i += 1;
                continue;
            }
            TokenKind::Symbol('.')
            | TokenKind::DoubleSymbol("?.")
            | TokenKind::DoubleSymbol("::") => {
                in_type = false;
                scope.on_receiver();
                i += 1;
                continue;
            }
            TokenKind::Ident(ident) => {
                // Skip Kotlin modifiers
                if matches!(
                    *ident,
                    "public"
                        | "private"
                        | "protected"
                        | "internal"
                        | "open"
                        | "final"
                        | "abstract"
                        | "sealed"
                        | "data"
                        | "enum"
                        | "annotation"
                        | "inline"
                        | "noinline"
                        | "crossinline"
                        | "value"
                        | "infix"
                        | "operator"
                        | "suspend"
                        | "tailrec"
                        | "vararg"
                        | "override"
                        | "lateinit"
                        | "inner"
                        | "external"
                        | "actual"
                        | "expect"
                        | "const"
                ) {
                    i += 1;
                    continue;
                }

                if matches!(*ident, "is" | "as") {
                    in_type = true;
                }

                match *ident {
                    "package" => {
                        let start_byte = tok.start;
                        let mut j = i + 1;
                        let mut pkg_name = String::new();
                        while j < tokens.len()
                            && tokens[j].kind != TokenKind::Symbol(';')
                            && tokens[j].kind != TokenKind::Newline
                        {
                            if let TokenKind::Ident(part) = tokens[j].kind {
                                pkg_name.push_str(part);
                            } else if let TokenKind::Symbol('.') = tokens[j].kind {
                                pkg_name.push('.');
                            }
                            j += 1;
                        }
                        if !pkg_name.is_empty() {
                            scope.open_definition_with_body_docs(
                                &pkg_name,
                                start_byte as usize,
                                false,
                                false,
                                &mut facts,
                            );
                            scope.on_word(&pkg_name);
                            i = j;
                            continue;
                        }
                    }
                    "class" | "interface" | "object" => {
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
                                scope.on_word(name);
                                i += 2;
                                continue;
                            }
                        }
                    }
                    "companion" => {
                        // companion object [Name]
                        let start_byte = tok.start;
                        if i + 1 < tokens.len()
                            && matches!(tokens[i + 1].kind, TokenKind::Ident("object"))
                        {
                            if i + 2 < tokens.len() {
                                if let TokenKind::Ident(name) = tokens[i + 2].kind {
                                    scope.open_definition_with_body_docs(
                                        name,
                                        start_byte as usize,
                                        true,
                                        false,
                                        &mut facts,
                                    );
                                    scope.on_word(name);
                                    i += 3;
                                    continue;
                                }
                            }
                            scope.open_definition_with_body_docs(
                                "Companion",
                                start_byte as usize,
                                true,
                                false,
                                &mut facts,
                            );
                            scope.on_word("Companion");
                            i += 2;
                            continue;
                        }
                        i += 1;
                        continue;
                    }
                    "fun" => {
                        let start_byte = tok.start;
                        // fun [<T>] [Receiver.]fnName(...)
                        let mut j = i + 1;
                        // Skip type parameters <...> if any
                        if j < tokens.len() && tokens[j].kind == TokenKind::Symbol('<') {
                            let mut depth = 1;
                            j += 1;
                            while j < tokens.len() && depth > 0 {
                                if tokens[j].kind == TokenKind::Symbol('<') {
                                    depth += 1;
                                } else if tokens[j].kind == TokenKind::Symbol('>') {
                                    depth -= 1;
                                }
                                j += 1;
                            }
                        }
                        // Now we might have Receiver.funcName or just funcName
                        let mut last_ident = None;
                        while j < tokens.len() {
                            if let TokenKind::Ident(name) = tokens[j].kind {
                                last_ident = Some(name);
                                if j + 1 < tokens.len()
                                    && (tokens[j + 1].kind == TokenKind::Symbol('.')
                                        || tokens[j + 1].kind == TokenKind::Symbol('<'))
                                {
                                    if tokens[j + 1].kind == TokenKind::Symbol('<') {
                                        let mut depth = 1;
                                        j += 2;
                                        while j < tokens.len() && depth > 0 {
                                            if tokens[j].kind == TokenKind::Symbol('<') {
                                                depth += 1;
                                            } else if tokens[j].kind == TokenKind::Symbol('>') {
                                                depth -= 1;
                                            }
                                            j += 1;
                                        }
                                        if j < tokens.len()
                                            && tokens[j].kind == TokenKind::Symbol('.')
                                        {
                                            j += 1;
                                            continue;
                                        }
                                        break;
                                    } else {
                                        j += 2;
                                        continue;
                                    }
                                }
                                j += 1;
                                break;
                            } else {
                                break;
                            }
                        }
                        if let Some(fn_name) = last_ident {
                            scope.open_definition_with_body_docs(
                                fn_name,
                                start_byte as usize,
                                true,
                                true,
                                &mut facts,
                            );
                            if expression_end.is_none() {
                                if let Some(end) = expression_body_end(&tokens, j) {
                                    if let Some(last) = scope.open.last_mut() {
                                        last.statement_scoped = true;
                                    }
                                    expression_end = Some(end);
                                }
                            }
                            scope.on_word(fn_name);
                            i = j;
                            continue;
                        }
                    }
                    "val" | "var" => {
                        let in_function = scope.open.iter().any(|o| o.collects_body_docs);
                        if in_function {
                            i += 1;
                            continue;
                        }
                        let start_byte = tok.start;
                        let mut j = i + 1;
                        // Skip type params <...> if any
                        if j < tokens.len() && tokens[j].kind == TokenKind::Symbol('<') {
                            let mut depth = 1;
                            j += 1;
                            while j < tokens.len() && depth > 0 {
                                if tokens[j].kind == TokenKind::Symbol('<') {
                                    depth += 1;
                                } else if tokens[j].kind == TokenKind::Symbol('>') {
                                    depth -= 1;
                                }
                                j += 1;
                            }
                        }
                        let mut last_ident = None;
                        while j < tokens.len() {
                            if let TokenKind::Ident(name) = tokens[j].kind {
                                last_ident = Some(name);
                                if j + 1 < tokens.len() && tokens[j].kind == TokenKind::Symbol('.')
                                {
                                    j += 2;
                                    continue;
                                }
                                j += 1;
                                break;
                            } else {
                                break;
                            }
                        }
                        if let Some(prop_name) = last_ident {
                            scope.open_definition_with_body_docs(
                                prop_name,
                                start_byte as usize,
                                false,
                                false,
                                &mut facts,
                            );
                            scope.on_word(prop_name);
                            i = j;
                            continue;
                        }
                    }
                    "typealias" => {
                        let start_byte = tok.start;
                        if i + 1 < tokens.len() {
                            if let TokenKind::Ident(name) = tokens[i + 1].kind {
                                scope.open_definition_with_body_docs(
                                    name,
                                    start_byte as usize,
                                    false,
                                    false,
                                    &mut facts,
                                );
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
                        let mut is_call = false;
                        if j < tokens.len() && !in_type {
                            if matches!(
                                tokens[j].kind,
                                TokenKind::Symbol('(') | TokenKind::Symbol('{')
                            ) {
                                is_call = true;
                            } else if tokens[j].kind == TokenKind::Symbol('<') {
                                let mut depth = 1;
                                let mut k = j + 1;
                                while k < tokens.len() && depth > 0 {
                                    if tokens[k].kind == TokenKind::Symbol('<') {
                                        depth += 1;
                                    } else if tokens[k].kind == TokenKind::Symbol('>') {
                                        depth -= 1;
                                    }
                                    k += 1;
                                }
                                while k < tokens.len() && tokens[k].kind == TokenKind::Newline {
                                    k += 1;
                                }
                                if k < tokens.len()
                                    && (tokens[k].kind == TokenKind::Symbol('(')
                                        || tokens[k].kind == TokenKind::Symbol('{'))
                                {
                                    is_call = true;
                                }
                            }
                        }

                        if is_call && calls.allows(ident) {
                            scope.record_call(ident, &mut facts);
                        } else if !is_call {
                            scope.had_receiver = false;
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

/// For `fun f(…): T = expr`, the index of the token ending `expr`: a line
/// break outside brackets when the next line does not continue it (`.`,
/// `?.`, `?:`, an operator), or the `}` closing what encloses the function.
/// `None` for a block body or no body.
fn expression_body_end(tokens: &[crate::lexer::Token<'_>], mut k: usize) -> Option<usize> {
    let mut depth = 0i32;
    loop {
        match tokens.get(k)?.kind {
            TokenKind::Symbol('(' | '[') => depth += 1,
            TokenKind::Symbol(')' | ']') => depth -= 1,
            TokenKind::Symbol('=') if depth == 0 => break,
            TokenKind::Symbol('{' | '}' | ';') if depth == 0 => return None,
            TokenKind::Ident("fun" | "val" | "var" | "class" | "interface" | "object")
                if depth == 0 =>
            {
                return None
            }
            _ => {}
        }
        k += 1;
    }
    k += 1;
    let mut depth = 0i32;
    let mut seen = false;
    while let Some(t) = tokens.get(k) {
        match t.kind {
            TokenKind::Symbol('(' | '[' | '{') => depth += 1,
            TokenKind::Symbol(')' | ']' | '}') if depth == 0 => return Some(k),
            TokenKind::Symbol(')' | ']' | '}') => depth -= 1,
            TokenKind::Symbol(';') if depth == 0 => return Some(k),
            TokenKind::Newline if depth == 0 && seen => {
                let next = tokens[k + 1..].iter().find(|t| {
                    !matches!(
                        t.kind,
                        TokenKind::Newline
                            | TokenKind::LineComment(_)
                            | TokenKind::BlockComment(_)
                            | TokenKind::DocComment(_)
                    )
                });
                let continues = next.is_some_and(|t| {
                    matches!(
                        t.kind,
                        TokenKind::Symbol('.' | '?' | ':' | '+' | '-' | '*' | '/' | '%' | '<' | '>')
                            | TokenKind::DoubleSymbol(
                                "?." | "&&" | "||" | "->" | "==" | "!=" | "<=" | ">=" | "::"
                            )
                    )
                });
                if !continues {
                    return Some(k);
                }
            }
            TokenKind::Newline => {}
            _ => seen = true,
        }
        k += 1;
    }
    Some(k)
}
