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

    while i < tokens.len() {
        let tok = &tokens[i];
        let line_start_pos = src[..tok.start as usize]
            .rfind('\n')
            .map(|p| p + 1)
            .unwrap_or(0);
        let current_line_indent = (tok.start as usize).saturating_sub(line_start_pos) as i32;

        match &tok.kind {
            TokenKind::DocComment(text)
            | TokenKind::LineComment(text)
            | TokenKind::BlockComment(text) => {
                while scope
                    .open
                    .last()
                    .is_some_and(|o| o.depth > current_line_indent)
                {
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
                while scope
                    .open
                    .last()
                    .is_some_and(|o| o.depth > current_line_indent)
                {
                    scope.on_close_delimiter(tok.start as usize, &mut facts);
                }

                match *ident {
                    "proc" | "func" | "method" | "iterator" | "template" | "macro"
                    | "converter" => {
                        let start_byte = tok.start;
                        if i + 1 < tokens.len() {
                            if let TokenKind::Ident(raw_name) = tokens[i + 1].kind {
                                let clean_name = raw_name.trim_end_matches('*');
                                scope.open_definition(
                                    clean_name,
                                    start_byte as usize,
                                    true,
                                    &mut facts,
                                );
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
                                scope.open_definition(
                                    clean_name,
                                    start_byte as usize,
                                    false,
                                    &mut facts,
                                );
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
