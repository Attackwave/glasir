//! Category 4: DevOps, Cloud & Configuration
//! Parsers for: HCL/Terraform, YAML, TOML, Dockerfile

use crate::doc::clean_doc;
use crate::facts::FileFacts;
use crate::lexer::{CommentStyle, Lexer, TokenKind};
use crate::scope::ScopeStack;

pub fn parse_hcl_terraform(src: &str) -> FileFacts {
    let mut facts = FileFacts::new();
    let style = CommentStyle {
        line_comment_prefix: &["#", "//"],
        doc_comment_prefix: &["#", "//"],
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
            TokenKind::Symbol('.') => {
                scope.on_receiver();
                i += 1;
                continue;
            }
            TokenKind::Ident(block_type) => {
                match *block_type {
                    // Counted in Terraform's own tree rather than guessed:
                    // `resource` 1,323, `data` 79, `action` 14, `ephemeral`
                    // 10. The last two were missing, so their blocks yielded
                    // no definition at all. `invalid`, `varyable` and `resorce`
                    // also appear there and are error fixtures, not block
                    // types.
                    "resource" | "data" | "action" | "ephemeral" => {
                        let start_byte = tok.start;
                        if i + 2 < tokens.len() {
                            if let (TokenKind::StringLit(_t), TokenKind::StringLit(n)) =
                                (&tokens[i + 1].kind, &tokens[i + 2].kind)
                            {
                                let clean_name = n.trim_matches('"');
                                scope.open_definition_with_body_docs(
                                    clean_name,
                                    start_byte as usize,
                                    true,
                                    false,
                                    &mut facts,
                                );
                                scope.on_word(clean_name);
                                i += 3;
                                continue;
                            }
                        }
                    }
                    // Counted in Terraform's own tree: `.tftest.hcl` and
                    // `.tfcomponent.hcl` carry block types the `.tf` grammar
                    // does not — `run` 410, `component` 193, `stack` 35,
                    // `mock_provider` 32. Without them a test file yielded no
                    // definition and every assertion in it was attributed to
                    // the file: 1,318 of 1,703 references across 555 `.hcl`
                    // files.
                    "module" | "variable" | "output" | "provider" | "check" | "run"
                    | "component" | "stack" | "mock_provider" => {
                        let start_byte = tok.start;
                        if i + 1 < tokens.len() {
                            if let TokenKind::StringLit(n) = &tokens[i + 1].kind {
                                let clean_name = n.trim_matches('"');
                                scope.open_definition_with_body_docs(
                                    clean_name,
                                    start_byte as usize,
                                    true,
                                    false,
                                    &mut facts,
                                );
                                scope.on_word(clean_name);
                                i += 2;
                                continue;
                            }
                        }
                    }
                    // A `locals { … }` block has no label, so it is named
                    // after the keyword — but its body still has to open a
                    // level, or the assignments inside it are attributed to
                    // the file. Measured on Terraform's own tree, `locals`
                    // alone accounts for most of the `<module>` share.
                    "locals" | "terraform" | "import" | "moved" | "removed" => {
                        let start_byte = tok.start;
                        scope.open_definition_with_body_docs(
                            *block_type,
                            start_byte as usize,
                            true,
                            false,
                            &mut facts,
                        );
                        // The `{` that follows is counted by the delimiter arm
                        // above, *after* this definition recorded its depth —
                        // so the matching `}` drops back to exactly that value
                        // and closes the block. Recording it one level lower
                        // is what keeps the body inside. Same shape as Lua,
                        // Fortran and Julia; fourth occurrence.
                        if let Some(last) = scope.open.last_mut() {
                            last.depth = scope.depth.saturating_sub(1);
                        }
                        if let Some(last) = scope.enclosing.last_mut() {
                            last.1 = scope.depth.saturating_sub(1);
                        }
                        scope.on_word(block_type);
                    }
                    _ => {
                        if scope.had_receiver {
                            scope.record_call(block_type, &mut facts);
                            scope.on_word(block_type);
                            i += 1;
                            continue;
                        }

                        let mut j = i + 1;
                        while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                            j += 1;
                        }
                        if j < tokens.len() && tokens[j].kind == TokenKind::Symbol('(') {
                            scope.record_call(block_type, &mut facts);
                        }
                        scope.on_word(block_type);
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

pub fn parse_yaml(src: &str) -> FileFacts {
    let mut facts = FileFacts::new();
    let mut byte_offset = 0;
    let mut pending_comments = Vec::new();

    for line in src.lines() {
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();

        if trimmed.starts_with('#') {
            let clean = clean_doc(trimmed);
            if !clean.is_empty() {
                pending_comments.push(clean);
            }
        } else if !trimmed.is_empty() {
            if let Some(colon_pos) = trimmed.find(':') {
                let key = trimmed[..colon_pos].trim();
                if !key.starts_with('-') && !key.is_empty() {
                    let start_byte = byte_offset + indent;
                    let end_byte = start_byte + key.len();
                    let doc = if !pending_comments.is_empty() {
                        let d = pending_comments.join(" ");
                        pending_comments.clear();
                        let cleaned = clean_doc(&d);
                        if cleaned.is_empty() {
                            None
                        } else {
                            Some(cleaned)
                        }
                    } else {
                        None
                    };
                    facts.add_definition(key, (start_byte as u32, end_byte as u32), doc);
                }
            }
        }
        byte_offset += line.len() + 1;
    }

    facts
}

pub fn parse_toml(src: &str) -> FileFacts {
    let mut facts = FileFacts::new();
    let mut byte_offset = 0;
    let mut pending_comments = Vec::new();

    for line in src.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            let clean = clean_doc(trimmed);
            if !clean.is_empty() {
                pending_comments.push(clean);
            }
        } else if !trimmed.is_empty() {
            let doc = if !pending_comments.is_empty() {
                let d = pending_comments.join(" ");
                pending_comments.clear();
                let cleaned = clean_doc(&d);
                if cleaned.is_empty() {
                    None
                } else {
                    Some(cleaned)
                }
            } else {
                None
            };

            if trimmed.starts_with('[') && trimmed.ends_with(']') {
                let section = trimmed[1..trimmed.len() - 1].trim();
                let start_byte = byte_offset + (line.find('[').unwrap_or(0));
                let end_byte = start_byte + section.len() + 2;
                facts.add_definition(section, (start_byte as u32, end_byte as u32), doc);
            } else if let Some(eq_pos) = trimmed.find('=') {
                let key = trimmed[..eq_pos].trim();
                let start_byte = byte_offset + (line.find(key).unwrap_or(0));
                let end_byte = start_byte + key.len();
                facts.add_definition(key, (start_byte as u32, end_byte as u32), doc);
            }
        }
        byte_offset += line.len() + 1;
    }

    facts
}

pub fn parse_dockerfile(src: &str) -> FileFacts {
    let mut facts = FileFacts::new();
    let mut byte_offset = 0;
    let mut pending_comments = Vec::new();

    for line in src.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            let clean = clean_doc(trimmed);
            if !clean.is_empty() {
                pending_comments.push(clean);
            }
        } else if !trimmed.is_empty() {
            let doc = if !pending_comments.is_empty() {
                let d = pending_comments.join(" ");
                pending_comments.clear();
                let cleaned = clean_doc(&d);
                if cleaned.is_empty() {
                    None
                } else {
                    Some(cleaned)
                }
            } else {
                None
            };

            let upper = trimmed.to_uppercase();
            if upper.starts_with("FROM ") {
                let parts: Vec<&str> = trimmed.split_whitespace().collect();
                if let Some(as_idx) = parts.iter().position(|&p| p.eq_ignore_ascii_case("AS")) {
                    if as_idx + 1 < parts.len() {
                        let stage_name = parts[as_idx + 1];
                        let start_byte = byte_offset + (line.find(stage_name).unwrap_or(0));
                        let end_byte = start_byte + stage_name.len();
                        facts.add_definition(stage_name, (start_byte as u32, end_byte as u32), doc);
                        // `FROM refuse AS commit_entry` builds one stage on
                        // another, which is the only dependency a Dockerfile
                        // expresses — and the scanner recorded none of them:
                        // 273 definitions per 1,000 lines and **zero** edges,
                        // so a multi-stage build looked like unrelated images.
                        if parts.len() >= 2 && as_idx >= 1 {
                            let base = parts[1];
                            if !base.contains(':') && !base.contains('/') {
                                facts.add_call(stage_name, base, false);
                            }
                        }
                    }
                } else if parts.len() >= 2 {
                    let image_name = parts[1];
                    let start_byte = byte_offset + (line.find(image_name).unwrap_or(0));
                    let end_byte = start_byte + image_name.len();
                    facts.add_definition(image_name, (start_byte as u32, end_byte as u32), doc);
                }
            }
        }
        byte_offset += line.len() + 1;
    }

    facts
}

pub fn parse_nix(src: &str) -> FileFacts {
    let mut facts = FileFacts::new();
    let style = CommentStyle {
        line_comment_prefix: &["#"],
        doc_comment_prefix: &["#"],
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
                if *ident == "let" || *ident == "rec" {
                    i += 1;
                    continue;
                }

                // Check for attribute binding: name = ...
                if i + 1 < tokens.len() && tokens[i + 1].kind == TokenKind::Symbol('=') {
                    let start_byte = tok.start;
                    if !matches!(
                        *ident,
                        "in" | "if" | "then" | "else" | "with" | "inherit" | "let" | "rec"
                    ) {
                        scope.open_definition_with_body_docs(
                            *ident,
                            start_byte as usize,
                            true,
                            false,
                            &mut facts,
                        );
                        scope.on_word(ident);
                        i += 2;
                        continue;
                    }
                }

                // Nix applies a function by juxtaposition like an ML —
                // `warnOwner owner` — so recording only receiver-qualified
                // names measured 500 definitions and **zero** edges on a
                // realistic expression.
                let applied = tokens.get(i + 1).is_some_and(|t| {
                    matches!(
                        &t.kind,
                        TokenKind::Ident(_) | TokenKind::StringLit(_) | TokenKind::Symbol('(')
                    )
                });
                if (scope.had_receiver || applied)
                    && !matches!(
                        *ident,
                        "in" | "if" | "then" | "else" | "with" | "inherit" | "let" | "rec"
                    )
                {
                    scope.record_call(ident, &mut facts);
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

pub fn parse_json(src: &str) -> FileFacts {
    let mut facts = FileFacts::new();
    let style = CommentStyle {
        line_comment_prefix: &["//"],
        doc_comment_prefix: &["//", "/**"],
        block_comment_start: Some("/*"),
        block_comment_end: Some("*/"),
        ident_suffix_marks: false,
        ident_dashes: false,
        raw_escapes: false,
    };
    let mut lexer = Lexer::new(src, style);
    let tokens = lexer.collect_all_tokens();

    let mut i = 0;
    let mut pending_doc: Option<String> = None;

    while i < tokens.len() {
        let tok = &tokens[i];
        match &tok.kind {
            TokenKind::DocComment(text)
            | TokenKind::LineComment(text)
            | TokenKind::BlockComment(text) => {
                let clean = clean_doc(text);
                if !clean.is_empty() {
                    pending_doc = Some(clean);
                }
                i += 1;
                continue;
            }
            TokenKind::StringLit(key_raw) => {
                let key = key_raw.trim_matches('"').trim_matches('\'');
                if i + 1 < tokens.len() && tokens[i + 1].kind == TokenKind::Symbol(':') {
                    facts.add_definition(key, (tok.start, tok.end), pending_doc.take());
                    i += 2;
                    continue;
                }
            }
            _ => {}
        }
        i += 1;
    }

    facts
}
