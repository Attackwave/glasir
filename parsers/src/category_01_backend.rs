//! Category 1: Mainstream, Systems & Backend
//! Parsers for: Rust, Go, Python, TypeScript/JavaScript

use crate::facts::FileFacts;
use crate::lexer::{CommentStyle, Lexer, TokenKind};
use crate::scope::ScopeStack;

pub fn parse_rust(src: &str) -> FileFacts {
    crate::rules::active().parse(crate::Language::Rust, src)
}

pub fn parse_go(src: &str) -> FileFacts {
    crate::rules::active().parse(crate::Language::Go, src)
}

pub fn parse_python(src: &str) -> FileFacts {
    let mut facts = FileFacts::new();
    let style = CommentStyle {
        line_comment_prefix: &["#"],
        doc_comment_prefix: &["#"],
        block_comment_start: None,
        block_comment_end: None,
        ident_suffix_marks: false,
        ident_dashes: false,
        raw_escapes: true,
    };
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
                            && !matches!(
                                *ident,
                                "if" | "elif"
                                    | "while"
                                    | "for"
                                    | "return"
                                    | "with"
                                    | "assert"
                                    | "import"
                                    | "from"
                                    | "raise"
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

pub fn parse_typescript_javascript(src: &str) -> FileFacts {
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
            TokenKind::Symbol('@') => {
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
            TokenKind::Symbol('.') | TokenKind::DoubleSymbol("?.") => {
                scope.on_receiver();
                i += 1;
                continue;
            }
            TokenKind::Ident(ident) => {
                if matches!(
                    *ident,
                    "export"
                        | "default"
                        | "async"
                        | "declare"
                        | "public"
                        | "private"
                        | "protected"
                        | "static"
                        | "readonly"
                ) {
                    i += 1;
                    continue;
                }

                match *ident {
                    "function" => {
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
                                scope.on_word(name);
                                i += 2;
                                continue;
                            }
                        }
                    }
                    "class" | "interface" | "type" | "enum" => {
                        let start_byte = tok.start;
                        if i + 1 < tokens.len() {
                            if let TokenKind::Ident(name) = tokens[i + 1].kind {
                                let opens_body = matches!(*ident, "class" | "interface" | "enum");
                                scope.open_definition_with_body_docs(
                                    name,
                                    start_byte as usize,
                                    opens_body,
                                    false,
                                    &mut facts,
                                );
                                scope.on_word(name);
                                i += 2;
                                continue;
                            }
                        }
                    }
                    // Only at file scope. Measured on 80 real files, 2,608 of
                    // 2,823 matched constants were function-local, and every
                    // one became a graph node nobody searches for.
                    "const" | "let" | "var" if scope.depth == 0 => {
                        let start_byte = tok.start;
                        if i + 1 < tokens.len() {
                            if let TokenKind::Ident(name) = tokens[i + 1].kind {
                                scope.open_statement_definition(
                                    name,
                                    start_byte as usize,
                                    &mut facts,
                                );
                                scope.on_word(name);
                                i += 2;
                                continue;
                            }
                        }
                    }
                    _ => {
                        // Check if this is a class method definition: `methodName(...) {`
                        let mut j = i + 1;
                        while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                            j += 1;
                        }
                        let mut is_method_def = false;
                        // `if (x) { … }` has exactly the shape of a shorthand
                        // method, and taking it as one put definitions called
                        // `if` and `catch` in the graph. Those collide with
                        // real names across files: measured on this tree,
                        // `assets/check_view.js` grew edges into `src/mcp.rs`
                        // and `cycles` reported a JavaScript file calling Rust.
                        if !crate::scope::is_control_keyword(ident)
                            && j < tokens.len()
                            && tokens[j].kind == TokenKind::Symbol('(')
                        {
                            // Look ahead past ')' and optional type annotations ': Type' to see if '{' follows
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
                                        TokenKind::Ident(_)
                                            | TokenKind::Symbol(':')
                                            | TokenKind::DoubleSymbol("=>")
                                    ))
                            {
                                k += 1;
                            }
                            if k < tokens.len()
                                && tokens[k].kind == TokenKind::Symbol('{')
                                && scope.depth > 0
                            {
                                is_method_def = true;
                            }
                        }

                        if is_method_def {
                            let start_byte = tok.start;
                            scope.open_definition(*ident, start_byte as usize, true, &mut facts);
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
                                    | "typeof"
                                    | "instanceof"
                                    | "new"
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

pub fn parse_typescript(src: &str) -> FileFacts {
    parse_typescript_javascript(src)
}

pub fn parse_javascript(src: &str) -> FileFacts {
    parse_typescript_javascript(src)
}

pub fn parse_java(src: &str) -> FileFacts {
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
    // `static final` seen, so the next `<type> <NAME> =` is a constant.
    let mut saw_static = false;
    let mut saw_final = false;

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
            TokenKind::Symbol('@') => {
                i += 1;
                if i < tokens.len() && matches!(tokens[i].kind, TokenKind::Ident("interface")) {
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
            TokenKind::Symbol('.') => {
                scope.on_receiver();
                i += 1;
                continue;
            }
            TokenKind::DoubleSymbol("::") => {
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
                        | "final"
                        | "abstract"
                        | "synchronized"
                        | "native"
                        | "strictfp"
                        | "transient"
                        | "volatile"
                        | "default"
                        | "sealed"
                        | "non-sealed"
                ) {
                    // Java writes a constant as `static final int MAX = 3;` —
                    // there is no keyword of its own, so the modifier pair is
                    // what marks one.
                    if *ident == "static" {
                        saw_static = true;
                    } else if *ident == "final" && saw_static {
                        saw_final = true;
                    }
                    i += 1;
                    continue;
                }

                if saw_final {
                    // `<type> <NAME> =` — the name is the token before the `=`.
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
                    saw_static = false;
                    saw_final = false;
                    if ok {
                        if let TokenKind::Ident(name) = tokens[k].kind {
                            scope.open_statement_definition(name, tok.start as usize, &mut facts);
                            scope.on_word(name);
                            i = k + 1;
                            continue;
                        }
                    }
                }

                match *ident {
                    "class" | "interface" | "enum" | "record" => {
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
                    _ => {
                        let mut j = i + 1;
                        while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                            j += 1;
                        }
                        let mut is_method_def = false;
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
                            while k < tokens.len()
                                && (tokens[k].kind == TokenKind::Newline
                                    || matches!(
                                        tokens[k].kind,
                                        TokenKind::Ident(_) | TokenKind::Symbol(',')
                                    ))
                            {
                                k += 1;
                            }
                            if k < tokens.len()
                                && tokens[k].kind == TokenKind::Symbol('{')
                                && scope.depth > 0
                            {
                                is_method_def = true;
                            }
                        }

                        if is_method_def {
                            let start_byte = tok.start;
                            scope.open_definition(*ident, start_byte as usize, true, &mut facts);
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
                                    | "synchronized"
                                    | "return"
                                    | "throw"
                                    | "new"
                                    | "super"
                                    | "this"
                                    | "assert"
                                    | "instanceof"
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

pub fn parse_csharp(src: &str) -> FileFacts {
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
                    // `const int MaxRetries = 3;` — the type sits between the
                    // keyword and the name.
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
                                    tok.start as usize,
                                    &mut facts,
                                );
                                scope.on_word(name);
                                i = k + 1;
                                continue;
                            }
                        }
                    }
                    "class" | "struct" | "interface" | "enum" | "record" => {
                        let start_byte = tok.start;
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
                        let mut j = i + 1;
                        while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                            j += 1;
                        }
                        let mut is_method_def = false;
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
                            while k < tokens.len()
                                && (tokens[k].kind == TokenKind::Newline
                                    || matches!(
                                        tokens[k].kind,
                                        TokenKind::Ident(_)
                                            | TokenKind::Symbol(':')
                                            | TokenKind::DoubleSymbol("=>")
                                    ))
                            {
                                k += 1;
                            }
                            if k < tokens.len()
                                && tokens[k].kind == TokenKind::Symbol('{')
                                && scope.depth > 0
                            {
                                is_method_def = true;
                            }
                        }

                        if is_method_def {
                            let start_byte = tok.start;
                            scope.open_definition(*ident, start_byte as usize, true, &mut facts);
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
                                    | "foreach"
                                    | "switch"
                                    | "catch"
                                    | "using"
                                    | "lock"
                                    | "fixed"
                                    | "return"
                                    | "yield"
                                    | "throw"
                                    | "new"
                                    | "typeof"
                                    | "sizeof"
                                    | "nameof"
                                    | "checked"
                                    | "unchecked"
                                    | "default"
                                    | "is"
                                    | "as"
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

pub fn parse_ruby(src: &str) -> FileFacts {
    crate::rules::active().parse(crate::Language::Ruby, src)
}

pub fn parse_php(src: &str) -> FileFacts {
    let mut facts = FileFacts::new();
    let style = CommentStyle {
        line_comment_prefix: &["//", "#"],
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
            TokenKind::DoubleSymbol("->")
            | TokenKind::DoubleSymbol("?->")
            | TokenKind::DoubleSymbol("::") => {
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
                        | "final"
                        | "abstract"
                        | "readonly"
                ) {
                    i += 1;
                    continue;
                }

                match *ident {
                    // `const MAX_RETRIES = 3;` — PHP names the constant right
                    // after the keyword, unlike C# and Java.
                    "const" => {
                        if i + 1 < tokens.len() {
                            if let TokenKind::Ident(name) = tokens[i + 1].kind {
                                scope.open_statement_definition(
                                    name,
                                    tok.start as usize,
                                    &mut facts,
                                );
                                scope.on_word(name);
                                i += 2;
                                continue;
                            }
                        }
                    }
                    "class" | "interface" | "trait" | "enum" => {
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
                    "function" | "fn" => {
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
                                scope.on_word(name);
                                i += 2;
                                continue;
                            }
                        }
                    }
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
                            } else if let TokenKind::Symbol('\\') = tokens[j].kind {
                                ns_name.push('\\');
                            }
                            j += 1;
                        }
                        if !ns_name.is_empty() {
                            scope.open_definition_with_body_docs(
                                &ns_name,
                                start_byte as usize,
                                false,
                                false,
                                &mut facts,
                            );
                            scope.on_word(&ns_name);
                            i = j;
                            continue;
                        }
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
                                "if" | "elseif"
                                    | "else"
                                    | "while"
                                    | "for"
                                    | "foreach"
                                    | "switch"
                                    | "case"
                                    | "catch"
                                    | "return"
                                    | "echo"
                                    | "print"
                                    | "include"
                                    | "require"
                                    | "include_once"
                                    | "require_once"
                                    | "throw"
                                    | "new"
                                    | "match"
                                    | "isset"
                                    | "empty"
                                    | "unset"
                                    | "list"
                                    | "array"
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

pub fn parse_groovy(src: &str) -> FileFacts {
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
                scope.on_statement_end(tok.end as usize, &mut facts);
                i += 1;
                continue;
            }
            TokenKind::Symbol('.')
            | TokenKind::DoubleSymbol("?.")
            | TokenKind::DoubleSymbol("&.") => {
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
                        | "final"
                        | "abstract"
                        | "synchronized"
                        | "transient"
                        | "volatile"
                        | "strictfp"
                ) {
                    i += 1;
                    continue;
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
                    "class" | "interface" | "trait" | "enum" | "record" => {
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
                        let in_method = scope.open.iter().any(|o| o.collects_body_docs);
                        if i + 1 < tokens.len() {
                            if let TokenKind::Ident(name) = tokens[i + 1].kind {
                                if !in_method {
                                    scope.open_definition_with_body_docs(
                                        name,
                                        start_byte as usize,
                                        true,
                                        true,
                                        &mut facts,
                                    );
                                    scope.on_word(name);
                                    i += 2;
                                    continue;
                                }
                            }
                        }
                    }
                    _ => {
                        let mut j = i + 1;
                        while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                            j += 1;
                        }
                        let mut is_method_def = false;
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
                            while k < tokens.len()
                                && (tokens[k].kind == TokenKind::Newline
                                    || matches!(
                                        tokens[k].kind,
                                        TokenKind::Ident(_) | TokenKind::Symbol(',')
                                    ))
                            {
                                k += 1;
                            }
                            if k < tokens.len()
                                && tokens[k].kind == TokenKind::Symbol('{')
                                && scope.depth > 0
                            {
                                is_method_def = true;
                            }
                        }

                        if is_method_def {
                            let start_byte = tok.start;
                            scope.open_definition_with_body_docs(
                                *ident,
                                start_byte as usize,
                                true,
                                true,
                                &mut facts,
                            );
                            scope.on_word(ident);
                            i += 1;
                            continue;
                        }

                        let mut j = i + 1;
                        while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                            j += 1;
                        }
                        let is_call = j < tokens.len()
                            && (tokens[j].kind == TokenKind::Symbol('(')
                                || tokens[j].kind == TokenKind::Symbol('{'));
                        if is_call
                            && !matches!(
                                *ident,
                                "if" | "while"
                                    | "for"
                                    | "switch"
                                    | "catch"
                                    | "synchronized"
                                    | "return"
                                    | "throw"
                                    | "new"
                                    | "super"
                                    | "this"
                                    | "assert"
                                    | "instanceof"
                                    | "in"
                                    | "as"
                                    | "import"
                                    | "package"
                                    | "class"
                                    | "interface"
                                    | "trait"
                                    | "enum"
                                    | "def"
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

pub fn parse_vb(src: &str) -> FileFacts {
    let mut facts = FileFacts::new();
    let style = CommentStyle {
        line_comment_prefix: &["'''", "'", "REM ", "Rem "],
        doc_comment_prefix: &["'''"],
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
            // `<Assembly: AssemblyTitle("x")>` is a declaration, not a call.
            // 129 of 469 VB files in the sample corpus are a generated
            // `AssemblyInfo.vb` that is nothing but these, and each attribute
            // was recorded as a call from `<module>`.
            TokenKind::Symbol('<') => {
                let mut depth = 1;
                i += 1;
                while i < tokens.len() && depth > 0 {
                    match tokens[i].kind {
                        TokenKind::Symbol('<') => depth += 1,
                        TokenKind::Symbol('>') => depth -= 1,
                        TokenKind::Newline => break,
                        _ => {}
                    }
                    i += 1;
                }
                continue;
            }
            TokenKind::Symbol('.') | TokenKind::DoubleSymbol("?.") => {
                // In VB, Me. and MyBase. are self receivers
                let is_self = matches!(
                    scope.last_word.as_deref().map(|s| s.to_ascii_lowercase()),
                    Some(ref s) if s == "me" || s == "mybase" || s == "myclass"
                );
                scope.had_receiver = !is_self;
                i += 1;
                continue;
            }
            TokenKind::Ident(ident) => {
                let lower = ident.to_ascii_lowercase();

                // Check for "End Sub", "End Function", "End Class", "End Module", "End Structure", "End Interface", "End Property", "End Namespace", "End Enum"
                if lower == "end" {
                    if i + 1 < tokens.len() {
                        if let TokenKind::Ident(kind) = tokens[i + 1].kind {
                            let k_lower = kind.to_ascii_lowercase();
                            if matches!(
                                k_lower.as_str(),
                                "sub"
                                    | "function"
                                    | "class"
                                    | "module"
                                    | "structure"
                                    | "interface"
                                    | "property"
                                    | "namespace"
                                    | "enum"
                            ) {
                                scope.on_close_delimiter(tokens[i + 1].end as usize, &mut facts);
                                i += 2;
                                continue;
                            }
                        }
                    }
                    i += 1;
                    continue;
                }

                if matches!(
                    lower.as_str(),
                    "public"
                        | "private"
                        | "protected"
                        | "friend"
                        | "shared"
                        | "overridable"
                        | "overrides"
                        | "mustoverride"
                        | "notoverridable"
                        | "readonly"
                        | "writeonly"
                        | "shadows"
                        | "partial"
                        | "async"
                        | "iterator"
                        | "default"
                        | "dim"
                        | "const"
                ) {
                    i += 1;
                    continue;
                }

                match lower.as_str() {
                    "namespace" => {
                        let start_byte = tok.start;
                        let mut j = i + 1;
                        let mut ns_name = String::new();
                        while j < tokens.len() && tokens[j].kind != TokenKind::Newline {
                            if let TokenKind::Ident(part) = tokens[j].kind {
                                ns_name.push_str(part);
                            } else if let TokenKind::Symbol('.') = tokens[j].kind {
                                ns_name.push('.');
                            }
                            j += 1;
                        }
                        if !ns_name.is_empty() {
                            scope.open_definition_with_body_docs(
                                &ns_name,
                                start_byte as usize,
                                true,
                                false,
                                &mut facts,
                            );
                            scope.on_open_delimiter();
                            scope.on_word(&ns_name);
                            i = j;
                            continue;
                        }
                    }
                    "class" | "module" | "structure" | "interface" | "enum" => {
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
                                scope.on_open_delimiter();
                                scope.on_word(name);
                                i += 2;
                                continue;
                            }
                        }
                    }
                    "sub" | "function" | "property" => {
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
                                scope.on_open_delimiter();
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
                        let is_call = j < tokens.len() && tokens[j].kind == TokenKind::Symbol('(');
                        if is_call
                            && !matches!(
                                lower.as_str(),
                                "if" | "then"
                                    | "else"
                                    | "elseif"
                                    | "while"
                                    | "for"
                                    | "each"
                                    | "to"
                                    | "step"
                                    | "next"
                                    | "select"
                                    | "case"
                                    | "try"
                                    | "catch"
                                    | "finally"
                                    | "throw"
                                    | "return"
                                    | "new"
                                    | "me"
                                    | "mybase"
                                    | "myclass"
                                    | "inherits"
                                    | "implements"
                                    | "imports"
                                    | "using"
                                    | "with"
                                    | "as"
                                    | "is"
                                    | "isnot"
                                    | "and"
                                    | "or"
                                    | "not"
                                    | "xor"
                                    | "andalso"
                                    | "orelse"
                                    | "typeof"
                                    | "cint"
                                    | "cstr"
                                    | "cdbl"
                                    | "cbool"
                                    | "cobj"
                                    | "ctype"
                                    | "directcast"
                                    | "trycast"
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

pub fn parse_cobol(src: &str) -> FileFacts {
    let mut facts = FileFacts::new();
    let style = CommentStyle {
        line_comment_prefix: &["*>", "*"],
        doc_comment_prefix: &["*>"],
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
            TokenKind::Ident(_) => {
                let start_byte = tok.start;
                let mut end_byte = tok.end;
                i += 1;
                while i + 1 < tokens.len() && tokens[i].kind == TokenKind::Symbol('-') {
                    if let TokenKind::Ident(_) = tokens[i + 1].kind {
                        end_byte = tokens[i + 1].end;
                        i += 2;
                    } else {
                        break;
                    }
                }
                let full_word = &src[start_byte as usize..end_byte as usize];
                let upper = full_word.to_ascii_uppercase();

                if upper == "PROGRAM-ID" || upper == "FUNCTION-ID" {
                    if i < tokens.len() && tokens[i].kind == TokenKind::Symbol('.') {
                        i += 1;
                    }
                    while i < tokens.len() && tokens[i].kind == TokenKind::Newline {
                        i += 1;
                    }
                    if i < tokens.len() && matches!(tokens[i].kind, TokenKind::Ident(_)) {
                        let name_start = tokens[i].start;
                        let mut name_end = tokens[i].end;
                        i += 1;
                        while i + 1 < tokens.len() && tokens[i].kind == TokenKind::Symbol('-') {
                            if let TokenKind::Ident(_) = tokens[i + 1].kind {
                                name_end = tokens[i + 1].end;
                                i += 2;
                            } else {
                                break;
                            }
                        }
                        let prog_name = &src[name_start as usize..name_end as usize];
                        scope.close_definitions_at_or_above(0, start_byte as usize, &mut facts);
                        scope.open_definition_with_body_docs(
                            prog_name,
                            start_byte as usize,
                            true,
                            true,
                            &mut facts,
                        );
                        scope.on_word(prog_name);
                        continue;
                    }
                } else if upper == "PERFORM" {
                    while i < tokens.len() && tokens[i].kind == TokenKind::Newline {
                        i += 1;
                    }
                    if i < tokens.len() && matches!(tokens[i].kind, TokenKind::Ident(_)) {
                        let name_start = tokens[i].start;
                        let mut name_end = tokens[i].end;
                        i += 1;
                        while i + 1 < tokens.len() && tokens[i].kind == TokenKind::Symbol('-') {
                            if let TokenKind::Ident(_) = tokens[i + 1].kind {
                                name_end = tokens[i + 1].end;
                                i += 2;
                            } else {
                                break;
                            }
                        }
                        let callee = &src[name_start as usize..name_end as usize];
                        let callee_upper = callee.to_ascii_uppercase();
                        if !matches!(
                            callee_upper.as_str(),
                            "UNTIL" | "VARYING" | "WITH" | "TEST" | "THRU" | "TIMES"
                        ) {
                            scope.record_call(callee, &mut facts);
                        }
                        continue;
                    }
                } else if upper == "CALL" {
                    while i < tokens.len() && tokens[i].kind == TokenKind::Newline {
                        i += 1;
                    }
                    if i < tokens.len() {
                        if let TokenKind::StringLit(callee_str) = tokens[i].kind {
                            let clean_name = callee_str.trim_matches('"').trim_matches('\'');
                            scope.record_call(clean_name, &mut facts);
                            i += 1;
                            continue;
                        } else if matches!(tokens[i].kind, TokenKind::Ident(_)) {
                            let name_start = tokens[i].start;
                            let mut name_end = tokens[i].end;
                            i += 1;
                            while i + 1 < tokens.len() && tokens[i].kind == TokenKind::Symbol('-') {
                                if let TokenKind::Ident(_) = tokens[i + 1].kind {
                                    name_end = tokens[i + 1].end;
                                    i += 2;
                                } else {
                                    break;
                                }
                            }
                            let callee = &src[name_start as usize..name_end as usize];
                            scope.record_call(callee, &mut facts);
                            continue;
                        }
                    }
                } else {
                    let is_section = i < tokens.len()
                        && matches!(tokens[i].kind, TokenKind::Ident("SECTION" | "section"));
                    let is_paragraph = i < tokens.len()
                        && tokens[i].kind == TokenKind::Symbol('.')
                        && !matches!(
                            upper.as_str(),
                            "DIVISION"
                                | "SECTION"
                                | "IDENTIFICATION"
                                | "ENVIRONMENT"
                                | "DATA"
                                | "PROCEDURE"
                                | "WORKING-STORAGE"
                                | "STOP"
                                | "EXIT"
                                | "GOBACK"
                        );

                    if is_section || is_paragraph {
                        scope.close_definitions_at_or_above(1, start_byte as usize, &mut facts);
                        scope.open_definition_with_body_docs(
                            full_word,
                            start_byte as usize,
                            true,
                            true,
                            &mut facts,
                        );
                        scope.on_word(full_word);
                        i += 1;
                        continue;
                    }
                    scope.on_word(full_word);
                }
            }
            _ => {
                i += 1;
            }
        }
    }

    scope.finish(src.len(), &mut facts);
    facts
}
