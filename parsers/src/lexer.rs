//! High-performance, zero-dependency streaming lexer for multi-language AST extraction.
//! 100% pure Rust standard library, no external dependencies or copyright notices required.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenKind<'a> {
    Ident(&'a str),
    Keyword(&'a str),
    StringLit(&'a str),
    Number(&'a str),
    LineComment(&'a str),
    DocComment(&'a str),
    BlockComment(&'a str),
    Symbol(char),
    DoubleSymbol(&'a str),
    Newline,
    Eof,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token<'a> {
    pub kind: TokenKind<'a>,
    pub start: u32,
    pub end: u32,
}

#[derive(Debug, Clone)]
pub struct CommentStyle<'s> {
    pub line_comment_prefix: &'s [&'s str],
    pub doc_comment_prefix: &'s [&'s str],
    pub block_comment_start: Option<&'s str>,
    pub block_comment_end: Option<&'s str>,
    /// Whether `!` and `?` end an identifier or belong to it.
    ///
    /// Ruby writes `empty?` and `save!` as names; Rust writes `assert!` as a
    /// macro *invocation* of `assert`. Taking the suffix there splits one
    /// symbol into two — measured on this repository, 1,036 calls landed on
    /// bang-suffixed names that nothing defines, and the partition lost four
    /// points to the resulting duplicates.
    pub ident_suffix_marks: bool,
    /// Whether `-` and `?` may appear *inside* an identifier.
    ///
    /// A Lisp names things `commit-entry` and `defn-`, and splitting on the
    /// dash costs both halves: the name becomes three tokens, so the scanner
    /// never sees the definition keyword `defn-` at all and every private
    /// function in the file is missed. Measured on a fixture, Clojure attributed
    /// 67% of its references to `<module>` for exactly this reason.
    ///
    /// Off everywhere else, because a dash is subtraction in every language
    /// that is not a Lisp: `a-b` must stay three tokens there.
    pub ident_dashes: bool,
    /// Whether a backslash keeps a quote from closing an `r"…"` string.
    ///
    /// Rust's raw strings have no escapes at all — `r"C:\"` ends at the second
    /// quote — but Python's do: `r"a\"b"` is one string, and the backslash
    /// stays in it. Read the Rust way, one regex such as `r"(['\"]?)"` closed
    /// early and the rest of the file became a string: measured on one real
    /// file, 15 of 34 functions after that line were missing from the graph.
    pub raw_escapes: bool,
    /// Whether `'` may open a name rather than a string: Rust's lifetimes
    /// (`'a`), ML type variables, Ada attributes (`X'Length`), Verilog sizes
    /// (`4'b1010`) and a Lisp's quote. Everywhere else `'` delimits a string,
    /// and reading `'a b'` as the name `'a` turned every single-quoted string
    /// holding a space into code, and the closing quote into a string to the
    /// end of the line that swallowed what followed.
    pub quote_identifiers: bool,
}

impl Default for CommentStyle<'_> {
    fn default() -> Self {
        Self {
            line_comment_prefix: &["//"],
            doc_comment_prefix: &["///", "//!"],
            block_comment_start: Some("/*"),
            block_comment_end: Some("*/"),
            ident_suffix_marks: false,
            ident_dashes: false,
            raw_escapes: false,
            quote_identifiers: false,
        }
    }
}

/// `do: x` and `end: 3`: the word at `i` is a key, no keyword.
pub fn is_key(tokens: &[Token<'_>], i: usize) -> bool {
    tokens
        .get(i + 1)
        .is_some_and(|t| t.kind == TokenKind::Symbol(':') && t.start == tokens[i].end)
}

/// `:end` and `range.end`: the word at `i` is a symbol or a member.
pub fn is_atom_or_member(tokens: &[Token<'_>], i: usize) -> bool {
    i > 0
        && matches!(
            tokens[i - 1].kind,
            TokenKind::Symbol(':' | '.') | TokenKind::DoubleSymbol("&.")
        )
        && tokens[i - 1].end == tokens[i].start
}

pub struct Lexer<'a, 's> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
    comment_style: CommentStyle<'s>,
    /// `~S|…|` and `?"` are literals: read as code, a sigil's `do` or a quote
    /// behind `?` unbalances the block scan of the whole file.
    pub sigils: bool,
    /// `/class Foo/` is a literal where a value may start.
    pub regexes: bool,
    /// Ruby: `<<~TEXT` holds lines of text, and `run /x/` passes a regexp.
    pub ruby_literals: bool,
    heredocs: Vec<String>,
}

impl<'a, 's> Lexer<'a, 's> {
    pub fn new(src: &'a str, comment_style: CommentStyle<'s>) -> Self {
        Self {
            src,
            bytes: src.as_bytes(),
            pos: 0,
            comment_style,
            sigils: false,
            regexes: false,
            ruby_literals: false,
            heredocs: Vec::new(),
        }
    }

    pub fn src(&self) -> &'a str {
        self.src
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn is_eof(&self) -> bool {
        self.pos >= self.bytes.len()
    }

    /// Text from `pos` to `end`, empty when either offset falls inside a
    /// character.
    ///
    /// Slicing a `&str` at an arbitrary byte offset panics, and a scanner walks
    /// bytes: any file with an em dash, an umlaut or a CJK character in a
    /// comment can land a boundary inside one. Measured, that took
    /// down 10 of 48 Java files and 1 of 382 C++ files before the slices went
    /// through here.
    fn slice(&self, start: usize, end: usize) -> &'a str {
        self.src.get(start..end).unwrap_or("")
    }

    fn rest(&self) -> &'a str {
        self.src.get(self.pos..).unwrap_or("")
    }

    fn starts_with(&self, s: &str) -> bool {
        self.rest().starts_with(s)
    }

    fn skip_whitespace_except_newline(&mut self) {
        while self.pos < self.bytes.len() {
            let b = self.bytes[self.pos];
            if b == b' ' || b == b'\t' || b == b'\r' {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    /// Every token starts and ends on a character boundary, never past the
    /// end. The branches below step in bytes, and an unterminated string or
    /// an escape before the end left the position inside a character; each
    /// scanner then sliced the source there and panicked — the class this
    /// crate met eleven times, closed here once rather than per branch.
    pub fn next_token(&mut self) -> Token<'a> {
        let mut token = self.next_token_raw();
        let len = self.bytes.len();
        let settle = |mut at: usize| {
            at = at.min(len);
            while !self.src.is_char_boundary(at) {
                at += 1;
            }
            at
        };
        self.pos = settle(self.pos);
        token.end = settle(token.end as usize) as u32;
        token
    }

    fn next_token_raw(&mut self) -> Token<'a> {
        self.skip_whitespace_except_newline();

        if self.pos >= self.bytes.len() {
            return Token {
                kind: TokenKind::Eof,
                start: self.pos as u32,
                end: self.pos as u32,
            };
        }

        let start = self.pos;

        // Check for newlines
        if self.bytes[self.pos] == b'\n' {
            self.pos += 1;
            self.skip_heredocs();
            return Token {
                kind: TokenKind::Newline,
                start: start as u32,
                end: self.pos as u32,
            };
        }

        // Check for block comments first (e.g. /* ... */, {- ... -}, (* ... *), --[[ ... ]], =pod ... =cut)
        if let (Some(b_start), Some(b_end)) = (
            self.comment_style.block_comment_start,
            self.comment_style.block_comment_end,
        ) {
            if self.starts_with(b_start) {
                self.pos += b_start.len();
                let mut depth = 1;
                while self.pos < self.bytes.len() && depth > 0 {
                    if self.starts_with(b_start) && b_start != b_end {
                        depth += 1;
                        self.pos += b_start.len();
                    } else if self.starts_with(b_end) {
                        depth -= 1;
                        self.pos += b_end.len();
                    } else {
                        self.pos += 1;
                    }
                }
                let text = self.slice(start, self.pos);
                return Token {
                    kind: TokenKind::BlockComment(text),
                    start: start as u32,
                    end: self.pos as u32,
                };
            }
        }

        // Check for doc comments
        for &doc_pfx in self.comment_style.doc_comment_prefix {
            if self.starts_with(doc_pfx) {
                while self.pos < self.bytes.len() && self.bytes[self.pos] != b'\n' {
                    self.pos += 1;
                }
                let text = self.slice(start, self.pos);
                return Token {
                    kind: TokenKind::DocComment(text),
                    start: start as u32,
                    end: self.pos as u32,
                };
            }
        }

        // Check for line comments
        for &line_pfx in self.comment_style.line_comment_prefix {
            if self.starts_with(line_pfx) {
                while self.pos < self.bytes.len() && self.bytes[self.pos] != b'\n' {
                    self.pos += 1;
                }
                let text = self.slice(start, self.pos);
                return Token {
                    kind: TokenKind::LineComment(text),
                    start: start as u32,
                    end: self.pos as u32,
                };
            }
        }

        // Check for Swift / Hash-prefixed raw strings: #"..."#, ##"..."## (where # is not a comment)
        if self.bytes[self.pos] == b'#' {
            let mut hash_count = 0;
            let mut k = self.pos;
            while k < self.bytes.len() && self.bytes[k] == b'#' {
                k += 1;
                hash_count += 1;
            }
            if k < self.bytes.len() && self.bytes[k] == b'"' {
                self.pos = k + 1;
                while self.pos < self.bytes.len() {
                    if self.bytes[self.pos] == b'"' {
                        let mut match_hashes = 0;
                        while match_hashes < hash_count
                            && self.pos + 1 + match_hashes < self.bytes.len()
                            && self.bytes[self.pos + 1 + match_hashes] == b'#'
                        {
                            match_hashes += 1;
                        }
                        if match_hashes == hash_count {
                            self.pos += 1 + hash_count;
                            let end = self.pos.min(self.bytes.len());
                            return Token {
                                kind: TokenKind::StringLit(self.slice(start, end)),
                                start: start as u32,
                                end: end as u32,
                            };
                        } else {
                            self.pos += 1;
                        }
                    } else {
                        self.pos += 1;
                    }
                }
                let end = self.pos.min(self.bytes.len());
                return Token {
                    kind: TokenKind::StringLit(self.slice(start, end)),
                    start: start as u32,
                    end: end as u32,
                };
            }
        }

        if self.sigils {
            if let Some(tok) = self.lex_sigil(start) {
                return tok;
            }
        }
        if self.ruby_literals {
            self.note_heredoc(start);
        }
        if self.ruby_literals && self.bytes[start] == b'%' {
            if let Some(tok) = self.lex_percent(start) {
                return tok;
            }
        }
        if self.regexes && self.bytes[start] == b'/' {
            if let Some(tok) = self.lex_regex(start) {
                return tok;
            }
        }

        // Check for Prefixed Strings & Raw Strings (Rust r#"..."#, r"...", br#"..."#, Python f"...", r"""...""", C++ R"(...)", etc.)
        if let Some(tok) = self.try_lex_prefixed_or_raw_string(start) {
            return tok;
        }

        // Check for standard triple quotes: """ or '''
        if self.starts_with("\"\"\"") || self.starts_with("'''") {
            // Compared as bytes, never as `&str`: slicing a string at an
            // arbitrary offset panics when the boundary falls inside a
            // multi-byte character, and one em dash in a triple-quoted string
            // is enough to bring the process down.
            let quote: [u8; 3] = [
                self.bytes[self.pos],
                self.bytes[self.pos + 1],
                self.bytes[self.pos + 2],
            ];
            self.pos += 3;
            // Unterminated, it runs to the end of the file. Stopping where the
            // loop stops — two bytes short — left the next token starting
            // inside a character, and the Python scanner sliced there.
            let mut closed = false;
            while self.pos + 2 < self.bytes.len() {
                if self.bytes[self.pos..self.pos + 3] == quote {
                    self.pos += 3;
                    closed = true;
                    break;
                }
                if self.bytes[self.pos] == b'\\' {
                    if self.pos + 1 < self.bytes.len() {
                        self.pos += 2;
                    } else {
                        self.pos += 1;
                    }
                } else {
                    self.pos += 1;
                }
            }
            if !closed {
                self.pos = self.bytes.len();
            }
            let end = self.pos.min(self.bytes.len());
            return Token {
                kind: TokenKind::StringLit(self.slice(start, end)),
                start: start as u32,
                end: end as u32,
            };
        }

        let b = self.bytes[self.pos];

        if b == b'\'' && self.comment_style.quote_identifiers {
            // Check for lifetime / type variable without closing quote: e.g. `'a,`, `&'a `, `<'a>`
            let mut j = self.pos + 1;
            let mut has_close_quote = false;
            while j < self.bytes.len() && self.bytes[j] != b'\n' {
                if self.bytes[j] == b'\\' {
                    j += 2;
                    continue;
                }
                if self.bytes[j] == b'\'' {
                    has_close_quote = true;
                    break;
                }
                if self.bytes[j] == b','
                    || self.bytes[j] == b'>'
                    || self.bytes[j] == b')'
                    || self.bytes[j] == b']'
                    || self.bytes[j] == b';'
                    || self.bytes[j] == b' '
                    || self.bytes[j] == b'\t'
                {
                    break;
                }
                j += 1;
            }

            if !has_close_quote
                && self.pos + 1 < self.bytes.len()
                && (self.bytes[self.pos + 1].is_ascii_alphabetic()
                    || self.bytes[self.pos + 1] == b'_')
            {
                self.pos += 1;
                while self.pos < self.bytes.len()
                    && (self.bytes[self.pos].is_ascii_alphanumeric()
                        || self.bytes[self.pos] == b'_')
                {
                    self.pos += 1;
                }
                let end = self.pos.min(self.bytes.len());
                return Token {
                    kind: TokenKind::Ident(self.slice(start, end)),
                    start: start as u32,
                    end: end as u32,
                };
            }
        }

        if b == b'"' || b == b'\'' || b == b'`' {
            let quote = b;
            self.pos += 1;
            while self.pos < self.bytes.len() {
                let cur = self.bytes[self.pos];
                if cur == b'\\' {
                    if self.pos + 1 < self.bytes.len()
                        && self.bytes[self.pos + 1] == b'\r'
                        && self.pos + 2 < self.bytes.len()
                        && self.bytes[self.pos + 2] == b'\n'
                    {
                        self.pos += 3;
                    } else if self.pos + 1 < self.bytes.len() {
                        self.pos += 2;
                    } else {
                        self.pos += 1;
                    }
                } else if cur == quote {
                    self.pos += 1;
                    break;
                } else if cur == b'\n' && quote == b'\'' {
                    break; // Unclosed char literal on single line
                } else {
                    self.pos += 1;
                }
            }
            let end = self.pos.min(self.bytes.len());
            return Token {
                kind: TokenKind::StringLit(self.slice(start, end)),
                start: start as u32,
                end: end as u32,
            };
        }

        // Numbers: integer (decimal, hex 0x, octal 0o, binary 0b) and float (1.0, 1e-5)
        if b.is_ascii_digit() {
            // Check for 0x, 0b, 0o
            if b == b'0' && self.pos + 1 < self.bytes.len() {
                let next_b = self.bytes[self.pos + 1];
                if matches!(next_b, b'x' | b'X' | b'b' | b'B' | b'o' | b'O') {
                    self.pos += 2;
                    while self.pos < self.bytes.len() {
                        let c = self.bytes[self.pos];
                        if c.is_ascii_alphanumeric() || c == b'_' {
                            self.pos += 1;
                        } else {
                            break;
                        }
                    }
                    return Token {
                        kind: TokenKind::Number(self.slice(start, self.pos)),
                        start: start as u32,
                        end: self.pos as u32,
                    };
                }
            }

            // Decimal digits
            while self.pos < self.bytes.len() {
                let c = self.bytes[self.pos];
                if c.is_ascii_digit() || c == b'_' {
                    self.pos += 1;
                } else {
                    break;
                }
            }

            // Check if followed by '.' for float:
            // A '.' is part of a float if and only if it is followed by a digit (e.g. 1.0, 0.5)
            // If followed by an ident (0.publish) or range (0..10) or space (0. ), the '.' is NOT part of the number!
            if self.pos + 1 < self.bytes.len()
                && self.bytes[self.pos] == b'.'
                && self.bytes[self.pos + 1].is_ascii_digit()
            {
                self.pos += 1; // Consume '.'
                while self.pos < self.bytes.len() {
                    let c = self.bytes[self.pos];
                    if c.is_ascii_digit() || c == b'_' {
                        self.pos += 1;
                    } else {
                        break;
                    }
                }
            }

            // Check for exponent: e.g. 1e10, 1.0e-5, 1E+3
            if self.pos < self.bytes.len()
                && (self.bytes[self.pos] == b'e' || self.bytes[self.pos] == b'E')
            {
                let mut exp_pos = self.pos + 1;
                if exp_pos < self.bytes.len()
                    && (self.bytes[exp_pos] == b'+' || self.bytes[exp_pos] == b'-')
                {
                    exp_pos += 1;
                }
                if exp_pos < self.bytes.len() && self.bytes[exp_pos].is_ascii_digit() {
                    self.pos = exp_pos + 1;
                    while self.pos < self.bytes.len()
                        && (self.bytes[self.pos].is_ascii_digit() || self.bytes[self.pos] == b'_')
                    {
                        self.pos += 1;
                    }
                }
            }

            // Optional type suffix: u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize, f32, f64
            if self.pos < self.bytes.len() {
                let c = self.bytes[self.pos];
                if c.is_ascii_alphabetic() || c == b'_' {
                    let mut s_pos = self.pos;
                    while s_pos < self.bytes.len()
                        && (self.bytes[s_pos].is_ascii_alphanumeric() || self.bytes[s_pos] == b'_')
                    {
                        s_pos += 1;
                    }
                    let suffix = self.slice(self.pos, s_pos);
                    if matches!(
                        suffix,
                        "u8" | "u16"
                            | "u32"
                            | "u64"
                            | "u128"
                            | "usize"
                            | "i8"
                            | "i16"
                            | "i32"
                            | "i64"
                            | "i128"
                            | "isize"
                            | "f32"
                            | "f64"
                            | "f"
                            | "d"
                            | "l"
                            | "ul"
                            | "ull"
                            | "lu"
                            | "llu"
                    ) {
                        self.pos = s_pos;
                    }
                }
            }

            return Token {
                kind: TokenKind::Number(self.slice(start, self.pos)),
                start: start as u32,
                end: self.pos as u32,
            };
        }

        // Rust raw identifiers: r#ident
        if self.starts_with("r#")
            && self.pos + 2 < self.bytes.len()
            && (self.bytes[self.pos + 2].is_ascii_alphabetic() || self.bytes[self.pos + 2] == b'_')
        {
            self.pos += 2;
            while self.pos < self.bytes.len() {
                let c = self.bytes[self.pos];
                if c.is_ascii_alphanumeric() || c == b'_' || c > 127 {
                    self.pos += 1;
                } else {
                    break;
                }
            }
            let word = self.slice(start, self.pos);
            return Token {
                kind: TokenKind::Ident(word),
                start: start as u32,
                end: self.pos as u32,
            };
        }

        // Identifiers and Keywords (including unicode)
        if b.is_ascii_alphabetic() || b == b'_' || b > 127 {
            self.pos += 1;
            while self.pos < self.bytes.len() {
                let c = self.bytes[self.pos];
                let suffix = self.comment_style.ident_suffix_marks && (c == b'!' || c == b'?');
                // A Lisp's `commit-entry` is one name; everywhere else `a-b`
                // is subtraction and must stay three tokens.
                let dash = self.comment_style.ident_dashes
                    && (c == b'-' || c == b'?' || c == b'*' || c == b'!');
                if c.is_ascii_alphanumeric() || c == b'_' || suffix || dash || c > 127 {
                    self.pos += 1;
                } else {
                    break;
                }
            }
            let word = self.slice(start, self.pos);
            return Token {
                kind: TokenKind::Ident(word),
                start: start as u32,
                end: self.pos as u32,
            };
        }

        // Multi-character symbols (::, ->, =>, ==, !=, <=, >=, &&, ||, ++, --, <-, ?., &., |>, etc.)
        if self.pos + 1 < self.bytes.len() {
            // `get`, not an index: the same boundary problem.
            let two = self.src.get(self.pos..self.pos + 2).unwrap_or("");
            match two {
                "::" | "->" | "=>" | "<-" | "==" | "!=" | "<=" | ">=" | "&&" | "||" | "++" | "--" | "<<" | ">>"
                | "+=" | "-=" | "*=" | "/=" | "%=" | "&=" | "|=" | "^=" | ".." | "//" | "/*" | "*/"
                // `:=` is assignment in Pascal, Go, Wolfram and PL/SQL; without it the
                // lexer splits it and no rule can match the pair.
                | "|>" | "?." | "&." | ":=" => {
                    self.pos += 2;
                    return Token {
                        kind: TokenKind::DoubleSymbol(two),
                        start: start as u32,
                        end: self.pos as u32,
                    };
                }
                _ => {}
            }
        }

        // Single symbols
        self.pos += 1;
        Token {
            kind: TokenKind::Symbol(b as char),
            start: start as u32,
            end: self.pos as u32,
        }
    }

    pub fn collect_all_tokens(&mut self) -> Vec<Token<'a>> {
        let mut tokens = Vec::new();
        loop {
            let tok = self.next_token();
            if tok.kind == TokenKind::Eof {
                tokens.push(tok);
                break;
            }
            tokens.push(tok);
        }
        tokens
    }

    /// `<<~TEXT`, `<<-TEXT`, `<<TEXT`, the tag perhaps quoted: its body follows
    /// the line, and `class << self` has no tag beside the `<<`.
    fn note_heredoc(&mut self, start: usize) {
        let Some(rest) = self.src.get(start..).and_then(|r| r.strip_prefix("<<")) else {
            return;
        };
        let dashed = rest.strip_prefix(['~', '-']);
        let tag = dashed.unwrap_or(rest).trim_start_matches(['"', '\'']);
        let end = tag
            .find(|c: char| !c.is_alphanumeric() && c != '_')
            .unwrap_or(tag.len());
        let tag = &tag[..end];
        if !tag.is_empty() && (dashed.is_some() || tag.starts_with(|c: char| c.is_uppercase())) {
            self.heredocs.push(tag.to_string());
        }
    }

    /// At the start of a line: past the bodies of the heredocs opened above.
    fn skip_heredocs(&mut self) {
        for tag in std::mem::take(&mut self.heredocs) {
            while self.pos < self.bytes.len() {
                let line = self.src.get(self.pos..).unwrap_or("");
                let line = line.split('\n').next().unwrap_or("");
                self.pos += line.len() + 1;
                if line.trim() == tag {
                    break;
                }
            }
        }
        self.pos = self.pos.min(self.bytes.len());
    }

    /// Whether a literal, not an operator, can begin at `start`.
    fn value_may_start(&self, start: usize) -> bool {
        let Some(before) = self.src.get(..start) else {
            return false;
        };
        let next = self.bytes.get(start + 1).copied().unwrap_or(b' ');
        let trimmed = before.trim_end_matches([' ', '\t']);
        let word = trimmed
            .rsplit(|c: char| !c.is_alphanumeric() && c != '_')
            .next()
            .unwrap_or("");
        match trimmed.as_bytes().last() {
            None | Some(b'\n' | b'(' | b',' | b'=' | b':' | b'[' | b'!' | b'&' | b'|') => true,
            Some(b'?' | b'{' | b'}' | b';' | b'~') => true,
            Some(b'>') => trimmed.ends_with("=>"),
            _ => {
                matches!(
                    word,
                    "return"
                        | "if"
                        | "unless"
                        | "elsif"
                        | "when"
                        | "while"
                        | "until"
                        | "and"
                        | "or"
                        | "not"
                        | "typeof"
                        | "case"
                ) || (self.ruby_literals
                    && !word.is_empty()
                    && trimmed.len() < before.len()
                    && !next.is_ascii_whitespace())
            }
        }
    }

    /// `%w[a b]`, `%r{x}`, `%(text)`: paired delimiters nest, others do not.
    fn lex_percent(&mut self, start: usize) -> Option<Token<'a>> {
        let mut i = start + 1;
        if matches!(
            self.bytes.get(i),
            Some(b'q' | b'Q' | b'w' | b'W' | b'i' | b'I' | b'r' | b's' | b'x')
        ) {
            i += 1;
        }
        let open = *self.bytes.get(i)?;
        let close = match open {
            b'(' => b')',
            b'[' => b']',
            b'{' => b'}',
            b'<' => b'>',
            b'|' | b'!' | b'/' | b'^' => open,
            _ => return None,
        };
        if !self.value_may_start(start) {
            return None;
        }
        let mut depth = 1;
        i += 1;
        while depth > 0 {
            let &b = self.bytes.get(i)?;
            if b == b'\\' {
                i += 1;
            } else if b == close {
                depth -= 1;
            } else if b == open {
                depth += 1;
            }
            i += 1;
        }
        while self.bytes.get(i).is_some_and(u8::is_ascii_alphabetic) {
            i += 1;
        }
        self.pos = i;
        Some(Token {
            kind: TokenKind::StringLit(self.slice(start, i)),
            start: start as u32,
            end: i as u32,
        })
    }

    /// A regexp stays on its line; what cannot be one is a division.
    fn lex_regex(&mut self, start: usize) -> Option<Token<'a>> {
        if matches!(self.bytes.get(start + 1), Some(b'/' | b'*')) || !self.value_may_start(start) {
            return None;
        }
        let mut i = start + 1;
        let mut class = false;
        while let Some(&b) = self.bytes.get(i) {
            match b {
                b'\n' => return None,
                b'\\' => i += 1,
                b'[' => class = true,
                b']' => class = false,
                b'/' if !class => {
                    i += 1;
                    while self.bytes.get(i).is_some_and(u8::is_ascii_alphabetic) {
                        i += 1;
                    }
                    self.pos = i;
                    return Some(Token {
                        kind: TokenKind::StringLit(self.slice(start, i)),
                        start: start as u32,
                        end: i as u32,
                    });
                }
                _ => {}
            }
            i += 1;
        }
        None
    }

    /// A sigil runs to its first unescaped closing delimiter, a heredoc one to
    /// its `"""`; an unterminated one to the end of the file.
    fn lex_sigil(&mut self, start: usize) -> Option<Token<'a>> {
        let byte = |i: usize| self.bytes.get(i).copied().unwrap_or(0);
        let mut end = match self.bytes[start] {
            b'?' if matches!(byte(start + 1), b'"' | b'\'' | b'(' | b'[' | b'{' | b'\\') => {
                start + 2 + usize::from(byte(start + 1) == b'\\')
            }
            b'~' if start == 0 || self.bytes[start - 1] != b'=' => {
                let mut i = start + 1;
                while byte(i).is_ascii_alphabetic() {
                    i += 1;
                }
                let close = match byte(i) {
                    _ if i == start + 1 => return None,
                    b'(' => b')',
                    b'[' => b']',
                    b'{' => b'}',
                    b'<' => b'>',
                    d @ (b'/' | b'|' | b'"' | b'\'') => d,
                    _ => return None,
                };
                let quote = [close; 3];
                let heredoc = self.bytes[i..].starts_with(&quote);
                i += if heredoc { 3 } else { 1 };
                while i < self.bytes.len() {
                    if heredoc && self.bytes[i..].starts_with(&quote) {
                        i += 3;
                        break;
                    }
                    if !heredoc && self.bytes[i] == close {
                        i += 1;
                        break;
                    }
                    i += if self.bytes[i] == b'\\' { 2 } else { 1 };
                }
                while byte(i).is_ascii_alphabetic() {
                    i += 1;
                }
                i
            }
            _ => return None,
        };
        end = end.min(self.bytes.len());
        self.pos = end;
        Some(Token {
            kind: TokenKind::StringLit(self.slice(start, end)),
            start: start as u32,
            end: end as u32,
        })
    }

    fn try_lex_prefixed_or_raw_string(&mut self, start: usize) -> Option<Token<'a>> {
        let remaining = self.rest();
        let bytes = self.bytes;

        // 1. Rust raw strings: (r|br|cr|R|BR|CR) followed by 0+ '#' and '"'
        let (prefix_len, is_rust_raw) = if remaining.starts_with("br")
            || remaining.starts_with("BR")
            || remaining.starts_with("cr")
            || remaining.starts_with("CR")
        {
            (2, true)
        } else if remaining.starts_with('r') || remaining.starts_with('R') {
            (1, true)
        } else {
            (0, false)
        };

        if is_rust_raw {
            let mut k = self.pos + prefix_len;
            let mut hash_count = 0;
            while k < bytes.len() && bytes[k] == b'#' {
                k += 1;
                hash_count += 1;
            }
            if k < bytes.len() && (bytes[k] == b'"' || bytes[k] == b'\'') {
                let quote_byte = bytes[k];
                let mut quote_count = 0;
                while k + quote_count < bytes.len() && bytes[k + quote_count] == quote_byte {
                    quote_count += 1;
                }
                let target_quotes = if quote_count >= 3 { 3 } else { 1 };
                self.pos = k + target_quotes;

                while self.pos < bytes.len() {
                    if self.comment_style.raw_escapes && hash_count == 0 && bytes[self.pos] == b'\\'
                    {
                        self.pos += 2;
                        continue;
                    }
                    if bytes[self.pos] == quote_byte {
                        let mut match_quotes = 0;
                        while match_quotes < target_quotes
                            && self.pos + match_quotes < bytes.len()
                            && bytes[self.pos + match_quotes] == quote_byte
                        {
                            match_quotes += 1;
                        }
                        if match_quotes == target_quotes {
                            let mut match_hashes = 0;
                            while match_hashes < hash_count
                                && self.pos + target_quotes + match_hashes < bytes.len()
                                && bytes[self.pos + target_quotes + match_hashes] == b'#'
                            {
                                match_hashes += 1;
                            }
                            if match_hashes == hash_count {
                                self.pos += target_quotes + hash_count;
                                let end = self.pos.min(bytes.len());
                                return Some(Token {
                                    kind: TokenKind::StringLit(self.slice(start, end)),
                                    start: start as u32,
                                    end: end as u32,
                                });
                            }
                        }
                    }
                    self.pos += 1;
                }
                let end = self.pos.min(bytes.len());
                return Some(Token {
                    kind: TokenKind::StringLit(self.slice(start, end)),
                    start: start as u32,
                    end: end as u32,
                });
            }
        }

        // 2. C++ raw strings: R"delim( ... )delim"
        if remaining.starts_with("R\"") {
            let delim_start = self.pos + 2;
            let mut paren_idx = None;
            let max_idx = bytes.len().min(delim_start + 17);
            for (idx, &b) in bytes.iter().enumerate().take(max_idx).skip(delim_start) {
                if b == b'(' {
                    paren_idx = Some(idx);
                    break;
                } else if b == b' ' || b == b'\\' || b == b')' || b == b'\n' {
                    break;
                }
            }
            if let Some(paren_pos) = paren_idx {
                let delim = self.slice(delim_start, paren_pos);
                let mut closing = String::with_capacity(delim.len() + 2);
                closing.push(')');
                closing.push_str(delim);
                closing.push('"');

                self.pos = paren_pos + 1;
                while self.pos < bytes.len() {
                    if self.src[self.pos..].starts_with(&closing) {
                        self.pos += closing.len();
                        let end = self.pos.min(bytes.len());
                        return Some(Token {
                            kind: TokenKind::StringLit(self.slice(start, end)),
                            start: start as u32,
                            end: end as u32,
                        });
                    }
                    self.pos += 1;
                }
                let end = self.pos.min(bytes.len());
                return Some(Token {
                    kind: TokenKind::StringLit(self.slice(start, end)),
                    start: start as u32,
                    end: end as u32,
                });
            }
        }

        // 3. Prefixed string literals (Python/Julia/Scala/C#): f"...", b"...", u"...", raw"...", $""...
        let prefixes = [
            "raw", "RAW", "fr", "FR", "rf", "RF", "br", "BR", "rb", "RB", "f", "F", "b", "B", "u",
            "U", "c", "C", "s", "S", "$$$", "$$", "$",
        ];
        for &pfx in &prefixes {
            if let Some(after) = remaining.strip_prefix(pfx) {
                if after.starts_with("\"\"\"") || after.starts_with("'''") {
                    let q = after.as_bytes();
                    let quote: [u8; 3] = [q[0], q[1], q[2]];
                    self.pos += pfx.len() + 3;
                    while self.pos + 2 < bytes.len() {
                        if self.bytes[self.pos..self.pos + 3] == quote {
                            self.pos += 3;
                            let end = self.pos.min(bytes.len());
                            return Some(Token {
                                kind: TokenKind::StringLit(self.slice(start, end)),
                                start: start as u32,
                                end: end as u32,
                            });
                        }
                        if bytes[self.pos] == b'\\' {
                            if self.pos + 1 < bytes.len() {
                                self.pos += 2;
                            } else {
                                self.pos += 1;
                            }
                        } else {
                            self.pos += 1;
                        }
                    }
                    let end = self.pos.min(bytes.len());
                    return Some(Token {
                        kind: TokenKind::StringLit(self.slice(start, end)),
                        start: start as u32,
                        end: end as u32,
                    });
                } else if after.starts_with('"') || after.starts_with('\'') {
                    let quote_byte = after.as_bytes()[0];
                    self.pos += pfx.len() + 1;
                    while self.pos < bytes.len() {
                        let cur = bytes[self.pos];
                        if cur == b'\\' {
                            if self.pos + 1 < bytes.len() {
                                self.pos += 2;
                            } else {
                                self.pos += 1;
                            }
                        } else if cur == quote_byte {
                            self.pos += 1;
                            let end = self.pos.min(bytes.len());
                            return Some(Token {
                                kind: TokenKind::StringLit(self.slice(start, end)),
                                start: start as u32,
                                end: end as u32,
                            });
                        } else if cur == b'\n' {
                            break;
                        } else {
                            self.pos += 1;
                        }
                    }
                    let end = self.pos.min(bytes.len());
                    return Some(Token {
                        kind: TokenKind::StringLit(self.slice(start, end)),
                        start: start as u32,
                        end: end as u32,
                    });
                }
            }
        }

        None
    }
}

/// Whether the `#define` name at token `name` is a function-like macro with
/// nothing after its parameters: the stub a build option leaves in place of
/// the real function, and no definition of that name.
pub fn is_empty_macro(tokens: &[Token<'_>], name: usize) -> bool {
    let (Some(n), Some(open)) = (tokens.get(name), tokens.get(name + 1)) else {
        return false;
    };
    if open.kind != TokenKind::Symbol('(') || open.start != n.end {
        return false;
    }
    let mut k = name + 2;
    while let Some(t) = tokens.get(k) {
        match t.kind {
            TokenKind::Symbol(')') => break,
            TokenKind::Newline => return false,
            _ => k += 1,
        }
    }
    directive_end(tokens, name) == tokens.get(k).map_or(0, |t| t.end)
}

/// The end byte of the logical line a preprocessor directive starting at
/// token `from` occupies: up to the first line break not escaped by `\`.
pub fn directive_end(tokens: &[Token<'_>], from: usize) -> u32 {
    let mut end = tokens.get(from).map_or(0, |t| t.end);
    let mut k = from;
    while let Some(t) = tokens.get(k) {
        if t.kind == TokenKind::Newline {
            let escaped = k > 0 && tokens[k - 1].kind == TokenKind::Symbol('\\');
            if !escaped {
                break;
            }
        } else {
            end = t.end;
        }
        k += 1;
    }
    end
}
