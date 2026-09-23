//! Category 3: Functional Languages
//! Parsers for: Haskell, Elixir, OCaml, Scala, Kotlin

use crate::facts::FileFacts;
use crate::lexer::{CommentStyle, Lexer, TokenKind};
use crate::scope::ScopeStack;

pub fn parse_haskell(src: &str) -> FileFacts {
    let mut facts = FileFacts::new();
    let style = CommentStyle {
        line_comment_prefix: &["-- |", "-- ^", "--"],
        doc_comment_prefix: &["-- |", "-- ^", "--"],
        block_comment_start: Some("{-"),
        block_comment_end: Some("-}"),
        ident_suffix_marks: false,
        ident_dashes: false,
        raw_escapes: false,
    };
    let mut lexer = Lexer::new(src, style);
    let tokens = lexer.collect_all_tokens();

    let mut i = 0;
    let mut scope = ScopeStack::new();
    let mut in_equation = false;
    let mut expect_callee = false;
    let mut paren_expect_callee: Vec<bool> = Vec::new();

    while i < tokens.len() {
        let tok = &tokens[i];
        match &tok.kind {
            TokenKind::DocComment(text) | TokenKind::LineComment(text) | TokenKind::BlockComment(text) => {
                scope.push_comment(text);
                i += 1;
                continue;
            }
            TokenKind::Newline => {
                in_equation = false;
                expect_callee = false;
                paren_expect_callee.clear();
                i += 1;
                continue;
            }
            TokenKind::Symbol('=') => {
                in_equation = true;
                expect_callee = true;
                i += 1;
                continue;
            }
            TokenKind::Symbol('(' | '[' | '{') => {
                paren_expect_callee.push(expect_callee);
                expect_callee = true;
                i += 1;
                continue;
            }
            TokenKind::Symbol(')' | ']' | '}') => {
                let _ = paren_expect_callee.pop();
                expect_callee = false;
                i += 1;
                continue;
            }
            TokenKind::Symbol('.') => {
                scope.on_receiver();
                i += 1;
                continue;
            }
            TokenKind::Symbol('$') => {
                expect_callee = true;
                i += 1;
                continue;
            }
            TokenKind::Symbol(c) if matches!(*c, '+' | '-' | '*' | '/' | '|' | ',' | ';') => {
                expect_callee = true;
                i += 1;
                continue;
            }
            TokenKind::DoubleSymbol(s) if matches!(*s, "++" | "==" | "/=" | "<=" | ">=" | "||" | "&&" | ">>" | "->") => {
                expect_callee = true;
                i += 1;
                continue;
            }
            TokenKind::Ident(ident) => {
                let is_keyword = matches!(
                    *ident,
                    "module" | "import" | "data" | "type" | "newtype" | "class" | "instance"
                        | "deriving" | "where" | "let" | "in" | "do" | "case" | "of" | "if"
                        | "then" | "else" | "default" | "foreign"
                );

                if !in_equation {
                    match *ident {
                        "module" => {
                            let start_byte = tok.start;
                            if i + 1 < tokens.len() {
                                if let TokenKind::Ident(mod_name) = tokens[i + 1].kind {
                                    scope.close_definitions_at_or_above(scope.depth, start_byte as usize, &mut facts);
                                    scope.open_definition(mod_name, start_byte as usize, false, &mut facts);
                                    scope.on_word(mod_name);
                                    i += 2;
                                    continue;
                                }
                            }
                        }
                        "data" | "newtype" | "type" | "class" | "instance" => {
                            let start_byte = tok.start;
                            if i + 1 < tokens.len() {
                                if let TokenKind::Ident(type_name) = tokens[i + 1].kind {
                                    scope.close_definitions_at_or_above(scope.depth, start_byte as usize, &mut facts);
                                    scope.open_definition_with_body_docs(type_name, start_byte as usize, true, false, &mut facts);
                                    scope.on_word(type_name);
                                    let mut j = i + 2;
                                    while j < tokens.len() && tokens[j].kind != TokenKind::Newline && tokens[j].kind != TokenKind::Symbol('=') {
                                        j += 1;
                                    }
                                    i = j;
                                    continue;
                                }
                            }
                        }
                        _ => {
                            let start_byte = tok.start;
                            if i + 1 < tokens.len() && tokens[i + 1].kind == TokenKind::DoubleSymbol("::") {
                                if !facts.defines.contains(&ident.to_string()) {
                                    scope.close_definitions_at_or_above(scope.depth, start_byte as usize, &mut facts);
                                    scope.open_definition(*ident, start_byte as usize, true, &mut facts);
                                } else {
                                    scope.enclosing.push((ident.to_string(), scope.depth));
                                }
                                scope.on_word(ident);
                                i += 2;
                                continue;
                            }

                            let mut j = i + 1;
                            while j < tokens.len() && tokens[j].kind != TokenKind::Newline && tokens[j].kind != TokenKind::Symbol('=') {
                                j += 1;
                            }
                            if j < tokens.len() && tokens[j].kind == TokenKind::Symbol('=') && !is_keyword {
                                if !facts.defines.contains(&ident.to_string()) {
                                    scope.close_definitions_at_or_above(scope.depth, start_byte as usize, &mut facts);
                                    scope.open_definition(*ident, start_byte as usize, true, &mut facts);
                                } else {
                                    scope.enclosing.push((ident.to_string(), scope.depth));
                                }
                                scope.on_word(ident);
                                in_equation = true;
                                expect_callee = true;
                                i = j + 1;
                                continue;
                            }
                        }
                    }
                } else if !is_keyword {
                    if expect_callee {
                        let j = i + 1;
                        let is_call = if j < tokens.len() {
                            matches!(
                                tokens[j].kind,
                                TokenKind::Ident(_)
                                    | TokenKind::Symbol('(' | '[' | '{' | '$')
                                    | TokenKind::StringLit(_)
                                    | TokenKind::Number(_)
                            )
                        } else {
                            false
                        };

                        if is_call {
                            scope.record_call(ident, &mut facts);
                        }
                        expect_callee = false;
                    }
                    scope.on_word(ident);
                } else if matches!(*ident, "then" | "else" | "do" | "in" | "where" | "return") {
                    expect_callee = true;
                    scope.on_word(ident);
                }
            }
            TokenKind::Number(num) => {
                expect_callee = false;
                scope.on_word(num);
            }
            TokenKind::StringLit(_) => {
                expect_callee = false;
            }
            _ => {
                scope.had_receiver = false;
            }
        }
        i += 1;
    }

    scope.finish(src.len(), &mut facts);
    facts
}

pub fn parse_elixir(src: &str) -> FileFacts {
    let mut facts = FileFacts::new();
    let style = CommentStyle {
        line_comment_prefix: &["#"],
        doc_comment_prefix: &["#"],
        block_comment_start: None,
        block_comment_end: None,
        ident_suffix_marks: false,
        ident_dashes: false,
        raw_escapes: false,
    };
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
            TokenKind::Symbol('.') => {
                scope.on_receiver();
                i += 1;
                continue;
            }
            TokenKind::Symbol('@') => {
                i += 1;
                if i < tokens.len() {
                    if let TokenKind::Ident(attr) = tokens[i].kind {
                        if matches!(attr, "doc" | "moduledoc" | "typedoc") {
                            let mut j = i + 1;
                            while j < tokens.len() && tokens[j].kind != TokenKind::Newline {
                                if let TokenKind::StringLit(s) = tokens[j].kind {
                                    scope.push_comment(s);
                                    break;
                                }
                                j += 1;
                            }
                            i = j;
                            continue;
                        }
                    }
                    // `@max_retries 3` is how Elixir writes a module
                    // constant. Only at module level and only with a value —
                    // `@behaviour Foo` names a module, not a tunable.
                    if let TokenKind::Ident(attr) = tokens[i].kind {
                        let has_value = i + 1 < tokens.len()
                            && matches!(
                                tokens[i + 1].kind,
                                TokenKind::Number(_) | TokenKind::StringLit(_)
                            );
                        if has_value {
                            scope.open_statement_definition(attr, tok.start as usize, &mut facts);
                            scope.on_word(attr);
                            while i < tokens.len() && tokens[i].kind != TokenKind::Newline {
                                i += 1;
                            }
                            continue;
                        }
                    }
                }
                while i < tokens.len() && tokens[i].kind != TokenKind::Newline {
                    i += 1;
                }
                continue;
            }
            TokenKind::Ident(ident) => {
                match *ident {
                    "do" => {
                        scope.on_open_delimiter();
                    }
                    "end" => {
                        scope.on_close_delimiter(tok.end as usize, &mut facts);
                    }
                    "defmodule" | "defprotocol" | "defimpl" => {
                        let start_byte = tok.start;
                        let mut mod_name = String::new();
                        let mut j = i + 1;
                        while j < tokens.len() {
                            if let TokenKind::Ident(part) = tokens[j].kind {
                                mod_name.push_str(part);
                                if j + 1 < tokens.len() && tokens[j + 1].kind == TokenKind::Symbol('.') {
                                    mod_name.push('.');
                                    j += 2;
                                    continue;
                                }
                                j += 1;
                                break;
                            } else {
                                break;
                            }
                        }
                        if !mod_name.is_empty() {
                            scope.open_definition_with_body_docs(&mod_name, start_byte as usize, true, false, &mut facts);
                            scope.on_word(&mod_name);
                            i = j;
                            continue;
                        }
                    }
                    "def" | "defp" | "defmacro" | "defguard" | "defdelegate" => {
                        let start_byte = tok.start;
                        let mut j = i + 1;
                        while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                            j += 1;
                        }
                        if j < tokens.len() {
                            if let TokenKind::Ident(fn_name) = tokens[j].kind {
                                scope.open_definition(fn_name, start_byte as usize, true, &mut facts);
                                scope.on_word(fn_name);
                                i = j + 1;
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
                            && !matches!(*ident, "if" | "unless" | "case" | "cond" | "with" | "fn" | "receive" | "try" | "raise" | "quote" | "unquote")
                        {
                            scope.record_call(ident, &mut facts);
                        }
                        scope.on_word(ident);
                    }
                }
            }
            _ => {
                if !matches!(tok.kind, TokenKind::StringLit(_) | TokenKind::Number(_)) {
                    scope.had_receiver = false;
                }
            }
        }
        i += 1;
    }

    scope.finish(src.len(), &mut facts);
    facts
}

pub fn parse_ocaml(src: &str) -> FileFacts {
    let mut facts = FileFacts::new();
    let style = CommentStyle {
        line_comment_prefix: &[],
        doc_comment_prefix: &[],
        block_comment_start: Some("(*"),
        block_comment_end: Some("*)"),
        ident_suffix_marks: false,
        ident_dashes: false,
        raw_escapes: false,
    };
    let mut lexer = Lexer::new(src, style);
    let tokens = lexer.collect_all_tokens();

    let mut i = 0;
    let mut scope = ScopeStack::new();
    let mut in_let_body = false;
    let mut expect_callee = false;
    let mut paren_expect_callee: Vec<bool> = Vec::new();

    // Helper to check if a `let` token at index `pos` is a top-level (or module-level) definition,
    // i.e., it does NOT have a corresponding `in` before the next top-level item/end.
    let is_top_level_let = |pos: usize| -> bool {
        let mut depth = 0;
        let mut k = pos + 1;
        while k < tokens.len() {
            match &tokens[k].kind {
                TokenKind::Symbol('(' | '[' | '{') => depth += 1,
                TokenKind::Symbol(')' | ']' | '}') => {
                    if depth > 0 {
                        depth -= 1;
                    }
                }
                TokenKind::Ident("struct" | "sig" | "begin") => depth += 1,
                TokenKind::Ident("end") => {
                    if depth > 0 {
                        depth -= 1;
                    } else {
                        return true;
                    }
                }
                TokenKind::Ident("in") if depth == 0 => {
                    return false;
                }
                TokenKind::Ident("let" | "type" | "module" | "val" | "exception") if depth == 0 => {
                    return true;
                }
                TokenKind::DoubleSymbol(";;") if depth == 0 => {
                    return true;
                }
                _ => {}
            }
            k += 1;
        }
        true
    };

    while i < tokens.len() {
        let tok = &tokens[i];
        match &tok.kind {
            TokenKind::DocComment(text) | TokenKind::BlockComment(text) => {
                scope.push_comment(text);
                i += 1;
                continue;
            }
            TokenKind::Newline => {
                i += 1;
                continue;
            }
            TokenKind::Symbol('=') => {
                in_let_body = true;
                expect_callee = true;
                i += 1;
                continue;
            }
            TokenKind::Symbol('(' | '[' | '{') => {
                paren_expect_callee.push(expect_callee);
                expect_callee = true;
                i += 1;
                continue;
            }
            TokenKind::Symbol(')' | ']' | '}') => {
                let _ = paren_expect_callee.pop();
                expect_callee = false;
                i += 1;
                continue;
            }
            TokenKind::Symbol('.') => {
                scope.on_receiver();
                expect_callee = true;
                i += 1;
                continue;
            }
            TokenKind::DoubleSymbol("|>") => {
                expect_callee = true;
                scope.had_receiver = false;
                i += 1;
                continue;
            }
            TokenKind::Symbol(c) if matches!(*c, '+' | '-' | '*' | '/' | '^' | '@' | ',' | ';') => {
                expect_callee = true;
                i += 1;
                continue;
            }
            TokenKind::DoubleSymbol(s) if matches!(*s, "==" | "!=" | "<=" | ">=" | "||" | "&&" | "::" | "->" | ":=") => {
                expect_callee = true;
                i += 1;
                continue;
            }
            TokenKind::Ident(ident) => {
                let is_keyword = matches!(
                    *ident,
                    "let" | "rec" | "in" | "and" | "type" | "match" | "with" | "fun" | "function"
                        | "if" | "then" | "else" | "module" | "open" | "struct" | "sig" | "end"
                        | "val" | "exception" | "for" | "to" | "do" | "done" | "while" | "begin" | "try" | "as"
                );

                match *ident {
                    "let" | "val" => {
                        let start_byte = tok.start;
                        let mut j = i + 1;
                        if j < tokens.len() && tokens[j].kind == TokenKind::Ident("rec") {
                            j += 1;
                        }
                        if j < tokens.len() {
                            if let TokenKind::Ident(name) = tokens[j].kind {
                                if is_top_level_let(i) {
                                    scope.close_definitions_at_or_above(scope.depth, start_byte as usize, &mut facts);
                                    scope.open_definition_with_body_docs(name, start_byte as usize, true, true, &mut facts);
                                    scope.on_word(name);
                                    in_let_body = false;
                                    expect_callee = false;
                                    // Advance to '=' if present
                                    let mut k = j + 1;
                                    while k < tokens.len() && tokens[k].kind != TokenKind::Symbol('=') && tokens[k].kind != TokenKind::Newline {
                                        k += 1;
                                    }
                                    if k < tokens.len() && tokens[k].kind == TokenKind::Symbol('=') {
                                        in_let_body = true;
                                        expect_callee = true;
                                        i = k + 1;
                                        continue;
                                    }
                                    i = j + 1;
                                    continue;
                                } else {
                                    // Local let binding: let x = ...
                                    let mut k = j + 1;
                                    while k < tokens.len() && tokens[k].kind != TokenKind::Symbol('=') && tokens[k].kind != TokenKind::Newline {
                                        k += 1;
                                    }
                                    if k < tokens.len() && tokens[k].kind == TokenKind::Symbol('=') {
                                        expect_callee = true;
                                        i = k + 1;
                                        continue;
                                    }
                                }
                            }
                        }
                    }
                    "type" | "exception" => {
                        let start_byte = tok.start;
                        if i + 1 < tokens.len() {
                            if let TokenKind::Ident(name) = tokens[i + 1].kind {
                                scope.close_definitions_at_or_above(scope.depth, start_byte as usize, &mut facts);
                                scope.open_definition_with_body_docs(name, start_byte as usize, false, false, &mut facts);
                                scope.on_word(name);
                                in_let_body = false;
                                expect_callee = false;
                                i += 2;
                                continue;
                            }
                        }
                    }
                    "module" => {
                        let start_byte = tok.start;
                        let mut j = i + 1;
                        if j < tokens.len() && tokens[j].kind == TokenKind::Ident("type") {
                            j += 1;
                        }
                        if j < tokens.len() {
                            if let TokenKind::Ident(name) = tokens[j].kind {
                                scope.close_definitions_at_or_above(scope.depth, start_byte as usize, &mut facts);
                                scope.open_definition_with_body_docs(name, start_byte as usize, true, false, &mut facts);
                                scope.on_word(name);
                                in_let_body = false;
                                expect_callee = false;
                                i = j + 1;
                                continue;
                            }
                        }
                    }
                    "struct" | "sig" | "begin" => {
                        scope.on_open_delimiter();
                    }
                    "end" => {
                        scope.on_close_delimiter(tok.end as usize, &mut facts);
                    }
                    _ => {
                        if in_let_body && !is_keyword {
                            if scope.had_receiver {
                                scope.record_call(ident, &mut facts);
                                expect_callee = false;
                            } else if expect_callee {
                                let j = i + 1;
                                let is_call = if j < tokens.len() {
                                    matches!(
                                        tokens[j].kind,
                                        TokenKind::Ident(_)
                                            | TokenKind::Symbol('(' | '[' | '{' | '~' | '?')
                                            | TokenKind::StringLit(_)
                                            | TokenKind::Number(_)
                                    )
                                } else {
                                    false
                                };

                                if is_call {
                                    scope.record_call(ident, &mut facts);
                                }
                                expect_callee = false;
                            }
                            scope.on_word(ident);
                        } else if matches!(*ident, "in" | "then" | "else" | "do" | "match" | "with" | "try" | "function" | "fun") {
                            expect_callee = true;
                            scope.on_word(ident);
                        } else {
                            scope.on_word(ident);
                        }
                    }
                }
            }
            TokenKind::Number(num) => {
                expect_callee = false;
                scope.on_word(num);
            }
            TokenKind::StringLit(_) => {
                expect_callee = false;
            }
            _ => {
                if !matches!(tok.kind, TokenKind::StringLit(_) | TokenKind::Number(_)) {
                    scope.had_receiver = false;
                }
            }
        }
        i += 1;
    }

    scope.finish(src.len(), &mut facts);
    facts
}

pub fn parse_scala(src: &str) -> FileFacts {
    let mut facts = FileFacts::new();
    let style = CommentStyle {
        line_comment_prefix: &["///", "//"],
        doc_comment_prefix: &["/**", "///"],
        block_comment_start: Some("/*"),
        block_comment_end: Some("*/"),
        ident_suffix_marks: false,
        ident_dashes: false,
        raw_escapes: false,
    };
    let mut lexer = Lexer::new(src, style);
    let tokens = lexer.collect_all_tokens();

    let mut i = 0;
    let mut scope = ScopeStack::new();
    let mut in_type = false;

    while i < tokens.len() {
        let tok = &tokens[i];
        match &tok.kind {
            TokenKind::DocComment(text) | TokenKind::LineComment(text) | TokenKind::BlockComment(text) => {
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
                        while j < tokens.len() && tokens[j].kind != TokenKind::Symbol(';') && tokens[j].kind != TokenKind::Symbol('{') && tokens[j].kind != TokenKind::Newline {
                            if let TokenKind::Ident(part) = tokens[j].kind {
                                pkg_name.push_str(part);
                            } else if let TokenKind::Symbol('.') = tokens[j].kind {
                                pkg_name.push('.');
                            }
                            j += 1;
                        }
                        if !pkg_name.is_empty() {
                            scope.open_definition_with_body_docs(&pkg_name, start_byte as usize, false, false, &mut facts);
                            scope.on_word(&pkg_name);
                            i = j;
                            continue;
                        }
                    }
                    "class" | "trait" | "object" | "enum" => {
                        let start_byte = tok.start;
                        if i + 1 < tokens.len() {
                            if let TokenKind::Ident(name) = tokens[i + 1].kind {
                                scope.open_definition_with_body_docs(name, start_byte as usize, true, false, &mut facts);
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
                                scope.open_definition_with_body_docs(&name, start_byte as usize, true, true, &mut facts);
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
                                scope.open_definition_with_body_docs(name, start_byte as usize, false, false, &mut facts);
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
                                scope.open_definition_with_body_docs(name, start_byte as usize, false, false, &mut facts);
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
                            if matches!(tokens[j].kind, TokenKind::Symbol('(') | TokenKind::Symbol('{')) {
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
                                if k < tokens.len() && (tokens[k].kind == TokenKind::Symbol('(') || tokens[k].kind == TokenKind::Symbol('{')) {
                                    is_call = true;
                                }
                            }
                        }

                        if is_call
                            && !matches!(
                                *ident,
                                "if" | "else"
                                    | "while"
                                    | "for"
                                    | "do"
                                    | "yield"
                                    | "match"
                                    | "case"
                                    | "try"
                                    | "catch"
                                    | "finally"
                                    | "throw"
                                    | "return"
                                    | "new"
                                    | "this"
                                    | "super"
                                    | "class"
                                    | "trait"
                                    | "object"
                                    | "enum"
                                    | "def"
                                    | "val"
                                    | "var"
                                    | "type"
                                    | "given"
                                    | "using"
                                    | "package"
                                    | "import"
                                    | "extends"
                                    | "with"
                                    | "derives"
                                    | "as"
                            )
                        {
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

pub fn parse_kotlin(src: &str) -> FileFacts {
    let mut facts = FileFacts::new();
    let style = CommentStyle {
        line_comment_prefix: &["///", "//"],
        doc_comment_prefix: &["/**", "///"],
        block_comment_start: Some("/*"),
        block_comment_end: Some("*/"),
        ident_suffix_marks: false,
        ident_dashes: false,
        raw_escapes: false,
    };
    let mut lexer = Lexer::new(src, style);
    let tokens = lexer.collect_all_tokens();

    let mut i = 0;
    let mut scope = ScopeStack::new();
    let mut in_type = false;

    while i < tokens.len() {
        let tok = &tokens[i];
        match &tok.kind {
            TokenKind::DocComment(text) | TokenKind::LineComment(text) | TokenKind::BlockComment(text) => {
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
            TokenKind::Symbol('.') | TokenKind::DoubleSymbol("?.") | TokenKind::DoubleSymbol("::") => {
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
                        while j < tokens.len() && tokens[j].kind != TokenKind::Symbol(';') && tokens[j].kind != TokenKind::Newline {
                            if let TokenKind::Ident(part) = tokens[j].kind {
                                pkg_name.push_str(part);
                            } else if let TokenKind::Symbol('.') = tokens[j].kind {
                                pkg_name.push('.');
                            }
                            j += 1;
                        }
                        if !pkg_name.is_empty() {
                            scope.open_definition_with_body_docs(&pkg_name, start_byte as usize, false, false, &mut facts);
                            scope.on_word(&pkg_name);
                            i = j;
                            continue;
                        }
                    }
                    "class" | "interface" | "object" => {
                        let start_byte = tok.start;
                        if i + 1 < tokens.len() {
                            if let TokenKind::Ident(name) = tokens[i + 1].kind {
                                scope.open_definition_with_body_docs(name, start_byte as usize, true, false, &mut facts);
                                scope.on_word(name);
                                i += 2;
                                continue;
                            }
                        }
                    }
                    "companion" => {
                        // companion object [Name]
                        let start_byte = tok.start;
                        if i + 1 < tokens.len() && matches!(tokens[i + 1].kind, TokenKind::Ident("object")) {
                            if i + 2 < tokens.len() {
                                if let TokenKind::Ident(name) = tokens[i + 2].kind {
                                    scope.open_definition_with_body_docs(name, start_byte as usize, true, false, &mut facts);
                                    scope.on_word(name);
                                    i += 3;
                                    continue;
                                }
                            }
                            scope.open_definition_with_body_docs("Companion", start_byte as usize, true, false, &mut facts);
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
                                if j + 1 < tokens.len() && (tokens[j + 1].kind == TokenKind::Symbol('.') || tokens[j + 1].kind == TokenKind::Symbol('<')) {
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
                                        if j < tokens.len() && tokens[j].kind == TokenKind::Symbol('.') {
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
                            scope.open_definition_with_body_docs(fn_name, start_byte as usize, true, true, &mut facts);
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
                                if j + 1 < tokens.len() && tokens[j].kind == TokenKind::Symbol('.') {
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
                            scope.open_definition_with_body_docs(prop_name, start_byte as usize, false, false, &mut facts);
                            scope.on_word(prop_name);
                            i = j;
                            continue;
                        }
                    }
                    "typealias" => {
                        let start_byte = tok.start;
                        if i + 1 < tokens.len() {
                            if let TokenKind::Ident(name) = tokens[i + 1].kind {
                                scope.open_definition_with_body_docs(name, start_byte as usize, false, false, &mut facts);
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
                            if matches!(tokens[j].kind, TokenKind::Symbol('(') | TokenKind::Symbol('{')) {
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
                                if k < tokens.len() && (tokens[k].kind == TokenKind::Symbol('(') || tokens[k].kind == TokenKind::Symbol('{')) {
                                    is_call = true;
                                }
                            }
                        }

                        if is_call
                            && !matches!(
                                *ident,
                                "if" | "else"
                                    | "when"
                                    | "while"
                                    | "for"
                                    | "do"
                                    | "try"
                                    | "catch"
                                    | "finally"
                                    | "throw"
                                    | "return"
                                    | "this"
                                    | "super"
                                    | "class"
                                    | "interface"
                                    | "object"
                                    | "fun"
                                    | "val"
                                    | "var"
                                    | "typealias"
                                    | "constructor"
                                    | "init"
                                    | "is"
                                    | "as"
                                    | "in"
                                    | "import"
                                    | "package"
                                    | "by"
                                    | "get"
                                    | "set"
                                    | "companion"
                                    | "enum"
                            )
                        {
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

pub fn parse_erlang(src: &str) -> FileFacts {
    let mut facts = FileFacts::new();
    let style = CommentStyle {
        line_comment_prefix: &["%%%", "%%", "%"],
        doc_comment_prefix: &["%%%", "%%"],
        block_comment_start: None,
        block_comment_end: None,
        ident_suffix_marks: false,
        ident_dashes: false,
        raw_escapes: false,
    };
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
            TokenKind::Symbol(':') => {
                scope.on_receiver();
                i += 1;
                continue;
            }
            TokenKind::Symbol('-') => {
                let start_byte = tok.start;
                if i + 1 < tokens.len() {
                    if let TokenKind::Ident(attr) = tokens[i + 1].kind {
                        match attr {
                            "module" => {
                                if i + 3 < tokens.len() && tokens[i + 2].kind == TokenKind::Symbol('(') {
                                    if let TokenKind::Ident(mod_name) = tokens[i + 3].kind {
                                        scope.open_definition_with_body_docs(mod_name, start_byte as usize, false, false, &mut facts);
                                        scope.on_word(mod_name);
                                        i += 4;
                                        continue;
                                    }
                                }
                            }
                            "record" if i + 3 < tokens.len() && tokens[i + 2].kind == TokenKind::Symbol('(') => {
                                if let TokenKind::Ident(rec_name) = tokens[i + 3].kind {
                                    scope.open_definition_with_body_docs(rec_name, start_byte as usize, false, false, &mut facts);
                                    scope.on_word(rec_name);
                                    i += 4;
                                    continue;
                                }
                            }
                            // Every other `-attribute(...)` is a directive,
                            // not a call: `-export([f/1])` and `-define(X, 1)`
                            // were being recorded as calls from `<module>`,
                            // which is what put the fixture at 29%. Skipping
                            // the parenthesised argument keeps a name inside
                            // it from being read as one either.
                            _ if i + 2 < tokens.len()
                                && tokens[i + 2].kind == TokenKind::Symbol('(') =>
                            {
                                let mut depth = 1;
                                i += 3;
                                while i < tokens.len() && depth > 0 {
                                    match tokens[i].kind {
                                        TokenKind::Symbol('(') => depth += 1,
                                        TokenKind::Symbol(')') => depth -= 1,
                                        _ => {}
                                    }
                                    i += 1;
                                }
                                continue;
                            }
                            _ => {}
                        }
                    }
                }
                i += 1;
                continue;
            }
            TokenKind::Symbol('.') => {
                scope.close_definitions_at_or_above(0, tok.end as usize, &mut facts);
                i += 1;
                continue;
            }
            TokenKind::Ident(ident) => {
                // Top-level function definition: fn_name(Args) -> ...
                if i + 1 < tokens.len() && tokens[i + 1].kind == TokenKind::Symbol('(') {
                    let mut k = i + 2;
                    let mut depth = 1;
                    while k < tokens.len() && depth > 0 {
                        if tokens[k].kind == TokenKind::Symbol('(') {
                            depth += 1;
                        } else if tokens[k].kind == TokenKind::Symbol(')') {
                            depth -= 1;
                        }
                        k += 1;
                    }
                    while k < tokens.len() && tokens[k].kind == TokenKind::Newline {
                        k += 1;
                    }
                    if k < tokens.len() && (tokens[k].kind == TokenKind::DoubleSymbol("->") || matches!(tokens[k].kind, TokenKind::Ident("when"))) && scope.open.is_empty() {
                        let start_byte = tok.start;
                        scope.open_definition_with_body_docs(*ident, start_byte as usize, true, true, &mut facts);
                        scope.on_word(ident);
                        i += 1;
                        continue;
                    }

                    if !matches!(*ident, "if" | "case" | "receive" | "try" | "catch" | "after" | "begin" | "end" | "fun") {
                        scope.record_call(ident, &mut facts);
                    }
                }
                scope.on_word(ident);
            }
            _ => {
                if !matches!(tok.kind, TokenKind::StringLit(_) | TokenKind::Number(_)) {
                    scope.had_receiver = false;
                }
            }
        }
        i += 1;
    }

    scope.finish(src.len(), &mut facts);
    facts
}

pub fn parse_fsharp(src: &str) -> FileFacts {
    let mut facts = FileFacts::new();
    let style = CommentStyle {
        line_comment_prefix: &["///", "//"],
        doc_comment_prefix: &["///"],
        block_comment_start: Some("(*"),
        block_comment_end: Some("*)"),
        ident_suffix_marks: false,
        ident_dashes: false,
        raw_escapes: false,
    };
    let mut lexer = Lexer::new(src, style);
    let tokens = lexer.collect_all_tokens();

    let mut i = 0;
    let mut scope = ScopeStack::new();

    while i < tokens.len() {
        let tok = &tokens[i];
        match &tok.kind {
            TokenKind::DocComment(text) | TokenKind::LineComment(text) | TokenKind::BlockComment(text) => {
                scope.push_comment(text);
                i += 1;
                continue;
            }
            TokenKind::Newline => {
                i += 1;
                continue;
            }
            TokenKind::Symbol('.') => {
                scope.on_receiver();
                i += 1;
                continue;
            }
            TokenKind::Ident(ident) => {
                match *ident {
                    "module" | "namespace" => {
                        let start_byte = tok.start;
                        let mut j = i + 1;
                        let mut mod_name = String::new();
                        while j < tokens.len() && tokens[j].kind != TokenKind::Newline && tokens[j].kind != TokenKind::Symbol('=') {
                            if let TokenKind::Ident(part) = tokens[j].kind {
                                mod_name.push_str(part);
                            } else if let TokenKind::Symbol('.') = tokens[j].kind {
                                mod_name.push('.');
                            }
                            j += 1;
                        }
                        if !mod_name.is_empty() {
                            scope.close_definitions_at_or_above(0, start_byte as usize, &mut facts);
                            scope.open_definition_with_body_docs(&mod_name, start_byte as usize, false, false, &mut facts);
                            scope.on_word(&mod_name);
                            i = j;
                            continue;
                        }
                    }
                    "type" => {
                        let start_byte = tok.start;
                        if i + 1 < tokens.len() {
                            if let TokenKind::Ident(name) = tokens[i + 1].kind {
                                scope.close_definitions_at_or_above(1, start_byte as usize, &mut facts);
                                scope.open_definition_with_body_docs(name, start_byte as usize, false, false, &mut facts);
                                scope.on_word(name);
                                i += 2;
                                continue;
                            }
                        }
                    }
                    // **`override` and `default` declare a member too**, which
                    // is how F# implements an interface — `override
                    // this.SayHello(…) = …`. The corpus is what showed it: a
                    // file whose whole content is interface implementations
                    // measured 42% on `<module>`, because every one of them
                    // was invisible and its body belonged to the file. The
                    // fixture writes only `let`, so it could not have found
                    // this.
                    "let" | "member" | "override" | "default" | "abstract" => {
                        let start_byte = tok.start;
                        let mut j = i + 1;
                        if j < tokens.len() && matches!(tokens[j].kind, TokenKind::Ident("rec" | "inline" | "static" | "public" | "private" | "internal")) {
                            j += 1;
                        }
                        // `override this.SayHello` names the member after the
                        // receiver, not before it.
                        if tokens.get(j + 1).map(|t| &t.kind) == Some(&TokenKind::Symbol('.')) {
                            j += 2;
                        }
                        if j < tokens.len() {
                            if let TokenKind::Ident(name) = tokens[j].kind {
                                scope.close_definitions_at_or_above(1, start_byte as usize, &mut facts);
                                scope.open_definition_with_body_docs(name, start_byte as usize, true, true, &mut facts);
                                scope.on_word(name);
                                i = j + 1;
                                continue;
                            }
                        }
                    }
                    _ => {
                        let mut j = i + 1;
                        while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                            j += 1;
                        }
                        // An ML applies a function by juxtaposition: `refuse
                        // owner`, no parentheses anywhere. Testing only for
                        // `(` measured 625 definitions and **zero** edges on a
                        // realistic module — the language looked extracted and
                        // carried no call at all.
                        let applied = tokens.get(j).is_some_and(|t| {
                            matches!(
                                &t.kind,
                                TokenKind::Symbol('(')
                                    | TokenKind::Ident(_)
                                    | TokenKind::StringLit(_)
                                    | TokenKind::Number(_)
                            )
                        });
                        let is_call = applied;
                        if is_call
                            && !matches!(
                                *ident,
                                "if" | "then"
                                    | "else"
                                    | "elif"
                                    | "match"
                                    | "with"
                                    | "try"
                                    | "catch"
                                    | "finally"
                                    | "for"
                                    | "to"
                                    | "while"
                                    | "do"
                                    | "done"
                                    | "in"
                                    | "open"
                                    | "type"
                                    | "let"
                                    | "member"
                                    | "override"
                                    | "default"
                                    | "abstract"
                            )
                        {
                            scope.record_call(ident, &mut facts);
                        } else if !is_call {
                            scope.had_receiver = false;
                        }
                        scope.on_word(ident);
                    }
                }
            }
            _ => {
                if !matches!(tok.kind, TokenKind::StringLit(_) | TokenKind::Number(_)) {
                    scope.had_receiver = false;
                }
            }
        }
        i += 1;
    }

    scope.finish(src.len(), &mut facts);
    facts
}

fn get_lisp_ident<'a>(tokens: &[crate::lexer::Token<'a>], idx: &mut usize, src: &'a str) -> Option<&'a str> {
    while *idx < tokens.len() && tokens[*idx].kind == TokenKind::Newline {
        *idx += 1;
    }
    if *idx >= tokens.len() {
        return None;
    }
    if matches!(tokens[*idx].kind, TokenKind::Ident(_)) {
        let start = tokens[*idx].start as usize;
        let mut end = tokens[*idx].end as usize;
        *idx += 1;
        while *idx < tokens.len() {
            match tokens[*idx].kind {
                TokenKind::Symbol('-') | TokenKind::Symbol('.') | TokenKind::Symbol('/') | TokenKind::Symbol('_') => {
                    if *idx + 1 < tokens.len() && matches!(tokens[*idx + 1].kind, TokenKind::Ident(_)) {
                        end = tokens[*idx + 1].end as usize;
                        *idx += 2;
                    } else {
                        break;
                    }
                }
                _ => break,
            }
        }
        Some(&src[start..end])
    } else {
        None
    }
}

pub fn parse_clojure(src: &str) -> FileFacts {
    let mut facts = FileFacts::new();
    let style = CommentStyle {
        line_comment_prefix: &[";;;", ";;", ";"],
        doc_comment_prefix: &[";;;", ";;"],
        block_comment_start: None,
        block_comment_end: None,
        ident_suffix_marks: false,
        ident_dashes: true,
        raw_escapes: false,
    };
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
            TokenKind::Symbol('(') => {
                scope.on_open_delimiter();
                let mut next_idx = i + 1;
                if let Some(form) = get_lisp_ident(&tokens, &mut next_idx, src) {
                    match form {
                        // Counted in Clojure's own library rather than
                        // guessed: `defmethod` 142, `defonce` 12, `definline`
                        // 8, `defstruct` 5 in `src/`, and `deftest` 672 with
                        // `defspec` 36 in `test/`. Missing `defmethod` put 85
                        // calls on `<module>` and `deftest` 382 — every
                        // multimethod implementation's and every test's body.
                        // `defcurried` and `def-aset` appear eight times each
                        // and are this project's own macros, not the language.
                        "defn" | "defn-" | "defmacro" | "defmethod" | "defmulti" | "defprotocol"
                        | "defrecord" | "deftype" | "defonce" | "definline" | "defstruct"
                        | "deftest" | "defspec" | "def" => {
                            let start_byte = tok.start;
                            if let Some(name) = get_lisp_ident(&tokens, &mut next_idx, src) {
                                let is_fn = matches!(
                                    form,
                                    "defn"
                                        | "defn-"
                                        | "defmacro"
                                        | "defmethod"
                                        | "definline"
                                        | "deftest"
                                );
                                scope.open_definition_with_body_docs(name, start_byte as usize, true, is_fn, &mut facts);
                                if let Some(last) = scope.open.last_mut() {
                                    last.depth = scope.depth - 1;
                                }
                                if let Some(last) = scope.enclosing.last_mut() {
                                    last.1 = scope.depth - 1;
                                }
                                scope.on_word(name);
                                i = next_idx;
                                continue;
                            }
                        }
                        "ns" => {
                            let start_byte = tok.start;
                            if let Some(name) = get_lisp_ident(&tokens, &mut next_idx, src) {
                                scope.open_definition_with_body_docs(name, start_byte as usize, true, false, &mut facts);
                                if let Some(last) = scope.open.last_mut() {
                                    last.depth = scope.depth - 1;
                                }
                                if let Some(last) = scope.enclosing.last_mut() {
                                    last.1 = scope.depth - 1;
                                }
                                scope.on_word(name);
                                i = next_idx;
                                continue;
                            }
                        }
                        _ => {
                            if !matches!(form, "if" | "when" | "let" | "loop" | "recur" | "do" | "fn" | "cond" | "case" | "quote") {
                                scope.record_call(form, &mut facts);
                            }
                        }
                    }
                }
                i += 1;
                continue;
            }
            TokenKind::Symbol(')') => {
                scope.on_close_delimiter(tok.end as usize, &mut facts);
                i += 1;
                continue;
            }
            TokenKind::Ident(ident) => {
                scope.on_word(ident);
            }
            _ => {}
        }
        i += 1;
    }

    scope.finish(src.len(), &mut facts);
    facts
}

pub fn parse_elm(src: &str) -> FileFacts {
    let mut facts = FileFacts::new();
    let style = CommentStyle {
        line_comment_prefix: &["--"],
        doc_comment_prefix: &["--"],
        block_comment_start: Some("{-"),
        block_comment_end: Some("-}"),
        ident_suffix_marks: false,
        ident_dashes: false,
        raw_escapes: false,
    };
    let mut lexer = Lexer::new(src, style);
    let tokens = lexer.collect_all_tokens();

    let mut i = 0;
    let mut scope = ScopeStack::new();

    while i < tokens.len() {
        let tok = &tokens[i];
        match &tok.kind {
            TokenKind::DocComment(text) | TokenKind::LineComment(text) | TokenKind::BlockComment(text) => {
                scope.push_comment(text);
                i += 1;
                continue;
            }
            TokenKind::Newline => {
                i += 1;
                continue;
            }
            TokenKind::Ident(ident) => {
                match *ident {
                    "module" => {
                        let start_byte = tok.start;
                        if i + 1 < tokens.len() {
                            if let TokenKind::Ident(name) = tokens[i + 1].kind {
                                scope.open_definition_with_body_docs(name, start_byte as usize, true, false, &mut facts);
                                scope.on_word(name);
                                i += 2;
                                continue;
                            }
                        }
                    }
                    "type" => {
                        let start_byte = tok.start;
                        let mut j = i + 1;
                        if j < tokens.len() && tokens[j].kind == TokenKind::Ident("alias") {
                            j += 1;
                        }
                        if j < tokens.len() {
                            if let TokenKind::Ident(name) = tokens[j].kind {
                                scope.open_definition_with_body_docs(name, start_byte as usize, true, false, &mut facts);
                                scope.on_word(name);
                                i = j + 1;
                                continue;
                            }
                        }
                    }
                    _ => {
                        let mut j = i + 1;
                        while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                            j += 1;
                        }
                        if j < tokens.len() && tokens[j].kind == TokenKind::Symbol(':') {
                            // Type annotation: `name : Type -> Type`
                            let mut k = j + 1;
                            while k < tokens.len() && tokens[k].kind != TokenKind::Newline {
                                k += 1;
                            }
                            let start_byte = tok.start;
                            scope.close_definitions_at_or_above(0, start_byte as usize, &mut facts);
                            scope.open_definition_with_body_docs(*ident, start_byte as usize, true, true, &mut facts);
                            scope.on_word(ident);
                            i = k;
                            continue;
                        } else if j < tokens.len() && tokens[j].kind == TokenKind::Symbol('=') && !scope.open.iter().any(|o| o.name == *ident) {
                            // Function definition without annotation: `name args = ...`
                            let start_byte = tok.start;
                            scope.close_definitions_at_or_above(0, start_byte as usize, &mut facts);
                            scope.open_definition_with_body_docs(*ident, start_byte as usize, true, true, &mut facts);
                            scope.on_word(ident);
                            i = j;
                            continue;
                        } else {
                            if !matches!(*ident, "if" | "then" | "else" | "case" | "of" | "let" | "in" | "exposing" | "as" | "import" | "module" | "type" | "port") {
                                scope.record_call(ident, &mut facts);
                            }
                            scope.on_word(ident);
                        }
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

pub fn parse_gleam(src: &str) -> FileFacts {
    let mut facts = FileFacts::new();
    let style = CommentStyle {
        line_comment_prefix: &["///", "//"],
        doc_comment_prefix: &["///"],
        block_comment_start: None,
        block_comment_end: None,
        ident_suffix_marks: false,
        ident_dashes: false,
        raw_escapes: false,
    };
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
            TokenKind::Symbol('.') => {
                scope.on_receiver();
                i += 1;
                continue;
            }
            TokenKind::Ident("pub") => {
                i += 1;
                continue;
            }
            TokenKind::Ident(ident) => {
                match *ident {
                    "fn" => {
                        let start_byte = tok.start;
                        let mut j = i + 1;
                        while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                            j += 1;
                        }
                        if j < tokens.len() {
                            if let TokenKind::Ident(fn_name) = tokens[j].kind {
                                scope.open_definition_with_body_docs(fn_name, start_byte as usize, true, true, &mut facts);
                                scope.on_word(fn_name);
                                i = j + 1;
                                continue;
                            }
                        }
                    }
                    "type" => {
                        let start_byte = tok.start;
                        let mut j = i + 1;
                        while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                            j += 1;
                        }
                        if j < tokens.len() {
                            if let TokenKind::Ident(name) = tokens[j].kind {
                                scope.open_definition_with_body_docs(name, start_byte as usize, true, false, &mut facts);
                                scope.on_word(name);
                                i = j + 1;
                                continue;
                            }
                        }
                    }
                    _ => {
                        let mut j = i + 1;
                        while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                            j += 1;
                        }
                        let is_call = j < tokens.len() && tokens[j].kind == TokenKind::Symbol('(');
                        if is_call
                            && !matches!(*ident, "if" | "else" | "case" | "let" | "panic" | "todo" | "assert" | "import" | "pub" | "type" | "fn")
                        {
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

pub fn parse_purescript(src: &str) -> FileFacts {
    let mut facts = FileFacts::new();
    let style = CommentStyle {
        line_comment_prefix: &["--"],
        doc_comment_prefix: &["--"],
        block_comment_start: Some("{-"),
        block_comment_end: Some("-}"),
        ident_suffix_marks: false,
        ident_dashes: false,
        raw_escapes: false,
    };
    let mut lexer = Lexer::new(src, style);
    let tokens = lexer.collect_all_tokens();

    let mut i = 0;
    let mut scope = ScopeStack::new();

    while i < tokens.len() {
        let tok = &tokens[i];
        match &tok.kind {
            TokenKind::DocComment(text) | TokenKind::LineComment(text) | TokenKind::BlockComment(text) => {
                scope.push_comment(text);
                i += 1;
                continue;
            }
            TokenKind::Newline => {
                i += 1;
                continue;
            }
            TokenKind::Ident(ident) => {
                match *ident {
                    "module" => {
                        let start_byte = tok.start;
                        let mut j = i + 1;
                        while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                            j += 1;
                        }
                        if j < tokens.len() {
                            let name_start = tokens[j].start;
                            let mut name_end = tokens[j].end;
                            while j < tokens.len() && tokens[j].kind != TokenKind::Ident("where") && tokens[j].kind != TokenKind::Newline {
                                if !matches!(tokens[j].kind, TokenKind::Symbol('(')) {
                                    name_end = tokens[j].end;
                                } else {
                                    break;
                                }
                                j += 1;
                            }
                            let full_name = src[name_start as usize..name_end as usize].trim();
                            scope.open_definition_with_body_docs(full_name, start_byte as usize, true, false, &mut facts);
                            scope.on_word(full_name);
                            while j < tokens.len() && tokens[j].kind != TokenKind::Ident("where") && tokens[j].kind != TokenKind::Newline {
                                j += 1;
                            }
                            i = j;
                            continue;
                        }
                    }
                    "data" | "type" | "newtype" | "class" | "instance" => {
                        let start_byte = tok.start;
                        let mut j = i + 1;
                        while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                            j += 1;
                        }
                        if j < tokens.len() {
                            if let TokenKind::Ident(name) = tokens[j].kind {
                                scope.open_definition_with_body_docs(name, start_byte as usize, true, false, &mut facts);
                                scope.on_word(name);
                                i = j + 1;
                                continue;
                            }
                        }
                    }
                    _ => {
                        let mut j = i + 1;
                        while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                            j += 1;
                        }
                        if j < tokens.len() && tokens[j].kind == TokenKind::DoubleSymbol("::") {
                            let mut k = j + 1;
                            while k < tokens.len() && tokens[k].kind != TokenKind::Newline {
                                k += 1;
                            }
                            let start_byte = tok.start;
                            scope.close_definitions_at_or_above(0, start_byte as usize, &mut facts);
                            scope.open_definition_with_body_docs(*ident, start_byte as usize, true, true, &mut facts);
                            scope.on_word(ident);
                            i = k;
                            continue;
                        } else if j < tokens.len() && tokens[j].kind == TokenKind::Symbol('=') && !scope.open.iter().any(|o| o.name == *ident) {
                            let start_byte = tok.start;
                            scope.close_definitions_at_or_above(0, start_byte as usize, &mut facts);
                            scope.open_definition_with_body_docs(*ident, start_byte as usize, true, true, &mut facts);
                            scope.on_word(ident);
                            i = j;
                            continue;
                        } else {
                            if !matches!(*ident, "if" | "then" | "else" | "case" | "of" | "let" | "in" | "where" | "do" | "ado" | "import" | "module" | "data" | "type" | "newtype" | "class" | "instance") {
                                scope.record_call(ident, &mut facts);
                            }
                            scope.on_word(ident);
                        }
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

pub fn parse_lisp(src: &str) -> FileFacts {
    let mut facts = FileFacts::new();
    let style = CommentStyle {
        line_comment_prefix: &[";;;", ";;", ";"],
        doc_comment_prefix: &[";;;", ";;"],
        block_comment_start: Some("#|"),
        block_comment_end: Some("|#"),
        ident_suffix_marks: false,
        ident_dashes: true,
        raw_escapes: false,
    };
    let mut lexer = Lexer::new(src, style);
    let tokens = lexer.collect_all_tokens();

    let mut i = 0;
    let mut scope = ScopeStack::new();

    while i < tokens.len() {
        let tok = &tokens[i];
        match &tok.kind {
            TokenKind::DocComment(text) | TokenKind::LineComment(text) | TokenKind::BlockComment(text) => {
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
                let mut next_idx = i + 1;
                if let Some(form) = get_lisp_ident(&tokens, &mut next_idx, src) {
                    let lower = form.to_ascii_lowercase();
                    match lower.as_str() {
                        // Counted across the corpus rather than guessed:
                        // `define` 903, `defun` 312, `defvar` 145, `defcustom`
                        // 36, `define-macro` 35, `defconst` 15. The three at
                        // the tail are Emacs Lisp and Scheme rather than
                        // Common Lisp, which is why the original list missed
                        // them.
                        "defun" | "defmacro" | "define-macro" | "defmethod" | "defgeneric"
                        | "defcustom" | "defconst" | "define" => {
                            let start_byte = tok.start;
                            if let Some(name) = get_lisp_ident(&tokens, &mut next_idx, src) {
                                scope.open_definition_with_body_docs(name, start_byte as usize, true, true, &mut facts);
                                if let Some(last) = scope.open.last_mut() {
                                    last.depth = scope.depth - 1;
                                }
                                if let Some(last) = scope.enclosing.last_mut() {
                                    last.1 = scope.depth - 1;
                                }
                                scope.on_word(name);
                                i = next_idx;
                                continue;
                            } else if next_idx < tokens.len() && tokens[next_idx].kind == TokenKind::Symbol('(') {
                                // Scheme (define (fn-name args) ...)
                                //
                                // The parameter list opens a delimiter the
                                // loop never sees, because `i` jumps past it.
                                // Counting it here keeps the depth honest —
                                // without it the list's `)` closed the
                                // definition itself, so every call in the body
                                // was attributed to `<module>`: measured on
                                // Julia's Scheme frontend, 94 of 94 in one
                                // file and 55% across the tree.
                                scope.on_open_delimiter();
                                next_idx += 1;
                                if let Some(name) = get_lisp_ident(&tokens, &mut next_idx, src) {
                                    scope.open_definition_with_body_docs(name, start_byte as usize, true, true, &mut facts);
                                    // Two levels below the current depth: the
                                    // form's own `(` and the parameter list's.
                                    if let Some(last) = scope.open.last_mut() {
                                        last.depth = scope.depth - 2;
                                    }
                                    if let Some(last) = scope.enclosing.last_mut() {
                                        last.1 = scope.depth - 2;
                                    }
                                    scope.on_word(name);
                                    i = next_idx;
                                    continue;
                                }
                            }
                        }
                        "defstruct" | "defclass" | "defparameter" | "defvar" | "defconstant" => {
                            let start_byte = tok.start;
                            if let Some(name) = get_lisp_ident(&tokens, &mut next_idx, src) {
                                scope.open_definition_with_body_docs(name, start_byte as usize, true, false, &mut facts);
                                if let Some(last) = scope.open.last_mut() {
                                    last.depth = scope.depth - 1;
                                }
                                if let Some(last) = scope.enclosing.last_mut() {
                                    last.1 = scope.depth - 1;
                                }
                                scope.on_word(name);
                                i = next_idx;
                                continue;
                            }
                        }
                        _ => {
                            if !matches!(lower.as_str(), "if" | "cond" | "when" | "unless" | "let" | "let*" | "labels" | "flet" | "progn" | "lambda") {
                                scope.record_call(form, &mut facts);
                            }
                        }
                    }
                }
                i += 1;
                continue;
            }
            TokenKind::Symbol(')') => {
                scope.on_close_delimiter(tok.end as usize, &mut facts);
                i += 1;
                continue;
            }
            TokenKind::Ident(ident) => {
                scope.on_word(ident);
            }
            _ => {}
        }
        i += 1;
    }

    scope.finish(src.len(), &mut facts);
    facts
}

pub fn parse_lean(src: &str) -> FileFacts {
    let mut facts = FileFacts::new();
    let style = CommentStyle {
        line_comment_prefix: &["--"],
        doc_comment_prefix: &["--"],
        block_comment_start: Some("/-"),
        block_comment_end: Some("-/"),
        ident_suffix_marks: false,
        ident_dashes: false,
        raw_escapes: false,
    };
    let mut lexer = Lexer::new(src, style);
    let tokens = lexer.collect_all_tokens();

    let mut i = 0;
    let mut scope = ScopeStack::new();

    while i < tokens.len() {
        let tok = &tokens[i];
        match &tok.kind {
            TokenKind::DocComment(text) | TokenKind::LineComment(text) | TokenKind::BlockComment(text) => {
                scope.push_comment(text);
                i += 1;
                continue;
            }
            TokenKind::Newline => {
                i += 1;
                continue;
            }
            TokenKind::Ident(ident) => {
                let lower = ident.to_ascii_lowercase();
                match lower.as_str() {
                    "theorem" | "lemma" | "def" | "definition" | "inductive" | "structure" | "axiom" | "opaque" => {
                        let start_byte = tok.start;
                        if i + 1 < tokens.len() {
                            if let TokenKind::Ident(name) = tokens[i + 1].kind {
                                scope.close_definitions_at_or_above(0, start_byte as usize, &mut facts);
                                let is_fn = matches!(lower.as_str(), "theorem" | "lemma" | "def" | "definition");
                                scope.open_definition_with_body_docs(name, start_byte as usize, true, is_fn, &mut facts);
                                scope.on_word(name);
                                i += 2;
                                continue;
                            }
                        }
                    }
                    _ => {
                        if scope.had_receiver {
                            scope.record_call(ident, &mut facts);
                        }
                        scope.on_word(ident);
                    }
                }
            }
            TokenKind::Symbol('.') => {
                scope.on_receiver();
            }
            _ => {
                if !matches!(tok.kind, TokenKind::StringLit(_) | TokenKind::Number(_)) {
                    scope.had_receiver = false;
                }
            }
        }
        i += 1;
    }

    scope.finish(src.len(), &mut facts);
    facts
}
