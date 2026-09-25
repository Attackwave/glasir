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
    /// early and the rest of the file became a string: measured on graphify's
    /// `cli.py`, 15 of 34 functions after that line were missing from the graph.
    pub raw_escapes: bool,
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
        }
    }
}

pub struct Lexer<'a, 's> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
    comment_style: CommentStyle<'s>,
}

impl<'a, 's> Lexer<'a, 's> {
    pub fn new(src: &'a str, comment_style: CommentStyle<'s>) -> Self {
        Self {
            src,
            bytes: src.as_bytes(),
            pos: 0,
            comment_style,
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
    /// comment can land a boundary inside one. Measured on real code, that took
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

        if b == b'\'' {
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
