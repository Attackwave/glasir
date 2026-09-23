//! Category 2: Modern Systems Languages
//! Parsers for: Zig, Nim, Odin

use crate::facts::FileFacts;
use crate::lexer::{CommentStyle, Lexer, TokenKind};
use crate::scope::ScopeStack;

pub fn parse_zig(src: &str) -> FileFacts {
    let mut facts = FileFacts::new();
    let style = CommentStyle {
        line_comment_prefix: &["///", "//!", "//"],
        doc_comment_prefix: &["///", "//!"],
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
            TokenKind::Ident(ident) => {
                if matches!(*ident, "pub" | "export" | "extern" | "inline" | "noinline") {
                    i += 1;
                    continue;
                }

                match *ident {
                    "fn" => {
                        let start_byte = tok.start;
                        if i + 1 < tokens.len() {
                            if let TokenKind::Ident(fn_name) = tokens[i + 1].kind {
                                scope.open_definition(fn_name, start_byte as usize, true, &mut facts);
                                scope.on_word(fn_name);
                                i += 2;
                                continue;
                            }
                        }
                    }
                    "test" => {
                        let start_byte = tok.start;
                        let test_name = if i + 1 < tokens.len() {
                            if let TokenKind::StringLit(s) = tokens[i + 1].kind {
                                s.trim_matches('"')
                            } else {
                                "test"
                            }
                        } else {
                            "test"
                        };
                        scope.open_definition(test_name, start_byte as usize, true, &mut facts);
                        scope.on_word(test_name);
                        i += 2;
                        continue;
                    }
                    "const" | "var" => {
                        let start_byte = tok.start;
                        if i + 1 < tokens.len() {
                            if let TokenKind::Ident(var_name) = tokens[i + 1].kind {
                                scope.open_definition(var_name, start_byte as usize, false, &mut facts);
                                scope.on_word(var_name);
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
                            && !matches!(*ident, "if" | "while" | "for" | "switch" | "return" | "try" | "catch")
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

pub fn parse_nim(src: &str) -> FileFacts {
    let mut facts = FileFacts::new();
    let style = CommentStyle {
        line_comment_prefix: &["##", "#"],
        doc_comment_prefix: &["##"],
        block_comment_start: Some("#["),
        block_comment_end: Some("]#"),
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
        let line_start_pos = src[..tok.start as usize].rfind('\n').map(|p| p + 1).unwrap_or(0);
        let current_line_indent = (tok.start as usize).saturating_sub(line_start_pos) as i32;

        match &tok.kind {
            TokenKind::DocComment(text) | TokenKind::LineComment(text) | TokenKind::BlockComment(text) => {
                while scope.open.last().is_some_and(|o| o.depth > current_line_indent) {
                    scope.on_close_delimiter(tok.start as usize, &mut facts);
                }
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
                while scope.open.last().is_some_and(|o| o.depth > current_line_indent) {
                    scope.on_close_delimiter(tok.start as usize, &mut facts);
                }

                match *ident {
                    "proc" | "func" | "method" | "iterator" | "template" | "macro" | "converter" => {
                        let start_byte = tok.start;
                        if i + 1 < tokens.len() {
                            if let TokenKind::Ident(raw_name) = tokens[i + 1].kind {
                                let clean_name = raw_name.trim_end_matches('*');
                                scope.open_definition(clean_name, start_byte as usize, true, &mut facts);
                                if let Some(last) = scope.open.last_mut() {
                                    last.depth = current_line_indent + 1;
                                }
                                scope.on_word(clean_name);
                                i += 2;
                                if i < tokens.len() && tokens[i].kind == TokenKind::Symbol('*') {
                                    i += 1;
                                }
                                continue;
                            }
                        }
                    }
                    "type" | "const" | "let" | "var" => {
                        let start_byte = tok.start;
                        // Nim writes a group as `const` alone on its line with
                        // the names indented under it — 2,968 of 8,516 in its
                        // own tree, so a third of all constants and types were
                        // missed entirely. The keyword opens the group; each
                        // indented name below is a definition of its own,
                        // recorded until a line starts at column zero again.
                        if tokens
                            .get(i + 1)
                            .is_some_and(|t| t.kind == TokenKind::Newline)
                        {
                            let mut j = i + 1;
                            while j < tokens.len() {
                                match tokens[j].kind {
                                    TokenKind::Newline => j += 1,
                                    // A `##` line documents the entry above it
                                    // and must not end the group.
                                    TokenKind::DocComment(text)
                                    | TokenKind::LineComment(text)
                                    | TokenKind::BlockComment(text) => {
                                        scope.push_comment(text);
                                        j += 1;
                                    }
                                    TokenKind::Ident(raw) => {
                                        let line_start = src[..tokens[j].start as usize]
                                            .rfind('\n')
                                            .map_or(0, |p| p + 1);
                                        if tokens[j].start as usize == line_start {
                                            break;
                                        }
                                        let clean = raw.trim_end_matches('*');
                                        scope.open_statement_definition(
                                            clean,
                                            tokens[j].start as usize,
                                            &mut facts,
                                        );
                                        scope.on_word(clean);
                                        // Skip to the end of this entry's line.
                                        while j < tokens.len()
                                            && tokens[j].kind != TokenKind::Newline
                                        {
                                            j += 1;
                                        }
                                        scope.on_statement_end(
                                            tokens[j.min(tokens.len() - 1)].end as usize,
                                            &mut facts,
                                        );
                                    }
                                    _ => break,
                                }
                            }
                            i = j;
                            continue;
                        }
                        if i + 1 < tokens.len() {
                            if let TokenKind::Ident(raw_name) = tokens[i + 1].kind {
                                let clean_name = raw_name.trim_end_matches('*');
                                scope.open_definition(clean_name, start_byte as usize, false, &mut facts);
                                scope.on_word(clean_name);
                                i += 2;
                                if i < tokens.len() && tokens[i].kind == TokenKind::Symbol('*') {
                                    i += 1;
                                }
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
                            && !matches!(*ident, "if" | "elif" | "while" | "for" | "return" | "case" | "of" | "when" | "discard")
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

pub fn parse_odin(src: &str) -> FileFacts {
    let mut facts = FileFacts::new();
    let style = CommentStyle {
        line_comment_prefix: &["//"],
        doc_comment_prefix: &["//"],
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
            TokenKind::Ident(ident) => {
                if i + 2 < tokens.len() && tokens[i + 1].kind == TokenKind::DoubleSymbol("::") {
                    let start_byte = tok.start;
                    let symbol_name = *ident;
                    let opens_body = matches!(tokens[i + 2].kind, TokenKind::Ident("proc" | "struct" | "enum" | "union"));
                    scope.open_definition(symbol_name, start_byte as usize, opens_body, &mut facts);
                    scope.on_word(symbol_name);
                    i += 3;
                    continue;
                }

                let mut j = i + 1;
                while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                    j += 1;
                }
                if j < tokens.len()
                    && tokens[j].kind == TokenKind::Symbol('(')
                    && !matches!(*ident, "if" | "for" | "switch" | "case" | "return" | "when" | "cast" | "transmute")
                {
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

pub fn parse_c(src: &str) -> FileFacts {
    let mut facts = FileFacts::new();
    let style = CommentStyle {
        line_comment_prefix: &["///", "//"],
        doc_comment_prefix: &["/**", "///", "/*!"],
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
            TokenKind::Symbol('#') => {
                i += 1;
                if i < tokens.len() && matches!(tokens[i].kind, TokenKind::Ident("define")) {
                    let start_byte = tok.start;
                    if i + 1 < tokens.len() {
                        if let TokenKind::Ident(macro_name) = tokens[i + 1].kind {
                            scope.open_definition_with_body_docs(macro_name, start_byte as usize, false, false, &mut facts);
                            scope.on_word(macro_name);
                            i += 2;
                            continue;
                        }
                    }
                }
                while i < tokens.len() && tokens[i].kind != TokenKind::Newline {
                    i += 1;
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
            TokenKind::Symbol('.') | TokenKind::DoubleSymbol("->") => {
                scope.on_receiver();
                i += 1;
                continue;
            }
            TokenKind::Ident(ident) => {
                if matches!(
                    *ident,
                    "static"
                        | "inline"
                        | "extern"
                        | "const"
                        | "volatile"
                        | "register"
                        | "auto"
                        | "restrict"
                        | "unsigned"
                        | "signed"
                        | "void"
                        | "int"
                        | "char"
                        | "short"
                        | "long"
                        | "float"
                        | "double"
                        | "size_t"
                        | "ssize_t"
                        | "int8_t"
                        | "int16_t"
                        | "int32_t"
                        | "int64_t"
                        | "uint8_t"
                        | "uint16_t"
                        | "uint32_t"
                        | "uint64_t"
                        | "bool"
                        | "_Bool"
                ) {
                    i += 1;
                    continue;
                }

                match *ident {
                    "struct" | "union" | "enum" => {
                        let start_byte = tok.start;
                        if i + 1 < tokens.len() {
                            if let TokenKind::Ident(name) = tokens[i + 1].kind {
                                let mut j = i + 2;
                                while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                                    j += 1;
                                }
                                if j < tokens.len() && tokens[j].kind == TokenKind::Symbol('{') {
                                    scope.open_definition_with_body_docs(name, start_byte as usize, true, false, &mut facts);
                                    scope.on_word(name);
                                    i = j;
                                    continue;
                                }
                            }
                        }
                    }
                    "typedef" => {
                        i += 1;
                        continue;
                    }
                    _ => {
                        let mut j = i + 1;
                        while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                            j += 1;
                        }
                        let mut is_fn_def = false;
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
                            while k < tokens.len() && (tokens[k].kind == TokenKind::Newline || matches!(tokens[k].kind, TokenKind::Ident(_) | TokenKind::Symbol('*'))) {
                                k += 1;
                            }
                            if k < tokens.len() && tokens[k].kind == TokenKind::Symbol('{') {
                                is_fn_def = true;
                            }
                        }

                        if is_fn_def {
                            let start_byte = tok.start;
                            scope.open_definition_with_body_docs(*ident, start_byte as usize, true, true, &mut facts);
                            scope.on_word(ident);
                            i += 1;
                            continue;
                        }

                        let mut j = i + 1;
                        while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                            j += 1;
                        }
                        if j < tokens.len()
                            && tokens[j].kind == TokenKind::Symbol('(')
                            && !matches!(
                                *ident,
                                "if" | "while"
                                    | "for"
                                    | "switch"
                                    | "return"
                                    | "sizeof"
                                    | "alignof"
                                    | "typeof"
                                    | "_Generic"
                            )
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

pub fn parse_cpp(src: &str) -> FileFacts {
    let mut facts = FileFacts::new();
    let style = CommentStyle {
        line_comment_prefix: &["///", "//"],
        doc_comment_prefix: &["/**", "///", "/*!"],
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
            TokenKind::Symbol('#') => {
                i += 1;
                if i < tokens.len() && matches!(tokens[i].kind, TokenKind::Ident("define")) {
                    let start_byte = tok.start;
                    if i + 1 < tokens.len() {
                        if let TokenKind::Ident(macro_name) = tokens[i + 1].kind {
                            scope.open_definition_with_body_docs(macro_name, start_byte as usize, false, false, &mut facts);
                            scope.on_word(macro_name);
                            i += 2;
                            continue;
                        }
                    }
                }
                while i < tokens.len() && tokens[i].kind != TokenKind::Newline {
                    i += 1;
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
            TokenKind::Symbol('.') | TokenKind::DoubleSymbol("->") | TokenKind::DoubleSymbol("::") => {
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
                        | "static"
                        | "inline"
                        | "virtual"
                        | "explicit"
                        | "friend"
                        | "constexpr"
                        | "consteval"
                        | "constinit"
                        | "const"
                        | "volatile"
                        | "mutable"
                        | "noexcept"
                        | "override"
                        | "final"
                        | "extern"
                        | "template"
                        | "typename"
                        | "auto"
                ) {
                    // `const int kMaxRetries = 3;` — the type sits between the
                    // keyword and the name, so the name is the token before `=`.
                    // Only at file scope: a local `const` is not a tunable.
                    if matches!(*ident, "const" | "constexpr") && scope.depth == 0 {
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
                                scope.open_statement_definition(name, tok.start as usize, &mut facts);
                                scope.on_word(name);
                                i = k + 1;
                                continue;
                            }
                        }
                    }
                    i += 1;
                    continue;
                }

                match *ident {
                    "class" | "struct" | "union" | "enum" | "namespace" | "concept" => {
                        let start_byte = tok.start;
                        let mut j = i + 1;
                        if j < tokens.len() && matches!(tokens[j].kind, TokenKind::Ident("class" | "struct")) {
                            j += 1;
                        }
                        if j < tokens.len() {
                            if let TokenKind::Ident(name) = tokens[j].kind {
                                let opens_body = matches!(*ident, "class" | "struct" | "union" | "enum" | "namespace");
                                scope.open_definition_with_body_docs(name, start_byte as usize, opens_body, false, &mut facts);
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
                        let mut is_fn_def = false;
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
                            while k < tokens.len() && (tokens[k].kind == TokenKind::Newline || matches!(tokens[k].kind, TokenKind::Ident(_) | TokenKind::Symbol(':') | TokenKind::DoubleSymbol("->") | TokenKind::Symbol('&') | TokenKind::Symbol('*'))) {
                                k += 1;
                            }
                            if k < tokens.len() && tokens[k].kind == TokenKind::Symbol('{') {
                                is_fn_def = true;
                            }
                        }

                        if is_fn_def {
                            let start_byte = tok.start;
                            scope.open_definition_with_body_docs(*ident, start_byte as usize, true, true, &mut facts);
                            scope.on_word(ident);
                            i += 1;
                            continue;
                        }

                        let mut j = i + 1;
                        while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                            j += 1;
                        }
                        if j < tokens.len()
                            && tokens[j].kind == TokenKind::Symbol('(')
                            && !matches!(
                                *ident,
                                "if" | "while"
                                    | "for"
                                    | "switch"
                                    | "catch"
                                    | "return"
                                    | "sizeof"
                                    | "decltype"
                                    | "typeid"
                                    | "alignof"
                                    | "dynamic_cast"
                                    | "static_cast"
                                    | "reinterpret_cast"
                                    | "const_cast"
                                    | "new"
                                    | "delete"
                                    | "throw"
                                    | "requires"
                                    | "co_await"
                                    | "co_yield"
                                    | "co_return"
                            )
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

pub fn parse_swift(src: &str) -> FileFacts {
    let mut facts = FileFacts::new();
    let style = CommentStyle {
        line_comment_prefix: &["///", "//"],
        doc_comment_prefix: &["///", "/**"],
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
            TokenKind::Symbol('.') | TokenKind::DoubleSymbol("?.") => {
                scope.on_receiver();
                i += 1;
                continue;
            }
            TokenKind::Ident(ident) => {
                if matches!(
                    *ident,
                    "public"
                        | "private"
                        | "fileprivate"
                        | "internal"
                        | "open"
                        | "static"
                        | "final"
                        | "mutating"
                        | "nonmutating"
                        | "override"
                        | "convenience"
                        | "required"
                        | "lazy"
                        | "weak"
                        | "unowned"
                        | "async"
                        | "throws"
                        | "rethrows"
                ) {
                    i += 1;
                    continue;
                }

                match *ident {
                    // `let maxRetries = 3` at file scope is a constant. Inside
                    // a body it is an ordinary binding, which nobody asks about.
                    "let" if scope.depth == 0 => {
                        if i + 1 < tokens.len() {
                            if let TokenKind::Ident(name) = tokens[i + 1].kind {
                                scope.open_statement_definition(name, tok.start as usize, &mut facts);
                                scope.on_word(name);
                                i += 2;
                                continue;
                            }
                        }
                    }
                    "class" | "struct" | "enum" | "protocol" | "extension" | "actor" => {
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
                    "func" => {
                        let start_byte = tok.start;
                        if i + 1 < tokens.len() {
                            if let TokenKind::Ident(name) = tokens[i + 1].kind {
                                scope.open_definition_with_body_docs(name, start_byte as usize, true, true, &mut facts);
                                scope.on_word(name);
                                i += 2;
                                continue;
                            }
                        }
                    }
                    "init" | "deinit" => {
                        let start_byte = tok.start;
                        scope.open_definition_with_body_docs(*ident, start_byte as usize, true, true, &mut facts);
                        scope.on_word(ident);
                        i += 1;
                        continue;
                    }
                    _ => {
                        let mut j = i + 1;
                        while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                            j += 1;
                        }
                        if j < tokens.len()
                            && tokens[j].kind == TokenKind::Symbol('(')
                            && !matches!(
                                *ident,
                                "if" | "guard"
                                    | "while"
                                    | "for"
                                    | "switch"
                                    | "case"
                                    | "catch"
                                    | "return"
                                    | "throw"
                                    | "try"
                                    | "await"
                                    | "let"
                                    | "var"
                            )
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

pub fn parse_ada(src: &str) -> FileFacts {
    let mut facts = FileFacts::new();
    let style = CommentStyle {
        line_comment_prefix: &["--"],
        doc_comment_prefix: &["--"],
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
            TokenKind::Ident(ident) => {
                let lower = ident.to_ascii_lowercase();

                if lower == "end" {
                    scope.on_close_delimiter(tok.end as usize, &mut facts);
                    i += 1;
                    continue;
                }

                match lower.as_str() {
                    "package" => {
                        let start_byte = tok.start;
                        let mut j = i + 1;
                        if j < tokens.len() && matches!(tokens[j].kind, TokenKind::Ident("body" | "BODY")) {
                            j += 1;
                        }
                        let mut pkg_name = String::new();
                        while j < tokens.len() && tokens[j].kind != TokenKind::Symbol(';') && tokens[j].kind != TokenKind::Newline && !matches!(tokens[j].kind, TokenKind::Ident("is" | "IS")) {
                            if let TokenKind::Ident(part) = tokens[j].kind {
                                pkg_name.push_str(part);
                            } else if let TokenKind::Symbol('.') = tokens[j].kind {
                                pkg_name.push('.');
                            }
                            j += 1;
                        }
                        if !pkg_name.is_empty() {
                            scope.open_definition_with_body_docs(&pkg_name, start_byte as usize, true, false, &mut facts);
                            scope.on_open_delimiter();
                            scope.on_word(&pkg_name);
                            i = j;
                            continue;
                        }
                    }
                    "procedure" | "function" | "entry" => {
                        let start_byte = tok.start;
                        if i + 1 < tokens.len() {
                            if let TokenKind::Ident(name) = tokens[i + 1].kind {
                                scope.open_definition_with_body_docs(name, start_byte as usize, true, true, &mut facts);
                                scope.on_open_delimiter();
                                scope.on_word(name);
                                i += 2;
                                continue;
                            }
                        }
                    }
                    "type" | "subtype" | "task" => {
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
                        if j < tokens.len()
                            && (tokens[j].kind == TokenKind::Symbol('(') || scope.had_receiver)
                            && !matches!(
                                lower.as_str(),
                                "if" | "then"
                                    | "else"
                                    | "elsif"
                                    | "while"
                                    | "for"
                                    | "loop"
                                    | "case"
                                    | "when"
                                    | "begin"
                                    | "declare"
                                    | "exception"
                                    | "raise"
                                    | "return"
                                    | "null"
                                    | "is"
                                    | "with"
                                    | "use"
                                    | "pragma"
                            )
                        {
                            scope.record_call(ident, &mut facts);
                        }
                        scope.on_word(ident);
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }

    scope.finish(src.len(), &mut facts);
    facts
}

pub fn parse_d(src: &str) -> FileFacts {
    let mut facts = FileFacts::new();
    let style = CommentStyle {
        line_comment_prefix: &["///", "//"],
        doc_comment_prefix: &["///", "/**", "/++"],
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
            TokenKind::Symbol('@') => {
                i += 1;
                if i < tokens.len() {
                    if let TokenKind::Ident(_) = tokens[i].kind {
                        i += 1;
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
                scope.on_statement_end(tok.end as usize, &mut facts);
                i += 1;
                continue;
            }
            TokenKind::Symbol('.') => {
                scope.on_receiver();
                i += 1;
                continue;
            }
            TokenKind::Ident(ident) => {
                if matches!(
                    *ident,
                    "public"
                        | "private"
                        | "protected"
                        | "package"
                        | "export"
                        | "static"
                        | "final"
                        | "abstract"
                        | "override"
                        | "pure"
                        | "nothrow"
                        | "const"
                        | "immutable"
                        | "shared"
                        | "gshared"
                        | "extern"
                        | "ref"
                        | "auto"
                ) {
                    i += 1;
                    continue;
                }

                match *ident {
                    "module" => {
                        let start_byte = tok.start;
                        let mut j = i + 1;
                        let mut mod_name = String::new();
                        while j < tokens.len() && tokens[j].kind != TokenKind::Symbol(';') && tokens[j].kind != TokenKind::Newline {
                            if let TokenKind::Ident(part) = tokens[j].kind {
                                mod_name.push_str(part);
                            } else if let TokenKind::Symbol('.') = tokens[j].kind {
                                mod_name.push('.');
                            }
                            j += 1;
                        }
                        if !mod_name.is_empty() {
                            scope.open_definition_with_body_docs(&mod_name, start_byte as usize, false, false, &mut facts);
                            scope.on_word(&mod_name);
                            i = j;
                            continue;
                        }
                    }
                    "class" | "struct" | "interface" | "union" | "enum" | "template" => {
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
                    _ => {
                        let mut j = i + 1;
                        while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                            j += 1;
                        }
                        let mut is_fn_def = false;
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
                            while k < tokens.len() && (tokens[k].kind == TokenKind::Newline || matches!(tokens[k].kind, TokenKind::Ident(_) | TokenKind::Symbol('@' | ':'))) {
                                k += 1;
                            }
                            if k < tokens.len() && tokens[k].kind == TokenKind::Symbol('{') {
                                is_fn_def = true;
                            }
                        }

                        if is_fn_def && !matches!(*ident, "if" | "while" | "for" | "foreach" | "switch" | "catch" | "version" | "debug") {
                            let start_byte = tok.start;
                            scope.open_definition_with_body_docs(*ident, start_byte as usize, true, true, &mut facts);
                            scope.on_word(ident);
                            i += 1;
                            continue;
                        }

                        let mut j = i + 1;
                        while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                            j += 1;
                        }
                        if j < tokens.len()
                            && (tokens[j].kind == TokenKind::Symbol('(') || scope.had_receiver)
                            && !matches!(
                                *ident,
                                "if" | "else"
                                    | "while"
                                    | "for"
                                    | "foreach"
                                    | "switch"
                                    | "case"
                                    | "default"
                                    | "catch"
                                    | "finally"
                                    | "try"
                                    | "throw"
                                    | "return"
                                    | "new"
                                    | "delete"
                                    | "this"
                                    | "super"
                                    | "assert"
                                    | "import"
                                    | "cast"
                                    | "typeid"
                                    | "sizeof"
                                    | "mixin"
                                    | "version"
                                    | "debug"
                            )
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

pub fn parse_wat(src: &str) -> FileFacts {
    let mut facts = FileFacts::new();
    let style = CommentStyle {
        line_comment_prefix: &["//", ";;"],
        doc_comment_prefix: &[],
        block_comment_start: Some("(;"),
        block_comment_end: Some(";)"),
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
            TokenKind::Symbol('(') => {
                scope.on_open_delimiter();
                if i + 1 < tokens.len() {
                    if let TokenKind::Ident(keyword) = tokens[i + 1].kind {
                        let is_func = keyword == "func";
                        let is_entity = matches!(keyword, "table" | "memory" | "global" | "type" | "data" | "elem");
                        if is_func || is_entity {
                            let start_byte = tok.start;
                            let mut next_idx = i + 2;
                            let mut name_start = None;
                            let mut name_end = None;
                            if next_idx < tokens.len() && tokens[next_idx].kind == TokenKind::Symbol('$') {
                                let s = tokens[next_idx].start;
                                next_idx += 1;
                                if next_idx < tokens.len() && matches!(tokens[next_idx].kind, TokenKind::Ident(_)) {
                                    name_start = Some(s);
                                    name_end = Some(tokens[next_idx].end);
                                    next_idx += 1;
                                }
                            } else if next_idx < tokens.len() && matches!(tokens[next_idx].kind, TokenKind::Ident(_)) {
                                name_start = Some(tokens[next_idx].start);
                                name_end = Some(tokens[next_idx].end);
                                next_idx += 1;
                            } else if tokens.get(next_idx).map(|t| &t.kind)
                                == Some(&TokenKind::Symbol('('))
                            {
                                // **`(func (export "jsPrint") …)` names itself
                                // through its export**, with no `$name` at all,
                                // and the corpus is what showed it: every such
                                // function was anonymous and its calls fell to
                                // `<module>` — 211 definitions per 1,000 lines
                                // and **100%** attribution, a contradiction the
                                // fixture could not produce because it writes
                                // only the `$name` form.
                                if tokens.get(next_idx + 1).map(|t| &t.kind)
                                    == Some(&TokenKind::Ident("export"))
                                {
                                    if let Some(TokenKind::StringLit(raw)) =
                                        tokens.get(next_idx + 2).map(|t| &t.kind)
                                    {
                                        let clean = raw.trim_matches('"');
                                        if !clean.is_empty() {
                                            name_start = Some(tokens[next_idx + 2].start + 1);
                                            name_end =
                                                Some(tokens[next_idx + 2].end.saturating_sub(1));
                                            // Step past the export group's own
                                            // closing paren. Leaving it open
                                            // left the definition one level too
                                            // deep, so the body's `call` still
                                            // belonged to the file: definitions
                                            // rose and attribution stayed at
                                            // 100%, which is the shape of a
                                            // half-applied scope fix this
                                            // project has recorded eleven times.
                                            next_idx += 3;
                                            if tokens.get(next_idx).map(|t| &t.kind)
                                                == Some(&TokenKind::Symbol(')'))
                                            {
                                                next_idx += 1;
                                            }
                                        }
                                    }
                                }
                            }

                            if let (Some(s), Some(e)) = (name_start, name_end) {
                                let full_name = &src[s as usize..e as usize];
                                scope.open_definition_with_body_docs(full_name, start_byte as usize, true, is_func, &mut facts);
                                if let Some(last) = scope.open.last_mut() {
                                    last.depth = scope.depth - 1;
                                }
                                if let Some(last) = scope.enclosing.last_mut() {
                                    last.1 = scope.depth - 1;
                                }
                                scope.on_word(full_name);
                                i = next_idx;
                                continue;
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
                if *ident == "call" || *ident == "call_indirect" {
                    let mut next_idx = i + 1;
                    let mut callee_start = None;
                    let mut callee_end = None;
                    if next_idx < tokens.len() && tokens[next_idx].kind == TokenKind::Symbol('$') {
                        let s = tokens[next_idx].start;
                        next_idx += 1;
                        if next_idx < tokens.len() && matches!(tokens[next_idx].kind, TokenKind::Ident(_)) {
                            callee_start = Some(s);
                            callee_end = Some(tokens[next_idx].end);
                            next_idx += 1;
                        }
                    } else if next_idx < tokens.len() && matches!(tokens[next_idx].kind, TokenKind::Ident(_)) {
                        callee_start = Some(tokens[next_idx].start);
                        callee_end = Some(tokens[next_idx].end);
                        next_idx += 1;
                    }
                    if let (Some(s), Some(e)) = (callee_start, callee_end) {
                        let full_callee = &src[s as usize..e as usize];
                        scope.record_call(full_callee, &mut facts);
                        i = next_idx;
                        continue;
                    }
                }
                scope.on_word(ident);
            }
            _ => {}
        }
        i += 1;
    }

    scope.finish(src.len(), &mut facts);
    facts
}
