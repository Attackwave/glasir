//! Category 7: Scripting & Automation
//! Parsers for: Bash/Shell, Perl, Lua

use crate::facts::FileFacts;
use crate::lexer::{CommentStyle, Lexer, TokenKind};
use crate::scope::ScopeStack;

pub fn parse_bash(src: &str) -> FileFacts {
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
            TokenKind::Ident(ident) => {
                // `MAX_RETRIES=3` — a shell has no keyword for a constant,
                // and the assignment carries no spaces around the `=`.
                if crate::scope::is_screaming_case(ident)
                    && i + 1 < tokens.len()
                    && tokens[i + 1].kind == TokenKind::Symbol('=')
                {
                    scope.open_statement_definition(*ident, tok.start as usize, &mut facts);
                    scope.on_word(ident);
                    i += 1;
                    continue;
                }
                if *ident == "function" {
                    let start_byte = tok.start;
                    if i + 1 < tokens.len() {
                        if let TokenKind::Ident(fn_name) = tokens[i + 1].kind {
                            scope.open_definition(fn_name, start_byte as usize, true, &mut facts);
                            scope.on_word(fn_name);
                            i += 2;
                            continue;
                        }
                    }
                } else if i + 2 < tokens.len()
                    && tokens[i + 1].kind == TokenKind::Symbol('(')
                    && tokens[i + 2].kind == TokenKind::Symbol(')')
                {
                    let start_byte = tok.start;
                    scope.open_definition(*ident, start_byte as usize, true, &mut facts);
                    scope.on_word(ident);
                    i += 3;
                    continue;
                } else {
                    if !matches!(
                        *ident,
                        "if" | "then"
                            | "else"
                            | "elif"
                            | "fi"
                            | "for"
                            | "while"
                            | "until"
                            | "do"
                            | "done"
                            | "case"
                            | "esac"
                            | "in"
                            | "return"
                            | "exit"
                            | "local"
                            | "export"
                            | "echo"
                    ) {
                        scope.record_call(ident, &mut facts);
                    }
                    scope.on_word(ident);
                }
            }
            _ => {}
        }
        i += 1;
    }

    scope.finish(src.len(), &mut facts);
    facts
}

pub fn parse_perl(src: &str) -> FileFacts {
    let mut facts = FileFacts::new();
    let style = CommentStyle {
        line_comment_prefix: &["#"],
        doc_comment_prefix: &["#"],
        block_comment_start: Some("=pod"),
        block_comment_end: Some("=cut"),
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
            TokenKind::DoubleSymbol("->") => {
                scope.on_receiver();
                i += 1;
                continue;
            }
            TokenKind::Ident(ident) => match *ident {
                "sub" | "method" => {
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
                "package" | "class" => {
                    let start_byte = tok.start;
                    if i + 1 < tokens.len() {
                        if let TokenKind::Ident(pkg_name) = tokens[i + 1].kind {
                            scope.open_definition(pkg_name, start_byte as usize, true, &mut facts);
                            scope.on_word(pkg_name);
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
                            "if" | "unless"
                                | "while"
                                | "until"
                                | "for"
                                | "foreach"
                                | "return"
                                | "my"
                                | "our"
                                | "state"
                        )
                    {
                        scope.record_call(ident, &mut facts);
                    }
                    scope.on_word(ident);
                }
            },
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

pub fn parse_lua(src: &str) -> FileFacts {
    let mut facts = FileFacts::new();
    let style = CommentStyle {
        line_comment_prefix: &["--"],
        doc_comment_prefix: &["--"],
        block_comment_start: Some("--[["),
        block_comment_end: Some("]]"),
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
            TokenKind::Symbol('.') | TokenKind::Symbol(':') => {
                scope.on_receiver();
                i += 1;
                continue;
            }
            TokenKind::Ident(ident) => {
                match *ident {
                    // `local MAX_RETRIES = 3` at file scope. Lua marks a
                    // module's tunables this way; a local inside a function is
                    // not one, hence the depth test.
                    "local" if scope.depth == 0 => {
                        if i + 1 < tokens.len() {
                            if let TokenKind::Ident(name) = tokens[i + 1].kind {
                                if i + 2 < tokens.len()
                                    && tokens[i + 2].kind == TokenKind::Symbol('=')
                                {
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
                    }
                    "function" => {
                        let start_byte = tok.start;
                        let mut j = i + 1;
                        let mut fn_name = String::new();
                        while j < tokens.len() {
                            if let TokenKind::Ident(part) = tokens[j].kind {
                                fn_name.push_str(part);
                                if j + 1 < tokens.len()
                                    && (tokens[j + 1].kind == TokenKind::Symbol('.')
                                        || tokens[j + 1].kind == TokenKind::Symbol(':'))
                                {
                                    if tokens[j + 1].kind == TokenKind::Symbol('.') {
                                        fn_name.push('.');
                                    } else {
                                        fn_name.push(':');
                                    }
                                    j += 2;
                                    continue;
                                }
                                j += 1;
                                break;
                            } else {
                                break;
                            }
                        }
                        if !fn_name.is_empty() {
                            scope.open_definition(&fn_name, start_byte as usize, true, &mut facts);
                            // See languages/lua.rs: the body opens a level the
                            // matching `end` closes, and `open_definition`
                            // does not raise the depth itself.
                            scope.on_open_delimiter();
                            scope.on_word(&fn_name);
                            i = j;
                            continue;
                        }
                    }
                    "if" | "for" | "while" | "do" => {
                        scope.on_open_delimiter();
                    }
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
                                || matches!(
                                    tokens[j].kind,
                                    TokenKind::StringLit(_) | TokenKind::Symbol('{')
                                ))
                            && !matches!(
                                *ident,
                                "if" | "then"
                                    | "else"
                                    | "elseif"
                                    | "while"
                                    | "repeat"
                                    | "until"
                                    | "for"
                                    | "do"
                                    | "return"
                                    | "local"
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

pub fn parse_powershell(src: &str) -> FileFacts {
    let mut facts = FileFacts::new();
    let style = CommentStyle {
        line_comment_prefix: &["#"],
        doc_comment_prefix: &["#"],
        block_comment_start: Some("<#"),
        block_comment_end: Some("#>"),
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
            TokenKind::Symbol('.') | TokenKind::DoubleSymbol("::") => {
                scope.on_receiver();
                i += 1;
                continue;
            }
            TokenKind::Ident(ident) => {
                let lower = ident.to_ascii_lowercase();
                match lower.as_str() {
                    "function" | "filter" | "workflow" | "configuration" => {
                        let start_byte = tok.start;
                        let mut j = i + 1;
                        while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                            j += 1;
                        }
                        if j < tokens.len() && matches!(tokens[j].kind, TokenKind::Ident(_)) {
                            let name_start = tokens[j].start;
                            let mut name_end = tokens[j].end;
                            j += 1;
                            while j + 1 < tokens.len()
                                && (tokens[j].kind == TokenKind::Symbol('-')
                                    || tokens[j].kind == TokenKind::Symbol(':'))
                            {
                                if matches!(tokens[j + 1].kind, TokenKind::Ident(_)) {
                                    name_end = tokens[j + 1].end;
                                    j += 2;
                                } else {
                                    break;
                                }
                            }
                            let full_name = &src[name_start as usize..name_end as usize];
                            scope.open_definition_with_body_docs(
                                full_name,
                                start_byte as usize,
                                true,
                                true,
                                &mut facts,
                            );
                            scope.on_word(full_name);
                            i = j;
                            continue;
                        }
                    }
                    "class" | "enum" => {
                        let start_byte = tok.start;
                        let j = i + 1;
                        if j < tokens.len() {
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
                    _ => {
                        let mut j = i + 1;
                        while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                            j += 1;
                        }
                        // Three shapes, and only the first two were covered:
                        // `name(...)`, a hyphenated cmdlet (`Write-Host`), and
                        // a bare command whose arguments follow without
                        // parentheses — `Refuse $Owner`, which is how a script
                        // calls its own functions. Measured on a realistic
                        // script, the third shape is what took the language to
                        // 130 definitions and **zero** edges.
                        // A bare command is an identifier whose next token
                        // starts an argument — `$var`, a string, or a number.
                        // Keying on the line start instead missed
                        // `return Refuse $Owner`, which is how a function
                        // hands back another's result.
                        let bare_command = tokens.get(j).is_some_and(|t| {
                            matches!(
                                &t.kind,
                                TokenKind::Symbol('$')
                                    | TokenKind::StringLit(_)
                                    | TokenKind::Number(_)
                            )
                        });
                        let is_call = (j < tokens.len()
                            && tokens[j].kind == TokenKind::Symbol('('))
                            || ident.contains('-')
                            || bare_command;
                        if is_call
                            && !matches!(
                                lower.as_str(),
                                "if" | "else"
                                    | "elseif"
                                    | "switch"
                                    | "while"
                                    | "for"
                                    | "foreach"
                                    | "do"
                                    | "until"
                                    | "try"
                                    | "catch"
                                    | "finally"
                                    | "trap"
                                    | "return"
                                    | "throw"
                                    | "param"
                                    | "in"
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

pub fn parse_gdscript(src: &str) -> FileFacts {
    let mut facts = FileFacts::new();
    let style = CommentStyle {
        line_comment_prefix: &["#"],
        doc_comment_prefix: &["##"],
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
                i += 1;
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

                if matches!(*ident, "static" | "remote" | "master" | "puppet" | "sync") {
                    i += 1;
                    continue;
                }
                match *ident {
                    "class_name" => {
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
                    "class" => {
                        let start_byte = tok.start;
                        if i + 1 < tokens.len() {
                            if let TokenKind::Ident(name) = tokens[i + 1].kind {
                                scope.close_definitions_at_or_above(
                                    0,
                                    start_byte as usize,
                                    &mut facts,
                                );
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
                    "func" => {
                        let start_byte = tok.start;
                        if i + 1 < tokens.len() {
                            if let TokenKind::Ident(name) = tokens[i + 1].kind {
                                scope.close_definitions_at_or_above(
                                    1,
                                    start_byte as usize,
                                    &mut facts,
                                );
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
                    _ => {
                        let mut j = i + 1;
                        while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                            j += 1;
                        }
                        let is_call = j < tokens.len() && tokens[j].kind == TokenKind::Symbol('(');
                        if is_call
                            && !matches!(
                                *ident,
                                "if" | "elif"
                                    | "else"
                                    | "for"
                                    | "while"
                                    | "match"
                                    | "return"
                                    | "pass"
                                    | "var"
                                    | "const"
                                    | "signal"
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

pub fn parse_batch(src: &str) -> FileFacts {
    let mut facts = FileFacts::new();
    let mut byte_offset = 0;
    let mut current_label: Option<String> = None;
    let mut pending_doc: Option<String> = None;

    for line in src.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("::") || trimmed.to_ascii_uppercase().starts_with("REM ") {
            let comment_text = if trimmed.starts_with("::") {
                trimmed.trim_start_matches(':').trim()
            } else {
                trimmed[4..].trim()
            };
            if !comment_text.is_empty() {
                pending_doc = Some(comment_text.to_string());
            }
        } else if trimmed.starts_with(':') {
            let label = trimmed
                .trim_start_matches(':')
                .split_whitespace()
                .next()
                .unwrap_or("");
            if !label.is_empty() {
                let start = byte_offset + line.find(label).unwrap_or(0);
                facts.add_definition(
                    label,
                    (start as u32, (start + label.len()) as u32),
                    pending_doc.take(),
                );
                current_label = Some(label.to_string());
            }
        } else {
            let upper = trimmed.to_ascii_uppercase();
            if upper.starts_with("CALL :") || upper.starts_with("CALL  :") {
                let parts: Vec<&str> = trimmed.split_whitespace().collect();
                if parts.len() >= 2 {
                    let target = parts[1].trim_start_matches(':');
                    if !target.is_empty() {
                        let caller = current_label
                            .clone()
                            .unwrap_or_else(|| "<batch>".to_string());
                        facts.calls.push((caller, target.to_string(), false));
                    }
                }
            } else if upper.starts_with("CALL ") {
                let parts: Vec<&str> = trimmed.split_whitespace().collect();
                if parts.len() >= 2 {
                    let target = parts[1].trim_start_matches(':');
                    if !target.is_empty() {
                        let caller = current_label
                            .clone()
                            .unwrap_or_else(|| "<batch>".to_string());
                        facts.calls.push((caller, target.to_string(), false));
                    }
                }
            }
        }
        byte_offset += line.len() + 1;
    }

    facts
}

pub fn parse_fish(src: &str) -> FileFacts {
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
            TokenKind::Ident(ident) => {
                if *ident == "function" {
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
                } else if *ident == "end" {
                    scope.on_close_delimiter(tok.end as usize, &mut facts);
                    i += 1;
                    continue;
                } else {
                    // **A call is the first word of a command, never an
                    // argument.** There was no such test: every identifier
                    // became a call, so `$argv`, `$LIMIT` and the `-gt` of a
                    // `test` expression each became an edge — 773 edges per
                    // 1,000 lines on a 22-line fixture, and 24% of references
                    // on `<module>`. The variables are the loudest: a shell
                    // script mentions them on nearly every line.
                    let command_position = tok.start == 0
                        || src
                            .as_bytes()
                            .get(tok.start as usize - 1)
                            .is_some_and(|b| matches!(b, b'\n' | b';' | b'|' | b'&' | b'('))
                        || src[..tok.start as usize]
                            .trim_end_matches([' ', '\t'])
                            .ends_with(['\n', ';', '|', '&', '('])
                        || src[..tok.start as usize].trim().is_empty();
                    let sigil = tok
                        .start
                        .checked_sub(1)
                        .is_some_and(|b| src.as_bytes().get(b as usize) == Some(&b'$'));
                    if command_position
                        && !sigil
                        && !matches!(
                            *ident,
                            "if" | "else"
                                | "switch"
                                | "case"
                                | "while"
                                | "for"
                                | "in"
                                | "begin"
                                | "return"
                                | "exit"
                                | "set"
                                | "test"
                                | "end"
                                | "function"
                        )
                    {
                        scope.record_call(ident, &mut facts);
                    }
                    scope.on_word(ident);
                }
            }
            _ => {}
        }
        i += 1;
    }

    scope.finish(src.len(), &mut facts);
    facts
}
