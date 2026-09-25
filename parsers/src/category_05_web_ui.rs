//! Category 5: Web & Declarative UI
//! Parsers for: HTML, CSS/SCSS, Dart

use crate::doc::clean_doc;
use crate::facts::FileFacts;
use crate::lexer::{CommentStyle, Lexer, TokenKind};
use crate::scope::ScopeStack;

pub fn parse_html(src: &str) -> FileFacts {
    let mut current_element: Option<String> = None;
    let mut facts = FileFacts::new();
    let mut i = 0;
    let bytes = src.as_bytes();
    let mut pending_comments = Vec::new();

    while i < bytes.len() {
        if bytes[i..].starts_with(b"<!--") {
            let start = i;
            if let Some(end_rel) = src[start..].find("-->") {
                let comment_text = &src[start..start + end_rel + 3];
                let clean = clean_doc(comment_text);
                if !clean.is_empty() {
                    pending_comments.push(clean);
                }
                i = start + end_rel + 3;
                continue;
            }
        }

        if bytes[i] == b'<' && i + 1 < bytes.len() && bytes[i + 1] != b'/' && bytes[i + 1] != b'!' {
            let tag_start = i;
            if let Some(tag_end_rel) = src[tag_start..].find('>') {
                let tag_str = &src[tag_start..=tag_start + tag_end_rel];
                let tag_name = tag_str[1..]
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .trim_end_matches('>');
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

                // **Two passes over the tag, because attribute order is
                // arbitrary**: `<button class="x" id="y">` and
                // `<button id="y" class="x">` are the same element, and a
                // single pass would attribute the classes of the first to
                // nothing. Identity first, references second.
                for attr in &["id=", "name=", "class=", "src=", "href="] {
                    if let Some(pos) = tag_str.find(attr) {
                        let val_start = pos + attr.len();
                        let Some(rest) = tag_str.get(val_start..) else {
                            continue;
                        };
                        let quote = rest.chars().next().unwrap_or(' ');
                        if quote == '"' || quote == '\'' {
                            if let Some(end_quote) = rest.get(1..).and_then(|r| r.find(quote)) {
                                let Some(val) = rest.get(1..=end_quote) else {
                                    continue;
                                };
                                let abs_start = tag_start + val_start + 1;
                                let abs_end = abs_start + val.len();
                                match *attr {
                                    // **`class` is a reference, not a
                                    // definition**, and it is what answers
                                    // "what colour is this button": the rule
                                    // lives in a stylesheet, the element only
                                    // names it. One attribute may list
                                    // several.
                                    "class=" => {
                                        let owner = current_element
                                            .clone()
                                            .unwrap_or_else(|| format!("<{tag_name}>"));
                                        for class in val.split_whitespace() {
                                            facts.add_call(
                                                owner.clone(),
                                                format!(".{class}"),
                                                false,
                                            );
                                        }
                                    }
                                    // `src`/`href` reach another file, which
                                    // is the other half of what a page is
                                    // wired to.
                                    "src=" | "href=" => {
                                        if !val.starts_with("http") && !val.starts_with('#') {
                                            let owner = current_element
                                                .clone()
                                                .unwrap_or_else(|| format!("<{tag_name}>"));
                                            facts.add_call(owner, val.to_string(), false);
                                        }
                                    }
                                    _ => {
                                        let sym = if *attr == "id=" {
                                            format!("id:{val}")
                                        } else {
                                            format!("{tag_name}:{val}")
                                        };
                                        facts.add_definition(
                                            sym.clone(),
                                            (abs_start as u32, abs_end as u32),
                                            doc.clone(),
                                        );
                                        current_element = Some(sym);
                                    }
                                }
                            }
                        }
                    }
                }
                // An element without an id or name still owns its classes:
                // attribute order in the tag decides nothing, so the element
                // is reset only once the whole tag is read.
                current_element = None;
                i = tag_start + tag_end_rel + 1;
                continue;
            }
        }
        i += 1;
    }

    facts
}

pub fn parse_css(src: &str) -> FileFacts {
    let mut facts = FileFacts::new();
    let mut i = 0;
    let bytes = src.as_bytes();
    let mut pending_comments = Vec::new();

    while i < bytes.len() {
        if bytes[i..].starts_with(b"/*") {
            let start = i;
            if let Some(end_rel) = src[start..].find("*/") {
                let comment_text = &src[start..start + end_rel + 2];
                let clean = clean_doc(comment_text);
                if !clean.is_empty() {
                    pending_comments.push(clean);
                }
                i = start + end_rel + 2;
                continue;
            }
        }

        if bytes[i] == b'.' || bytes[i] == b'#' || bytes[i] == b'@' || bytes[i..].starts_with(b"--")
        {
            let start = i;
            let mut j = i;
            while j < bytes.len()
                && (bytes[j].is_ascii_alphanumeric()
                    || bytes[j] == b'-'
                    || bytes[j] == b'_'
                    || bytes[j] == b'.'
                    || bytes[j] == b'#'
                    || bytes[j] == b'@')
            {
                j += 1;
            }
            if j > start {
                let sym = &src[start..j];
                let mut k = j;
                while k < bytes.len() && bytes[k].is_ascii_whitespace() {
                    k += 1;
                }
                if k < bytes.len() && (bytes[k] == b'{' || bytes[k] == b':' || bytes[k] == b',') {
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
                    // **The range covers the whole rule, not just the
                    // selector.** `get_code_snippet` hands this back, and a
                    // snippet reading `.btn-primary` alone answers nothing —
                    // the declarations inside the braces are what the question
                    // "what colour is this button" is about.
                    let mut depth = 0usize;
                    let mut end = j;
                    for (off, ch) in src[j..].char_indices() {
                        match ch {
                            '{' => depth += 1,
                            // A close before any open ends an enclosing block:
                            // this was a declaration, not a rule. Subtracting
                            // anyway underflowed — a panic in a debug build, a
                            // range that never closed in a release one.
                            '}' if depth == 0 => break,
                            '}' => {
                                depth -= 1;
                                if depth == 0 {
                                    end = j + off + 1;
                                    break;
                                }
                            }
                            _ => {}
                        }
                    }
                    facts.add_definition(sym, (start as u32, end as u32), doc);
                    i = j;
                    continue;
                }
            }
        }
        i += 1;
    }

    facts
}

pub fn parse_dart(src: &str) -> FileFacts {
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
                scope.on_statement_end(tok.end as usize, &mut facts);
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
                            }
                        }

                        if is_fn_def
                            && !matches!(
                                *ident,
                                "if" | "while" | "for" | "switch" | "catch" | "assert"
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
                            scope.on_word(ident);
                            i += 1;
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
                                if k < tokens.len() && tokens[k].kind == TokenKind::Symbol('(') {
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
                                    | "switch"
                                    | "case"
                                    | "default"
                                    | "catch"
                                    | "on"
                                    | "finally"
                                    | "try"
                                    | "throw"
                                    | "rethrow"
                                    | "return"
                                    | "break"
                                    | "continue"
                                    | "yield"
                                    | "await"
                                    | "assert"
                                    | "this"
                                    | "super"
                                    | "class"
                                    | "mixin"
                                    | "extension"
                                    | "enum"
                                    | "typedef"
                                    | "library"
                                    | "import"
                                    | "export"
                                    | "part"
                                    | "is"
                                    | "as"
                                    | "in"
                                    | "new"
                                    | "const"
                                    | "var"
                                    | "final"
                                    | "late"
                                    | "factory"
                                    | "get"
                                    | "set"
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

pub fn parse_vue(src: &str) -> FileFacts {
    let mut facts = FileFacts::new();
    let mut pos = 0;

    // Extract all <script ...> ... </script> blocks and parse them
    while let Some(start_script) = src[pos..].find("<script") {
        let abs_start = pos + start_script;
        if let Some(tag_end) = src[abs_start..].find('>') {
            let script_content_start = abs_start + tag_end + 1;
            if let Some(end_script) = src[script_content_start..].find("</script>") {
                let script_content_end = script_content_start + end_script;
                let script_code = &src[script_content_start..script_content_end];
                let sub_facts =
                    crate::category_01_backend::parse_typescript_javascript(script_code);

                for def in sub_facts.defines {
                    facts.defines.push(def);
                }
                for (name, range) in sub_facts.ranges {
                    facts.ranges.push((
                        name,
                        (
                            range.0 + script_content_start as u32,
                            range.1 + script_content_start as u32,
                        ),
                    ));
                }
                for (name, doc) in sub_facts.docs {
                    facts.docs.push((name, doc));
                }
                for (caller, callee, rec) in sub_facts.calls {
                    facts.calls.push((caller, callee, rec));
                }
                pos = script_content_end + "</script>".len();
                continue;
            }
        }
        break;
    }

    // Also extract HTML definitions from template
    let html_facts = parse_html(src);
    for def in html_facts.defines {
        if !facts.defines.contains(&def) {
            facts.defines.push(def);
        }
    }

    facts
}

pub fn parse_svelte(src: &str) -> FileFacts {
    let mut facts = FileFacts::new();
    let mut pos = 0;

    while let Some(start_script) = src[pos..].find("<script") {
        let abs_start = pos + start_script;
        if let Some(tag_end) = src[abs_start..].find('>') {
            let script_content_start = abs_start + tag_end + 1;
            if let Some(end_script) = src[script_content_start..].find("</script>") {
                let script_content_end = script_content_start + end_script;
                let script_code = &src[script_content_start..script_content_end];
                let sub_facts =
                    crate::category_01_backend::parse_typescript_javascript(script_code);

                for def in sub_facts.defines {
                    facts.defines.push(def);
                }
                for (name, range) in sub_facts.ranges {
                    facts.ranges.push((
                        name,
                        (
                            range.0 + script_content_start as u32,
                            range.1 + script_content_start as u32,
                        ),
                    ));
                }
                for (name, doc) in sub_facts.docs {
                    facts.docs.push((name, doc));
                }
                for (caller, callee, rec) in sub_facts.calls {
                    facts.calls.push((caller, callee, rec));
                }
                pos = script_content_end + "</script>".len();
                continue;
            }
        }
        break;
    }

    let html_facts = parse_html(src);
    for def in html_facts.defines {
        if !facts.defines.contains(&def) {
            facts.defines.push(def);
        }
    }

    facts
}

pub fn parse_xml(src: &str) -> FileFacts {
    let mut facts = FileFacts::new();
    let mut i = 0;
    let bytes = src.as_bytes();
    let mut pending_comments = Vec::new();

    while i < bytes.len() {
        if bytes[i..].starts_with(b"<!--") {
            let start = i;
            if let Some(end_rel) = src[start..].find("-->") {
                let comment_text = &src[start..start + end_rel + 3];
                let clean = clean_doc(comment_text);
                if !clean.is_empty() {
                    pending_comments.push(clean);
                }
                i = start + end_rel + 3;
                continue;
            }
        }

        if bytes[i] == b'<'
            && i + 1 < bytes.len()
            && bytes[i + 1] != b'/'
            && bytes[i + 1] != b'!'
            && bytes[i + 1] != b'?'
        {
            let tag_start = i;
            if let Some(tag_end_rel) = src[tag_start..].find('>') {
                let tag_str = &src[tag_start..=tag_start + tag_end_rel];
                let tag_name = tag_str[1..]
                    .split(|c: char| c.is_whitespace() || c == '/' || c == '>')
                    .next()
                    .unwrap_or("")
                    .trim();

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

                if !tag_name.is_empty() && !facts.defines.contains(&tag_name.to_string()) {
                    facts.add_definition(
                        tag_name,
                        (tag_start as u32, (tag_start + tag_end_rel) as u32),
                        doc.clone(),
                    );
                }

                for attr in &["id=", "name="] {
                    if let Some(pos) = tag_str.find(attr) {
                        let val_start = pos + attr.len();
                        let rest = &tag_str[val_start..];
                        let quote = rest.chars().next().unwrap_or(' ');
                        if quote == '"' || quote == '\'' {
                            if let Some(end_quote) = rest[1..].find(quote) {
                                let val = &rest[1..=end_quote];
                                let abs_start = tag_start + val_start + 1;
                                let abs_end = abs_start + val.len();
                                let sym = format!("{}:{}", tag_name, val);
                                if !facts.defines.contains(&sym) {
                                    facts.add_definition(
                                        sym,
                                        (abs_start as u32, abs_end as u32),
                                        doc.clone(),
                                    );
                                }
                            }
                        }
                    }
                }

                i = tag_start + tag_end_rel + 1;
                continue;
            }
        }

        i += 1;
    }

    facts
}
