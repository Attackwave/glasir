//! Language-specific syntax; lexical and call rules come from the registry.

use crate::facts::FileFacts;
use crate::lexer::{CommentStyle, Lexer, Token, TokenKind};
use crate::scope::ScopeStack;

/// The token after the type arguments opening at `i`, if `<` there opens any:
/// `foo<T extends Object>(` and `foo<Set<int>>(` but not `a < b`.
fn after_type_arguments(tokens: &[Token<'_>], i: usize) -> Option<usize> {
    let mut depth = 0;
    let mut k = i;
    loop {
        match tokens.get(k)?.kind {
            TokenKind::Symbol('<') => depth += 1,
            TokenKind::Symbol('>') => depth -= 1,
            TokenKind::DoubleSymbol(">>") => depth -= 2,
            TokenKind::Ident(_) | TokenKind::Newline | TokenKind::Symbol(',' | '.' | '?') => {}
            _ => return None,
        }
        k += 1;
        if depth <= 0 {
            return (depth == 0).then_some(k);
        }
    }
}

pub(crate) fn parse(src: &str, style: CommentStyle<'_>, calls: &crate::rules::Calls) -> FileFacts {
    let mut facts = FileFacts::new();
    let mut lexer = Lexer::new(src, style);
    let tokens = lexer.collect_all_tokens();

    let mut i = 0;
    let mut scope = ScopeStack::new();

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
            // Dart annotations: `@override`, `@pragma(...)`
            TokenKind::Symbol('@') => {
                i += 1;
                if i < tokens.len() {
                    if let TokenKind::Ident(_) = tokens[i].kind {
                        i += 1;
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
            TokenKind::Symbol(';') => {
                // `=> () { a(); }();`: the closure's statements end nothing.
                let depth = scope.depth;
                if !scope
                    .open
                    .last()
                    .is_some_and(|o| o.statement_scoped && o.depth < depth)
                {
                    scope.on_statement_end(tok.end as usize, &mut facts);
                }
                i += 1;
                continue;
            }
            TokenKind::Symbol('.')
            | TokenKind::DoubleSymbol("?.")
            | TokenKind::DoubleSymbol("..") => {
                scope.on_receiver();
                i += 1;
                continue;
            }
            TokenKind::Ident(ident) => {
                // Skip Dart modifiers
                if matches!(
                    *ident,
                    "abstract"
                        | "base"
                        | "final"
                        | "interface"
                        | "sealed"
                        | "mixin"
                        | "extension"
                        | "static"
                        | "const"
                        | "late"
                        | "required"
                        | "external"
                        | "factory"
                        | "async"
                        | "covariant"
                ) {
                    // Check if followed by class/mixin/extension/enum/typedef
                    if i + 1 < tokens.len()
                        && matches!(
                            tokens[i + 1].kind,
                            TokenKind::Ident("class" | "mixin" | "extension" | "enum" | "typedef")
                        )
                    {
                        i += 1;
                        continue;
                    }
                }

                match *ident {
                    // `const int maxRetries = 3;` at file scope — the type sits
                    // between the keyword and the name.
                    "const" | "final" if scope.depth == 0 => {
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
                                    tok.start as usize,
                                    &mut facts,
                                );
                                scope.on_word(name);
                                i = k + 1;
                                continue;
                            }
                        }
                    }
                    // `String get name => …` and `String get name { … }`.
                    "get"
                        if matches!(
                            tokens.get(i + 1).map(|t| &t.kind),
                            Some(TokenKind::Ident(_))
                        ) && matches!(
                            tokens.get(i + 2).map(|t| &t.kind),
                            Some(
                                TokenKind::Symbol('{')
                                    | TokenKind::DoubleSymbol("=>")
                                    | TokenKind::Ident("async" | "sync")
                            )
                        ) =>
                    {
                        if let TokenKind::Ident(name) = tokens[i + 1].kind {
                            scope.open_definition_with_body_docs(
                                name,
                                tok.start as usize,
                                true,
                                true,
                                &mut facts,
                            );
                            if tokens[i + 2].kind == TokenKind::DoubleSymbol("=>") {
                                scope.body_is_one_statement();
                            }
                            scope.on_word(name);
                            i += 2;
                            continue;
                        }
                    }
                    "class" | "mixin" | "extension" | "enum" => {
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
                    "typedef" => {
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
                    "library" | "part" => {
                        let start_byte = tok.start;
                        let mut j = i + 1;
                        if j < tokens.len() && matches!(tokens[j].kind, TokenKind::Ident("of")) {
                            j += 1;
                        }
                        let mut lib_name = String::new();
                        while j < tokens.len()
                            && tokens[j].kind != TokenKind::Symbol(';')
                            && tokens[j].kind != TokenKind::Newline
                        {
                            if let TokenKind::Ident(part) = tokens[j].kind {
                                lib_name.push_str(part);
                            } else if let TokenKind::Symbol('.') = tokens[j].kind {
                                lib_name.push('.');
                            }
                            j += 1;
                        }
                        if !lib_name.is_empty() {
                            scope.open_definition_with_body_docs(
                                &lib_name,
                                start_byte as usize,
                                false,
                                false,
                                &mut facts,
                            );
                            scope.on_word(&lib_name);
                            i = j;
                            continue;
                        }
                    }
                    _ => {
                        let mut j = i + 1;
                        while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                            j += 1;
                        }
                        let mut is_fn_def = false;
                        let mut body = i + 1;
                        if tokens
                            .get(j)
                            .is_some_and(|t| t.kind == TokenKind::Symbol('<'))
                        {
                            j = after_type_arguments(&tokens, j).unwrap_or(j);
                        }
                        if j < tokens.len() && tokens[j].kind == TokenKind::Symbol('(') {
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
                            while k < tokens.len()
                                && (tokens[k].kind == TokenKind::Newline
                                    || matches!(
                                        tokens[k].kind,
                                        TokenKind::Ident("async" | "sync")
                                            | TokenKind::Symbol('*')
                                            | TokenKind::Symbol(':')
                                    ))
                            {
                                k += 1;
                            }
                            if k < tokens.len()
                                && (tokens[k].kind == TokenKind::Symbol('{')
                                    || tokens[k].kind == TokenKind::DoubleSymbol("=>"))
                            {
                                is_fn_def = true;
                                body = k;
                            }
                        }

                        if is_fn_def
                            && !matches!(
                                *ident,
                                "if" | "while"
                                    | "for"
                                    | "switch"
                                    | "catch"
                                    | "assert"
                                    | "return"
                                    | "await"
                                    | "yield"
                                    | "throw"
                                    | "super"
                                    | "this"
                            )
                        {
                            let start_byte = tok.start;
                            scope.open_definition_with_body_docs(
                                *ident,
                                start_byte as usize,
                                true,
                                true,
                                &mut facts,
                            );
                            if tokens[body].kind == TokenKind::DoubleSymbol("=>") {
                                scope.body_is_one_statement();
                            }
                            scope.on_word(ident);
                            // Past the parameters: the braces of named ones
                            // would close the definition just opened.
                            i = body;
                            continue;
                        }

                        let mut j = i + 1;
                        while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                            j += 1;
                        }
                        let mut is_call = false;
                        if j < tokens.len() {
                            if tokens[j].kind == TokenKind::Symbol('(') {
                                is_call = true;
                            } else if tokens[j].kind == TokenKind::Symbol('<') {
                                is_call = after_type_arguments(&tokens, j).is_some_and(|k| {
                                    tokens[k..]
                                        .iter()
                                        .find(|t| t.kind != TokenKind::Newline)
                                        .is_some_and(|t| t.kind == TokenKind::Symbol('('))
                                });
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
