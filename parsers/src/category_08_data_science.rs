//! Category 8: Data Analysis & Scientific
//! Parsers for: R, Julia, MATLAB, Mojo, Fortran

use crate::facts::FileFacts;
use crate::lexer::{CommentStyle, Lexer, TokenKind};
use crate::scope::ScopeStack;

pub fn parse_r(src: &str) -> FileFacts {
    let mut facts = FileFacts::new();
    let style = CommentStyle {
        line_comment_prefix: &["#'", "#"],
        doc_comment_prefix: &["#'"],
        block_comment_start: None,
        block_comment_end: None,
        ident_suffix_marks: false,
        ident_dashes: false,
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
            TokenKind::Symbol('$') | TokenKind::DoubleSymbol("::") => {
                scope.on_receiver();
                i += 1;
                continue;
            }
            TokenKind::Ident(ident) => {
                let mut j = i + 1;
                while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                    j += 1;
                }
                if j < tokens.len() {
                    let is_assign = matches!(&tokens[j].kind, TokenKind::DoubleSymbol("<-") | TokenKind::Symbol('='));
                    if is_assign {
                        let mut k = j + 1;
                        while k < tokens.len() && tokens[k].kind == TokenKind::Newline {
                            k += 1;
                        }
                        if k < tokens.len() && tokens[k].kind == TokenKind::Ident("function") {
                            let start_byte = tok.start;
                            scope.open_definition(*ident, start_byte as usize, true, &mut facts);
                            scope.on_word(ident);
                            i = k + 1;
                            continue;
                        }
                    }
                }

                // Check if this is a function call: `name(`
                let mut j = i + 1;
                while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                    j += 1;
                }
                if j < tokens.len()
                    && tokens[j].kind == TokenKind::Symbol('(')
                    && !matches!(*ident, "if" | "else" | "for" | "while" | "repeat" | "function" | "return" | "break" | "next")
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

pub fn parse_julia(src: &str) -> FileFacts {
    let mut facts = FileFacts::new();
    let style = CommentStyle {
        line_comment_prefix: &["#"],
        doc_comment_prefix: &["#"],
        block_comment_start: Some("#="),
        block_comment_end: Some("=#"),
        ident_suffix_marks: false,
        ident_dashes: false,
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
            TokenKind::StringLit(s) => {
                if s.starts_with("\"\"\"") {
                    scope.push_comment(s);
                }
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
            TokenKind::Ident("mutable") | TokenKind::Ident("abstract") | TokenKind::Ident("primitive") => {
                i += 1;
                continue;
            }
            TokenKind::Ident(ident) => {
                // `pay(cart) = charge(cart.total)` is Julia's short form, and
                // it is as common as `function … end`: measured on 200 files of
                // the language's own base library, 3,939 against 4,037. Without
                // it half the definitions are missing and their bodies' calls
                // land on `<module>` — that rate read 59% before this.
                //
                // The test is a name, a balanced parenthesis group, then a
                // single `=`. `==` is a comparison and `(a, b) = f()` is
                // destructuring, so both are left alone.
                if scope.depth == 0 && i + 1 < tokens.len() && tokens[i + 1].kind == TokenKind::Symbol('(') {
                    let mut k = i + 1;
                    let mut paren = 0i32;
                    while k < tokens.len() {
                        match tokens[k].kind {
                            TokenKind::Symbol('(') => paren += 1,
                            TokenKind::Symbol(')') => {
                                paren -= 1;
                                if paren == 0 { break; }
                            }
                            TokenKind::Newline => break,
                            _ => {}
                        }
                        k += 1;
                    }
                    if paren == 0
                        && k + 1 < tokens.len()
                        && tokens[k + 1].kind == TokenKind::Symbol('=')
                        && !matches!(tokens.get(k + 2).map(|t| &t.kind), Some(TokenKind::Symbol('=')))
                    {
                        scope.open_statement_definition(*ident, tok.start as usize, &mut facts);
                        scope.on_word(ident);
                        i = k + 2;
                        continue;
                    }
                }

                match *ident {
                    "function" | "macro" => {
                        let start_byte = tok.start;
                        if i + 1 < tokens.len() {
                            if let TokenKind::Ident(fn_name) = tokens[i + 1].kind {
                                scope.open_definition(fn_name, start_byte as usize, true, &mut facts);
                                // The body opens a level its `end` closes.
                                // `open_definition` records the depth it was
                                // called at and does not raise it, so without
                                // this the first `end` *inside* the body — an
                                // `if`'s — closed the function and every later
                                // call fell to `<module>`.
                                scope.on_open_delimiter();
                                if let Some(last) = scope.open.last_mut() {
                                    last.depth = scope.depth - 1;
                                }
                                if let Some(last) = scope.enclosing.last_mut() {
                                    last.1 = scope.depth - 1;
                                }
                                scope.on_word(fn_name);
                                i += 2;
                                continue;
                            }
                        }
                    }
                    "struct" | "module" | "type" | "const" => {
                        let start_byte = tok.start;
                        if i + 1 < tokens.len() {
                            if let TokenKind::Ident(name) = tokens[i + 1].kind {
                                let opens_body = matches!(*ident, "struct" | "module");
                                scope.open_definition(name, start_byte as usize, opens_body, &mut facts);
                                scope.on_word(name);
                                i += 2;
                                continue;
                            }
                        }
                    }
                    // Every one of these is closed by `end` in Julia; there
                    // is no one-line form that omits it.
                    "if" | "for" | "while" | "try" | "begin" | "let" | "quote" | "do" => {
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
                            && tokens[j].kind == TokenKind::Symbol('(')
                            && !matches!(*ident, "if" | "elseif" | "else" | "while" | "for" | "return" | "begin" | "try" | "catch" | "finally")
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

pub fn parse_matlab(src: &str) -> FileFacts {
    let mut facts = FileFacts::new();
    let style = CommentStyle {
        line_comment_prefix: &["%%", "%"],
        doc_comment_prefix: &["%%"],
        block_comment_start: Some("%{"),
        block_comment_end: Some("%}"),
        ident_suffix_marks: false,
        ident_dashes: false,
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
                    "function" => {
                        let start_byte = tok.start;
                        let mut j = i + 1;
                        while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                            j += 1;
                        }
                        let mut fn_name = None;
                        let mut equals_pos = None;
                        let mut k = j;
                        while k < tokens.len() && tokens[k].kind != TokenKind::Newline {
                            if tokens[k].kind == TokenKind::Symbol('=') {
                                equals_pos = Some(k);
                                break;
                            }
                            k += 1;
                        }
                        if let Some(eq) = equals_pos {
                            let mut after_eq = eq + 1;
                            while after_eq < tokens.len() && tokens[after_eq].kind == TokenKind::Newline {
                                after_eq += 1;
                            }
                            if after_eq < tokens.len() {
                                if let TokenKind::Ident(name) = tokens[after_eq].kind {
                                    fn_name = Some((name, after_eq));
                                }
                            }
                        } else if j < tokens.len() {
                            if let TokenKind::Ident(name) = tokens[j].kind {
                                fn_name = Some((name, j));
                            }
                        }
                        if let Some((name, name_idx)) = fn_name {
                            scope.on_open_delimiter();
                            scope.open_definition_with_body_docs(name, start_byte as usize, true, true, &mut facts);
                            scope.on_word(name);
                            i = name_idx + 1;
                            continue;
                        }
                    }
                    "classdef" => {
                        let start_byte = tok.start;
                        let mut j = i + 1;
                        if j < tokens.len() && tokens[j].kind == TokenKind::Symbol('(') {
                            j += 1;
                            while j < tokens.len() && tokens[j].kind != TokenKind::Symbol(')') {
                                j += 1;
                            }
                            if j < tokens.len() {
                                j += 1;
                            }
                        }
                        while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                            j += 1;
                        }
                        if j < tokens.len() {
                            if let TokenKind::Ident(name) = tokens[j].kind {
                                scope.on_open_delimiter();
                                scope.open_definition_with_body_docs(name, start_byte as usize, true, false, &mut facts);
                                scope.on_word(name);
                                i = j + 1;
                                continue;
                            }
                        }
                    }
                    "methods" | "properties" | "events" | "if" | "for" | "while" | "switch" | "try" | "parfor" | "spmd" => {
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
                            && tokens[j].kind == TokenKind::Symbol('(')
                            && !matches!(*ident, "if" | "elseif" | "else" | "while" | "for" | "return" | "break" | "continue" | "switch" | "case" | "otherwise" | "try" | "catch")
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

pub fn parse_mojo(src: &str) -> FileFacts {
    let mut facts = FileFacts::new();
    let style = CommentStyle {
        line_comment_prefix: &["#"],
        doc_comment_prefix: &["#"],
        block_comment_start: None,
        block_comment_end: None,
        ident_suffix_marks: false,
        ident_dashes: false,
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
            TokenKind::DocComment(text) | TokenKind::LineComment(text) => {
                while scope.open.last().is_some_and(|o| o.depth > current_line_indent) {
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
            TokenKind::StringLit(s) => {
                if s.starts_with("\"\"\"") || s.starts_with("'''") {
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
                while scope.open.last().is_some_and(|o| o.depth > current_line_indent) {
                    scope.on_close_delimiter(tok.start as usize, &mut facts);
                }
                scope.depth = current_line_indent;

                match *ident {
                    "fn" | "def" => {
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
                    "struct" | "trait" => {
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
                    "alias" => {
                        let start_byte = tok.start;
                        let mut j = i + 1;
                        while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                            j += 1;
                        }
                        if j < tokens.len() {
                            if let TokenKind::Ident(name) = tokens[j].kind {
                                scope.open_definition_with_body_docs(name, start_byte as usize, false, false, &mut facts);
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
                        if j < tokens.len()
                            && tokens[j].kind == TokenKind::Symbol('(')
                            && !matches!(*ident, "if" | "elif" | "else" | "while" | "for" | "return" | "raise" | "with" | "try" | "except" | "finally" | "var" | "let")
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

pub fn parse_fortran(src: &str) -> FileFacts {
    let mut facts = FileFacts::new();
    let style = CommentStyle {
        line_comment_prefix: &["!"],
        doc_comment_prefix: &["!"],
        block_comment_start: None,
        block_comment_end: None,
        ident_suffix_marks: false,
        ident_dashes: false,
    };
    let mut lexer = Lexer::new(src, style);
    let tokens = lexer.collect_all_tokens();

    let mut i = 0;
    let mut scope = ScopeStack::new();
    // Fortran closes with `end function charge`, repeating both keyword and
    // name; without this the closing line opened a second definition of the
    // same symbol — measured on the fixture, every one appeared twice.
    let mut after_end = false;

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
            TokenKind::Symbol('%') | TokenKind::Symbol('.') => {
                // Fortran uses % for derived type component selection, e.g. self%method()
                scope.on_receiver();
                i += 1;
                continue;
            }
            TokenKind::Ident(ident) => {
                let after_end_now = std::mem::take(&mut after_end);
                let id_lower = ident.to_ascii_lowercase();
                match id_lower.as_str() {
                    "subroutine" | "function" | "module" | "program" | "type"
                        if after_end_now =>
                    {
                        i += 1;
                        if matches!(tokens.get(i).map(|t| &t.kind), Some(TokenKind::Ident(_))) {
                            i += 1;
                        }
                        continue;
                    }
                    "subroutine" => {
                        let start_byte = tok.start;
                        let mut j = i + 1;
                        while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                            j += 1;
                        }
                        if j < tokens.len() {
                            if let TokenKind::Ident(name) = tokens[j].kind {
                                scope.on_open_delimiter();
                                scope.open_definition_with_body_docs(name, start_byte as usize, true, true, &mut facts);
                                // The definition belongs one level below the
                                // paren that opened its body, or an inner
                                // `end if` — which drops back to exactly this
                                // depth — closes the function with it.
                                if let Some(last) = scope.open.last_mut() {
                                    last.depth = scope.depth - 1;
                                }
                                if let Some(last) = scope.enclosing.last_mut() {
                                    last.1 = scope.depth - 1;
                                }
                                scope.on_word(name);
                                i = j + 1;
                                continue;
                            }
                        }
                    }
                    "function" => {
                        let start_byte = tok.start;
                        let mut j = i + 1;
                        while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                            j += 1;
                        }
                        if j < tokens.len() {
                            if let TokenKind::Ident(name) = tokens[j].kind {
                                scope.on_open_delimiter();
                                scope.open_definition_with_body_docs(name, start_byte as usize, true, true, &mut facts);
                                // The definition belongs one level below the
                                // paren that opened its body, or an inner
                                // `end if` — which drops back to exactly this
                                // depth — closes the function with it.
                                if let Some(last) = scope.open.last_mut() {
                                    last.depth = scope.depth - 1;
                                }
                                if let Some(last) = scope.enclosing.last_mut() {
                                    last.1 = scope.depth - 1;
                                }
                                scope.on_word(name);
                                i = j + 1;
                                continue;
                            }
                        }
                    }
                    "program" | "module" => {
                        let start_byte = tok.start;
                        let mut j = i + 1;
                        while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                            j += 1;
                        }
                        if j < tokens.len() {
                            if let TokenKind::Ident(name) = tokens[j].kind {
                                if !name.eq_ignore_ascii_case("procedure") {
                                    scope.on_open_delimiter();
                                    scope.open_definition_with_body_docs(name, start_byte as usize, true, false, &mut facts);
                                    scope.on_word(name);
                                    i = j + 1;
                                    continue;
                                }
                            }
                        }
                    }
                    "call" => {
                        let mut j = i + 1;
                        while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                            j += 1;
                        }
                        if j < tokens.len() {
                            if let TokenKind::Ident(callee) = tokens[j].kind {
                                scope.record_call(callee, &mut facts);
                                scope.on_word(callee);
                                i = j + 1;
                                continue;
                            }
                        }
                    }
                    // A block `if` opens the level its `end if` closes; the
                    // one-line `if (x) return` has no `end` and must not.
                    "if" if fortran_opens_block(&tokens, i + 1) => {
                        scope.on_open_delimiter();
                    }
                    "do" | "select" | "associate" => {
                        scope.on_open_delimiter();
                    }
                    "where" if fortran_opens_block(&tokens, i + 1) => {
                        scope.on_open_delimiter();
                    }
                    "end" => {
                        scope.on_close_delimiter(tok.end as usize, &mut facts);
                        after_end = true;
                        i += 1;
                        continue;
                    }
                    _ => {
                        if id_lower.starts_with("end") && id_lower.len() > 3 {
                            scope.on_close_delimiter(tok.end as usize, &mut facts);
                        } else {
                            let mut j = i + 1;
                            while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                                j += 1;
                            }
                            if j < tokens.len()
                                && tokens[j].kind == TokenKind::Symbol('(')
                                && !matches!(id_lower.as_str(), "if" | "then" | "else" | "elseif" | "do" | "select" | "case" | "where" | "return" | "write" | "read" | "print" | "allocate" | "deallocate")
                            {
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

/// Whether `then` closes this line — what separates Fortran's block
/// `if (…) then … end if` from the one-line `if (…) return`.
fn fortran_opens_block(tokens: &[crate::lexer::Token<'_>], from: usize) -> bool {
    tokens[from..]
        .iter()
        .take_while(|t| t.kind != TokenKind::Newline)
        .any(|t| matches!(t.kind, TokenKind::Ident(w) if w.eq_ignore_ascii_case("then")))
}
