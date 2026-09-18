//! Category 6: Databases & Schemas
//! Parsers for: SQL, Protocol Buffers (Protobuf)

use crate::doc::clean_doc;
use crate::facts::FileFacts;
use crate::lexer::{CommentStyle, Lexer, TokenKind};
use crate::scope::ScopeStack;

pub fn parse_sql(src: &str) -> FileFacts {
    let mut facts = FileFacts::new();
    let style = CommentStyle {
        line_comment_prefix: &["--"],
        doc_comment_prefix: &["--"],
        block_comment_start: Some("/*"),
        block_comment_end: Some("*/"),
        ident_suffix_marks: false,
        ident_dashes: false,
    };
    let mut lexer = Lexer::new(src, style);
    let tokens = lexer.collect_all_tokens();

    let mut i = 0;
    let mut scope = ScopeStack::new();
    let mut in_block = false;

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
            TokenKind::Symbol(';') => {
                if !in_block && scope.depth > 0 {
                    scope.on_close_delimiter(tok.end as usize, &mut facts);
                }
                i += 1;
                continue;
            }
            TokenKind::Symbol('.') => {
                scope.on_receiver();
                i += 1;
                continue;
            }
            TokenKind::Ident(ident) => {
                if ident.eq_ignore_ascii_case("BEGIN") {
                    scope.depth += 1;
                    in_block = true;
                    i += 1;
                    continue;
                }
                if ident.eq_ignore_ascii_case("END") {
                    if scope.depth > 0 {
                        scope.on_close_delimiter(tok.end as usize, &mut facts);
                    }
                    in_block = false;
                    i += 1;
                    continue;
                }

                if ident.eq_ignore_ascii_case("CREATE") {
                    let start_byte = tok.start;
                    let mut j = i + 1;
                    let mut is_materialized = false;
                    while j < tokens.len() {
                        if let TokenKind::Ident(w) = tokens[j].kind {
                            if w.eq_ignore_ascii_case("OR")
                                || w.eq_ignore_ascii_case("REPLACE")
                                || w.eq_ignore_ascii_case("TEMP")
                                || w.eq_ignore_ascii_case("TEMPORARY")
                                || w.eq_ignore_ascii_case("UNLOGGED")
                                || w.eq_ignore_ascii_case("UNIQUE")
                            {
                                j += 1;
                                continue;
                            }
                            if w.eq_ignore_ascii_case("MATERIALIZED") {
                                is_materialized = true;
                                j += 1;
                                continue;
                            }
                        }
                        break;
                    }

                    if j < tokens.len() {
                        if let TokenKind::Ident(obj_type) = tokens[j].kind {
                            let mut obj_type_upper = obj_type.to_uppercase();
                            if is_materialized && obj_type_upper == "VIEW" {
                                obj_type_upper = "MATERIALIZED_VIEW".to_string();
                            }
                            if matches!(
                                obj_type_upper.as_str(),
                                "TABLE" | "VIEW" | "MATERIALIZED_VIEW" | "FUNCTION" | "PROCEDURE" | "SCHEMA" | "INDEX" | "TRIGGER" | "SEQUENCE" | "TYPE" | "DOMAIN"
                            ) {
                                j += 1;
                                if j + 2 < tokens.len() {
                                    if let (TokenKind::Ident(w1), TokenKind::Ident(w2), TokenKind::Ident(w3)) =
                                        (&tokens[j].kind, &tokens[j + 1].kind, &tokens[j + 2].kind)
                                    {
                                        if w1.eq_ignore_ascii_case("IF")
                                            && w2.eq_ignore_ascii_case("NOT")
                                            && w3.eq_ignore_ascii_case("EXISTS")
                                        {
                                            j += 3;
                                        }
                                    }
                                }
                                if j < tokens.len() {
                                    if let TokenKind::Ident(name) = tokens[j].kind {
                                        let clean_name = name.trim_matches('"').trim_matches('`');
                                        // `CREATE INDEX … ON vets (…)` names the
                                        // table it indexes, and that reference
                                        // belongs to the index rather than to
                                        // the file. It has no braced body, so
                                        // its scope runs to the `;` — the shape
                                        // a constant with an initialiser has.
                                        let statement_scoped = matches!(
                                            obj_type_upper.as_str(),
                                            "INDEX" | "SEQUENCE" | "TYPE" | "DOMAIN" | "SCHEMA"
                                        );
                                        let opens_body = matches!(obj_type_upper.as_str(), "TABLE" | "VIEW" | "MATERIALIZED_VIEW" | "FUNCTION" | "PROCEDURE" | "TRIGGER");
                                        let collects_body_docs = matches!(obj_type_upper.as_str(), "FUNCTION" | "PROCEDURE" | "TRIGGER");
                                        if statement_scoped {
                                            scope.open_statement_definition(clean_name, start_byte as usize, &mut facts);
                                        } else {
                                            scope.open_definition_with_body_docs(clean_name, start_byte as usize, opens_body, collects_body_docs, &mut facts);
                                            if opens_body {
                                                scope.depth += 1;
                                            }
                                        }
                                        scope.on_word(clean_name);
                                        i = j + 1;
                                        continue;
                                    }
                                }
                            }
                        }
                    }
                } else if ident.eq_ignore_ascii_case("FROM")
                    || ident.eq_ignore_ascii_case("JOIN")
                    || ident.eq_ignore_ascii_case("REFERENCES")
                    || ident.eq_ignore_ascii_case("INTO")
                    || ident.eq_ignore_ascii_case("UPDATE")
                {
                    let mut j = i + 1;
                    while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                        j += 1;
                    }
                    if j < tokens.len() {
                        if let TokenKind::Ident(tbl_name) = tokens[j].kind {
                            let upper_tbl = tbl_name.to_uppercase();
                            if !matches!(
                                upper_tbl.as_str(),
                                "SELECT" | "WHERE" | "JOIN" | "INNER" | "LEFT" | "RIGHT" | "FULL" | "OUTER" | "CROSS" | "NATURAL" | "ON" | "LATERAL" | "ONLY" | "SET" | "VALUES"
                            ) {
                                let clean_tbl = tbl_name.trim_matches('"').trim_matches('`');
                                scope.record_call(clean_tbl, &mut facts);
                            }
                        }
                    }
                    scope.on_word(ident);
                } else if ident.eq_ignore_ascii_case("CALL") || ident.eq_ignore_ascii_case("EXEC") || ident.eq_ignore_ascii_case("PERFORM") {
                    let mut j = i + 1;
                    while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                        j += 1;
                    }
                    if j < tokens.len() {
                        if let TokenKind::Ident(proc_name) = tokens[j].kind {
                            let clean_proc = proc_name.trim_matches('"').trim_matches('`');
                            scope.record_call(clean_proc, &mut facts);
                            scope.on_word(clean_proc);
                            i = j + 1;
                            continue;
                        }
                    }
                    scope.on_word(ident);
                } else {
                    let mut j = i + 1;
                    while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                        j += 1;
                    }
                    if j < tokens.len() && tokens[j].kind == TokenKind::Symbol('(') {
                        let upper = ident.to_uppercase();
                        if !matches!(
                            upper.as_str(),
                            "SELECT" | "FROM" | "WHERE" | "INSERT" | "INTO" | "VALUES" | "UPDATE" | "SET" | "DELETE" | "JOIN" | "ON" | "GROUP" | "ORDER" | "BY" | "HAVING" | "AND" | "OR" | "NOT" | "IN" | "EXISTS" | "CREATE" | "TABLE" | "VIEW" | "IF" | "RETURNS" | "PRIMARY" | "KEY" | "FOREIGN" | "REFERENCES" | "CONSTRAINT" | "DEFAULT" | "CHECK" | "UNIQUE" | "NULL" | "CASE" | "WHEN" | "THEN" | "ELSE" | "END" | "CAST"
                        ) {
                            scope.record_call(ident, &mut facts);
                        }
                    }
                    scope.on_word(ident);
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

pub fn parse_protobuf(src: &str) -> FileFacts {
    let mut facts = FileFacts::new();
    let style = CommentStyle {
        line_comment_prefix: &["//"],
        doc_comment_prefix: &["//"],
        block_comment_start: Some("/*"),
        block_comment_end: Some("*/"),
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
                match *ident {
                    "message" | "service" | "enum" | "extend" => {
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
                    "rpc" => {
                        let start_byte = tok.start;
                        if i + 1 < tokens.len() {
                            if let TokenKind::Ident(rpc_name) = tokens[i + 1].kind {
                                scope.open_definition_with_body_docs(rpc_name, start_byte as usize, false, false, &mut facts);
                                scope.on_word(rpc_name);
                                i += 2;
                                continue;
                            }
                        }
                    }
                    "package" => {
                        let start_byte = tok.start;
                        if i + 1 < tokens.len() {
                            if let TokenKind::Ident(pkg_name) = tokens[i + 1].kind {
                                scope.open_definition_with_body_docs(pkg_name, start_byte as usize, false, false, &mut facts);
                                scope.on_word(pkg_name);
                                i += 2;
                                continue;
                            }
                        }
                    }
                    _ => {
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

pub fn parse_graphql(src: &str) -> FileFacts {
    let mut facts = FileFacts::new();
    let style = CommentStyle {
        line_comment_prefix: &["#"],
        doc_comment_prefix: &["#"],
        block_comment_start: Some("\"\"\""),
        block_comment_end: Some("\"\"\""),
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
                match *ident {
                    "type" | "interface" | "union" | "enum" | "input" | "scalar" | "fragment" => {
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
                    "query" | "mutation" | "subscription" => {
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
                    _ => {
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

pub fn parse_thrift(src: &str) -> FileFacts {
    let mut facts = FileFacts::new();
    let style = CommentStyle {
        line_comment_prefix: &["//", "#"],
        doc_comment_prefix: &["/**", "///"],
        block_comment_start: Some("/*"),
        block_comment_end: Some("*/"),
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
                match *ident {
                    "service" | "struct" | "union" | "exception" | "enum" => {
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
                    "typedef" => {
                        let start_byte = tok.start;
                        let mut j = i + 1;
                        while j < tokens.len() && tokens[j].kind != TokenKind::Newline && tokens[j].kind != TokenKind::Symbol(';') {
                            j += 1;
                        }
                        if j > i + 1 {
                            if let TokenKind::Ident(alias) = tokens[j - 1].kind {
                                scope.open_definition_with_body_docs(alias, start_byte as usize, false, false, &mut facts);
                                scope.on_word(alias);
                            }
                        }
                        i = j;
                        continue;
                    }
                    _ => {
                        // Check for Thrift service method: `return_type method_name(...)`
                        let mut j = i + 1;
                        while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                            j += 1;
                        }
                        if j < tokens.len() {
                            if let TokenKind::Ident(method_name) = tokens[j].kind {
                                let mut k = j + 1;
                                while k < tokens.len() && tokens[k].kind == TokenKind::Newline {
                                    k += 1;
                                }
                                if k < tokens.len() && tokens[k].kind == TokenKind::Symbol('(') {
                                    let start_byte = tok.start;
                                    scope.open_definition_with_body_docs(method_name, start_byte as usize, false, false, &mut facts);
                                    scope.on_word(method_name);
                                    // Skip to closing paren
                                    let mut paren_depth = 1;
                                    let mut l = k + 1;
                                    while l < tokens.len() && paren_depth > 0 {
                                        match tokens[l].kind {
                                            TokenKind::Symbol('(') => paren_depth += 1,
                                            TokenKind::Symbol(')') => paren_depth -= 1,
                                            _ => {}
                                        }
                                        l += 1;
                                    }
                                    i = l;
                                    continue;
                                }
                            }
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

pub fn parse_flatbuffers(src: &str) -> FileFacts {
    let mut facts = FileFacts::new();
    let style = CommentStyle {
        line_comment_prefix: &["///", "//"],
        doc_comment_prefix: &["///"],
        block_comment_start: Some("/*"),
        block_comment_end: Some("*/"),
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
                match *ident {
                    "table" | "struct" | "enum" | "union" | "rpc_service" => {
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
                    "namespace" => {
                        let start_byte = tok.start;
                        let mut j = i + 1;
                        let mut ns_name = String::new();
                        while j < tokens.len() && tokens[j].kind != TokenKind::Symbol(';') && tokens[j].kind != TokenKind::Newline {
                            if let TokenKind::Ident(part) = tokens[j].kind {
                                ns_name.push_str(part);
                            } else if let TokenKind::Symbol('.') = tokens[j].kind {
                                ns_name.push('.');
                            }
                            j += 1;
                        }
                        if !ns_name.is_empty() {
                            scope.open_definition_with_body_docs(&ns_name, start_byte as usize, false, false, &mut facts);
                            scope.on_word(&ns_name);
                            i = j;
                            continue;
                        }
                    }
                    _ => {
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

pub fn parse_capnp(src: &str) -> FileFacts {
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
                match *ident {
                    "struct" | "interface" | "enum" | "const" => {
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

pub fn parse_cypher(src: &str) -> FileFacts {
    let mut last_variable: Option<String> = None;
    let mut facts = FileFacts::new();
    let style = CommentStyle {
        line_comment_prefix: &["//"],
        doc_comment_prefix: &["//"],
        block_comment_start: Some("/*"),
        block_comment_end: Some("*/"),
        ident_suffix_marks: false,
        ident_dashes: false,
    };
    let mut lexer = Lexer::new(src, style);
    let tokens = lexer.collect_all_tokens();

    let mut i = 0;
    let mut pending_doc: Option<String> = None;

    while i < tokens.len() {
        let tok = &tokens[i];
        match &tok.kind {
            TokenKind::DocComment(text) | TokenKind::LineComment(text) | TokenKind::BlockComment(text) => {
                let clean = clean_doc(text);
                if !clean.is_empty() {
                    pending_doc = Some(clean);
                }
                i += 1;
                continue;
            }
            TokenKind::Symbol(':') => {
                // Check if inside node pattern (:Label) or [:REL]
                let mut j = i + 1;
                while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                    j += 1;
                }
                if j < tokens.len() {
                    if let TokenKind::Ident(label) = tokens[j].kind {
                        if !facts.defines.contains(&label.to_string()) {
                            facts.add_definition(label, (tokens[j].start, tokens[j].end), pending_doc.take());
                        }
                        i = j + 1;
                        continue;
                    }
                }
            }
            // `(a)-[:CALLS]->(b)` is the only edge this language expresses,
            // and the scanner recorded none: 273 definitions per 1,000 lines
            // and **zero** edges, so a graph query about relationships carried
            // no relationship. The arrow may point either way; the variable
            // before it is the source, the one after the target.
            TokenKind::DoubleSymbol("->") | TokenKind::DoubleSymbol("<-") => {
                if let Some(TokenKind::Ident(target)) = tokens
                    .get(i + 1)
                    .filter(|t| t.kind == TokenKind::Symbol('('))
                    .and(tokens.get(i + 2).map(|t| &t.kind))
                {
                    if let Some(src_name) = last_variable.clone() {
                        facts.add_call(src_name, (*target).to_string(), false);
                    }
                }
                i += 1;
                continue;
            }
            TokenKind::Ident(ident) => {
                let upper = ident.to_ascii_uppercase();
                // Remember the most recent variable inside a pattern, which is
                // what an arrow leaving it refers back to.
                if i > 0 && tokens[i - 1].kind == TokenKind::Symbol('(') {
                    last_variable = Some((*ident).to_string());
                }
                if upper == "CREATE" || upper == "MERGE" {
                    if i + 1 < tokens.len() {
                        if let TokenKind::Ident(clause) = tokens[i + 1].kind {
                            let c_upper = clause.to_ascii_uppercase();
                            if matches!(c_upper.as_str(), "INDEX" | "CONSTRAINT") && i + 2 < tokens.len() {
                                if let TokenKind::Ident(name) = tokens[i + 2].kind {
                                    if !facts.defines.contains(&name.to_string()) {
                                        facts.add_definition(name, (tokens[i + 2].start, tokens[i + 2].end), pending_doc.take());
                                    }
                                    i += 3;
                                    continue;
                                }
                            }
                        }
                    }
                } else if upper == "CALL" && i + 1 < tokens.len() {
                    let mut j = i + 1;
                    while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                        j += 1;
                    }
                    if j < tokens.len() {
                        if let TokenKind::Ident(proc_name) = tokens[j].kind {
                            facts.calls.push(("<query>".to_string(), proc_name.to_string(), false));
                        }
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }

    facts
}
