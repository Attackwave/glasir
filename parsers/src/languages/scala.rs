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
            // Scala annotations: `@annotation` or `@annotation(...)`
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
            TokenKind::Symbol(':') => {
                in_type = true;
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
            TokenKind::Symbol('.') => {
                in_type = false;
                scope.on_receiver();
                i += 1;
                continue;
            }
            TokenKind::Ident(ident) => {
                // Skip Scala modifiers
                if matches!(
                    *ident,
                    "public"
                        | "private"
                        | "protected"
                        | "override"
                        | "final"
                        | "abstract"
                        | "lazy"
                        | "implicit"
                        | "sealed"
                        | "case"
                        | "inline"
                        | "opaque"
                        | "open"
                        | "transparent"
                        | "infix"
                ) {
                    // Check for private[this] or protected[scope]
                    if i + 1 < tokens.len() && tokens[i + 1].kind == TokenKind::Symbol('[') {
                        let mut depth = 1;
                        i += 2;
                        while i < tokens.len() && depth > 0 {
                            if tokens[i].kind == TokenKind::Symbol('[') {
                                depth += 1;
                            } else if tokens[i].kind == TokenKind::Symbol(']') {
                                depth -= 1;
                            }
                            i += 1;
                        }
                    } else {
                        i += 1;
                    }
                    continue;
                }

                if matches!(*ident, "as" | "is" | "extends" | "with" | "derives") {
                    in_type = true;
                }

                match *ident {
                    "package" => {
                        let start_byte = tok.start;
                        let mut j = i + 1;
                        let mut pkg_name = String::new();
                        while j < tokens.len()
                            && tokens[j].kind != TokenKind::Symbol(';')
                            && tokens[j].kind != TokenKind::Symbol('{')
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
                    "class" | "trait" | "object" | "enum" => {
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
                    "def" => {
                        let start_byte = tok.start;
                        if i + 1 < tokens.len() {
                            if let TokenKind::Ident(name) = tokens[i + 1].kind {
                                // `def this(…)` is Scala's auxiliary constructor.
                                // Named after its class, the way Java names one,
                                // because a symbol called `this` collides with
                                // every other file that has one — measured, 113
                                // of them across the language's own library.
                                let name = if name == "this" {
                                    scope
                                        .open
                                        .iter()
                                        .rev()
                                        .find(|o| o.opens_body)
                                        .map(|o| o.name.clone())
                                        .unwrap_or_else(|| "this".to_string())
                                } else {
                                    name.to_string()
                                };
                                scope.open_definition_with_body_docs(
                                    &name,
                                    start_byte as usize,
                                    true,
                                    true,
                                    &mut facts,
                                );
                                scope.on_word(&name);
                                i += 2;
                                continue;
                            }
                        }
                    }
                    "val" | "var" => {
                        let in_method = scope.open.iter().any(|o| o.collects_body_docs);
                        if in_method {
                            i += 1;
                            continue;
                        }
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
                    "type" => {
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
                            } else if tokens[j].kind == TokenKind::Symbol('[') {
                                let mut depth = 1;
                                let mut k = j + 1;
                                while k < tokens.len() && depth > 0 {
                                    if tokens[k].kind == TokenKind::Symbol('[') {
                                        depth += 1;
                                    } else if tokens[k].kind == TokenKind::Symbol(']') {
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
