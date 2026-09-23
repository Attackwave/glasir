//! Category 9: Smart Contracts & Hardware Description
//! Parsers for: Solidity, Verilog / SystemVerilog

use crate::facts::FileFacts;
use crate::lexer::{CommentStyle, Lexer, TokenKind};
use crate::scope::ScopeStack;

pub fn parse_solidity(src: &str) -> FileFacts {
    let mut facts = FileFacts::new();
    let style = CommentStyle {
        line_comment_prefix: &["///", "//"],
        doc_comment_prefix: &["///"],
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
                let mut cur_ident = *ident;
                if cur_ident == "abstract" && i + 1 < tokens.len() && tokens[i + 1].kind == TokenKind::Ident("contract") {
                    i += 1;
                    cur_ident = "contract";
                }

                match cur_ident {
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
                    "contract" | "interface" | "library" => {
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
                    "function" | "modifier" => {
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
                    "event" | "error" => {
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
                    "struct" | "enum" => {
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
                        if j < tokens.len()
                            && tokens[j].kind == TokenKind::Symbol('(')
                            && !matches!(
                                *ident,
                                "if" | "while"
                                    | "for"
                                    | "return"
                                    | "returns"
                                    | "require"
                                    | "revert"
                                    | "emit"
                                    | "payable"
                                    | "public"
                                    | "private"
                                    | "external"
                                    | "internal"
                                    | "view"
                                    | "pure"
                                    | "override"
                                    | "virtual"
                                    | "memory"
                                    | "storage"
                                    | "calldata"
                                    | "indexed"
                                    | "unchecked"
                                    | "assembly"
                                    | "modifier"
                                    | "constructor"
                                    | "fallback"
                                    | "receive"
                                    | "mapping"
                                    | "type"
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

pub fn parse_verilog(src: &str) -> FileFacts {
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
            TokenKind::Symbol('.') => {
                scope.on_receiver();
                i += 1;
                continue;
            }
            TokenKind::Ident(ident) => {
                match *ident {
                    "module" | "interface" | "package" | "class" | "program" => {
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
                    "task" | "function" => {
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
                    "endmodule" | "endinterface" | "endpackage" | "endclass" | "endprogram" | "endtask" | "endfunction" => {
                        scope.on_close_delimiter(tok.end as usize, &mut facts);
                    }
                    _ => {
                        let mut j = i + 1;
                        while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                            j += 1;
                        }
                        if j < tokens.len()
                            && tokens[j].kind == TokenKind::Symbol('(')
                            && !matches!(*ident, "if" | "while" | "for" | "case" | "begin" | "end" | "initial" | "always" | "always_comb" | "always_ff" | "assign")
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

pub fn parse_vhdl(src: &str) -> FileFacts {
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
                let id_lower = ident.to_ascii_lowercase();
                match id_lower.as_str() {
                    "entity" | "architecture" | "component" | "package" => {
                        let start_byte = tok.start;
                        let mut j = i + 1;
                        while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                            j += 1;
                        }
                        if j < tokens.len() && tokens[j].kind == TokenKind::Ident("body") {
                            j += 1;
                            while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                                j += 1;
                            }
                        }
                        if j < tokens.len() {
                            if let TokenKind::Ident(name) = tokens[j].kind {
                                // Definition first, body's level after — the
                                // same ordering the routine branch below
                                // needed. Reversed, an `entity`, `package` or
                                // `architecture` closed itself at its first
                                // inner `end` and everything after it fell to
                                // `<module>`.
                                scope.open_definition_with_body_docs(name, start_byte as usize, true, false, &mut facts);
                                scope.on_word(name);
                                scope.on_open_delimiter();
                                i = j + 1;
                                continue;
                            }
                        }
                    }
                    "procedure" | "function" => {
                        let start_byte = tok.start;
                        let mut j = i + 1;
                        while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                            j += 1;
                        }
                        if j < tokens.len() {
                            if let TokenKind::Ident(name) = tokens[j].kind {
                                // **The definition is opened first, the body's
                                // level after it.** Reversed, the definition
                                // records the already-raised depth and the
                                // matching `end` drops back to exactly that
                                // value, closing the routine at its own first
                                // `end` — measured, 25% of references on
                                // `<module>`. Eleventh occurrence of the shape
                                // Lua, Fortran, Julia, Scheme, HCL, Tcl,
                                // PL/SQL, ReScript and ChiaLisp each needed.
                                scope.open_definition_with_body_docs(name, start_byte as usize, true, true, &mut facts);
                                scope.on_word(name);
                                // Step over the parameter list, whose closing
                                // parenthesis would otherwise end the routine.
                                let mut k = j + 1;
                                if tokens.get(k).map(|t| &t.kind) == Some(&TokenKind::Symbol('(')) {
                                    let mut depth = 0usize;
                                    while k < tokens.len() {
                                        match tokens[k].kind {
                                            TokenKind::Symbol('(') => depth += 1,
                                            TokenKind::Symbol(')') => {
                                                depth -= 1;
                                                if depth == 0 {
                                                    break;
                                                }
                                            }
                                            _ => {}
                                        }
                                        k += 1;
                                    }
                                    k += 1;
                                }
                                scope.on_open_delimiter();
                                i = k;
                                continue;
                            }
                        }
                    }
                    "process" | "block" => {
                        scope.on_open_delimiter();
                    }
                    "end" => {
                        scope.on_close_delimiter(tok.end as usize, &mut facts);
                        // Skip optional following keyword/name like `end entity foo;` or `end process;`
                        let mut j = i + 1;
                        while j < tokens.len() && tokens[j].kind != TokenKind::Symbol(';') && tokens[j].kind != TokenKind::Newline {
                            j += 1;
                        }
                        i = j;
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
                                id_lower.as_str(),
                                "if" | "then" | "elsif" | "else" | "case" | "when" | "for" | "while" | "loop" | "return" | "wait" | "assert" | "report" | "severity" | "is" | "of" | "to" | "downto" | "in" | "out" | "inout" | "buffer" | "signal" | "variable" | "constant" | "type" | "subtype" | "port" | "generic" | "map"
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

pub fn parse_shader(src: &str) -> FileFacts {
    let mut facts = FileFacts::new();
    let style = CommentStyle {
        line_comment_prefix: &["///", "//"],
        doc_comment_prefix: &["///"],
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
            TokenKind::Symbol('@') => {
                // Skip attributes like @vertex, @fragment, @compute, @group(0), @binding(0)
                let mut j = i + 1;
                if j < tokens.len() && matches!(tokens[j].kind, TokenKind::Ident(_)) {
                    j += 1;
                    if j < tokens.len() && tokens[j].kind == TokenKind::Symbol('(') {
                        j += 1;
                        while j < tokens.len() && tokens[j].kind != TokenKind::Symbol(')') {
                            j += 1;
                        }
                        if j < tokens.len() {
                            j += 1;
                        }
                    }
                }
                i = j;
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
                            if let TokenKind::Ident(name) = tokens[j].kind {
                                scope.open_definition_with_body_docs(name, start_byte as usize, true, true, &mut facts);
                                scope.on_word(name);
                                i = j + 1;
                                continue;
                            }
                        }
                    }
                    "struct" | "cbuffer" => {
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
                        // Check for C/GLSL-style function: `type fn_name(...) {`
                        let mut j = i + 1;
                        while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                            j += 1;
                        }
                        if j < tokens.len() {
                            if let TokenKind::Ident(fn_name) = tokens[j].kind {
                                let mut k = j + 1;
                                while k < tokens.len() && tokens[k].kind == TokenKind::Newline {
                                    k += 1;
                                }
                                if k < tokens.len()
                                    && tokens[k].kind == TokenKind::Symbol('(')
                                    && !matches!(*ident, "return" | "if" | "while" | "for" | "switch" | "case" | "discard")
                                {
                                    // Find opening brace to verify function definition vs call
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
                                    while l < tokens.len() && (tokens[l].kind == TokenKind::Newline || matches!(tokens[l].kind, TokenKind::Ident(_) | TokenKind::Symbol(':'))) {
                                        l += 1;
                                    }
                                    if l < tokens.len() && tokens[l].kind == TokenKind::Symbol('{') {
                                        let start_byte = tok.start;
                                        scope.open_definition_with_body_docs(fn_name, start_byte as usize, true, true, &mut facts);
                                        scope.on_word(fn_name);
                                        i = j + 1;
                                        continue;
                                    }
                                }
                            }
                        }

                        // Check if function call: `name(`
                        let mut j = i + 1;
                        while j < tokens.len() && tokens[j].kind == TokenKind::Newline {
                            j += 1;
                        }
                        if j < tokens.len()
                            && tokens[j].kind == TokenKind::Symbol('(')
                            && !matches!(
                                *ident,
                                "if" | "while" | "for" | "switch" | "return" | "discard" | "vec2" | "vec3" | "vec4" | "ivec2" | "ivec3" | "ivec4" | "mat2" | "mat3" | "mat4" | "float" | "int" | "uint" | "bool" | "float2" | "float3" | "float4" | "int2" | "int3" | "int4" | "uint2" | "uint3" | "uint4"
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
