//! Architectural scope stack and definition tracking.
//!
//! Manages open definition scopes, brace/indent depth, body-comment
//! accumulation (which belong to all enclosing definitions), and
//! receiver context tracking (self/this/Self disambiguation).

use crate::doc::clean_doc;
use crate::facts::FileFacts;

/// A definition that has started and may accumulate inner comments until its body closes.
#[derive(Debug, Clone)]
pub struct OpenDef {
    pub name: String,
    pub start_byte: usize,
    pub depth: i32,
    pub doc_raw: String,
    pub opens_body: bool,
    pub collects_body_docs: bool,
    /// True for a definition whose scope ends at the next `;` rather than at a
    /// closing brace — a constant with an initialiser.
    pub statement_scoped: bool,
}

#[derive(Debug, Default)]
pub struct ScopeStack {
    /// Pending doc comments seen before a definition begins.
    pub pending_docs: Vec<String>,
    /// Active open definitions, innermost last.
    pub open: Vec<OpenDef>,
    /// Stack of enclosing function/method names with the depth their body opened at.
    pub enclosing: Vec<(String, i32)>,
    /// Last word/identifier seen, used to check whether a receiver is `self` or `this`.
    pub last_word: Option<String>,
    /// Set when '.' or '::' or '->' is seen, indicating the next call is on an external receiver.
    pub had_receiver: bool,
    /// The module a `::` call went through, when the receiver was one.
    pub module_receiver: Option<String>,
    /// Current nesting depth (braces, indents, or block delimiters).
    pub depth: i32,
    /// Flagged if closing delimiters occur with depth == 0.
    pub saw_unbalanced: bool,
}

impl ScopeStack {
    pub fn new() -> Self {
        Self::default()
    }

    /// Push a comment line or block.
    /// 1. Always stores as pending doc for the next incoming definition.
    /// 2. If inside an open body (`depth > def.depth`) of a definition that accumulates body docs (like `fn` or `impl`),
    ///    ALSO appends to that definition's doc. Structs/enums do not accumulate field docs.
    pub fn push_comment(&mut self, text: &str) {
        // An inner doc comment documents the file, not what follows it.
        // Attributing a header to the first definition gives every symbol in
        // the file one identical score, which measured worse than no
        // documentation at all — so the pending run is dropped rather than
        // extended.
        let t = text.trim_start();
        if t.starts_with("//!") || t.starts_with("/*!") || t.starts_with("#!") {
            self.pending_docs.clear();
            return;
        }
        self.pending_docs.push(text.to_string());

        let inside = self
            .open
            .iter()
            .rposition(|o| o.collects_body_docs && o.depth < self.depth);
        if let Some(first) = inside {
            for o in self.open.iter_mut().take(first + 1) {
                if o.collects_body_docs && o.depth < self.depth {
                    o.doc_raw.push(' ');
                    o.doc_raw.push_str(text);
                }
            }
        }
    }

    /// Open a new definition that opens a body and optionally collects body docs.
    /// Ends a statement-scoped definition. A constant opens no brace —
    /// `const X: usize = f();` — so its scope closes at the `;`, and the call
    /// in its initialiser belongs to it rather than to the module. Without
    /// this it would keep taking every call until the enclosing block closed.
    pub fn on_statement_end(&mut self, current_byte: usize, facts: &mut FileFacts) {
        while self.open.last().is_some_and(|o| o.statement_scoped) {
            let def = self.open.pop().expect("checked");
            facts.ranges.push((
                def.name.clone(),
                (def.start_byte as u32, current_byte as u32),
            ));
            let cleaned = clean_doc(&def.doc_raw);
            if !cleaned.is_empty() {
                facts.docs.push((def.name.clone(), cleaned));
            }
            if let Some(pos) = self.enclosing.iter().rposition(|(n, _)| *n == def.name) {
                self.enclosing.remove(pos);
            }
        }
        self.pending_docs.clear();
    }

    /// A definition whose scope ends at the next `;`: the call in a constant's
    /// initialiser belongs to the constant, not to the module.
    pub fn open_statement_definition(
        &mut self,
        name: impl Into<String>,
        start_byte: usize,
        facts: &mut FileFacts,
    ) {
        self.open_definition_with_body_docs(name, start_byte, false, false, facts);
        if let Some(last) = self.open.last_mut() {
            last.statement_scoped = true;
            let n = last.name.clone();
            self.enclosing.push((n, self.depth));
        }
    }

    pub fn open_definition_with_body_docs(
        &mut self,
        name: impl Into<String>,
        start_byte: usize,
        opens_body: bool,
        collects_body_docs: bool,
        facts: &mut FileFacts,
    ) {
        let name_str = name.into();
        facts.defines.push(name_str.clone());

        let head_doc = self.pending_docs.join(" ");
        self.pending_docs.clear();

        self.open.push(OpenDef {
            name: name_str.clone(),
            start_byte,
            depth: self.depth,
            doc_raw: head_doc,
            opens_body,
            collects_body_docs,
            statement_scoped: false,
        });

        if opens_body {
            self.enclosing.push((name_str, self.depth));
        }
        self.had_receiver = false;
    }

    /// Moves the definition just opened to the depth of its body. For an
    /// indentation language the body is one level in from the keyword; both
    /// stacks move, or the range closes on the dedent while calls after it
    /// stay attributed to the nested definition.
    pub fn set_body_depth(&mut self, depth: i32) {
        let Some(last) = self.open.last_mut() else {
            return;
        };
        last.depth = depth;
        if last.opens_body {
            if let Some(e) = self.enclosing.last_mut() {
                e.1 = depth;
            }
        }
    }

    /// Open a new definition (defaults collects_body_docs to opens_body for function-like definitions).
    pub fn open_definition(
        &mut self,
        name: impl Into<String>,
        start_byte: usize,
        opens_body: bool,
        facts: &mut FileFacts,
    ) {
        self.open_definition_with_body_docs(name, start_byte, opens_body, opens_body, facts);
    }

    /// Called when an opening delimiter ('{', block begin, or indent increase) is encountered.
    pub fn on_open_delimiter(&mut self) {
        self.depth += 1;
    }

    /// Called when a closing delimiter ('}', 'end', or dedent) is encountered.
    /// Pops all definitions whose scope has closed, emits ranges and cleaned doc comments.
    pub fn on_close_delimiter(&mut self, current_byte: usize, facts: &mut FileFacts) {
        if self.depth == 0 {
            self.saw_unbalanced = true;
        } else {
            self.depth -= 1;
        }

        // A statement-scoped definition ends at its `;`, never at a brace:
        // `const f = ({ a }) => …` opens and closes a brace inside its own
        // parameter list, and closing on that put every call in the body on
        // `<module>`. Measured on OpenZeppelin's TypeScript, 205 of 313.
        while self
            .open
            .last()
            .is_some_and(|o| !o.statement_scoped && o.depth >= self.depth)
        {
            let def = self.open.pop().unwrap();
            facts.ranges.push((
                def.name.clone(),
                (def.start_byte as u32, current_byte as u32),
            ));
            let cleaned = clean_doc(&def.doc_raw);
            if !cleaned.is_empty() {
                facts.docs.push((def.name, cleaned));
            }
        }

        // Same reasoning for the caller stack: a statement-scoped definition
        // is still open, so it must stay here until its `;`. Both halves are
        // needed — `open` drives the byte range, `enclosing` drives which
        // symbol a call is attributed to, and protecting only the first leaves
        // the calls on `<module>`.
        let statement = self
            .open
            .last()
            .map(|o| o.name.clone())
            .filter(|_| self.open.last().is_some_and(|o| o.statement_scoped));
        while self
            .enclosing
            .last()
            .is_some_and(|(n, d)| *d >= self.depth && statement.as_deref() != Some(n.as_str()))
        {
            self.enclosing.pop();
        }

        self.pending_docs.clear();
    }

    /// Close all definitions at or above the specified depth without decrementing depth.
    /// Used when a new definition at the same level starts in languages without explicit closing braces.
    pub fn close_definitions_at_or_above(
        &mut self,
        depth: i32,
        current_byte: usize,
        facts: &mut FileFacts,
    ) {
        while self.open.last().is_some_and(|o| o.depth >= depth) {
            let def = self.open.pop().unwrap();
            facts.ranges.push((
                def.name.clone(),
                (def.start_byte as u32, current_byte as u32),
            ));
            let cleaned = clean_doc(&def.doc_raw);
            if !cleaned.is_empty() {
                facts.docs.push((def.name, cleaned));
            }
        }

        while self.enclosing.last().is_some_and(|&(_, d)| d >= depth) {
            self.enclosing.pop();
        }
    }

    /// Set receiver status on '.' or '::' or '->' or '$'.
    /// Distinguishes 'self.', 'this.', 'Self::', '$this->' from foreign receivers.
    pub fn on_receiver(&mut self) {
        let is_self = matches!(
            self.last_word.as_deref(),
            Some("self" | "Self" | "this" | "$this" | "cls" | "super" | "base" | "parent")
        );
        self.had_receiver = !is_self;
        // A lower-case receiver before `::` is a *module*, and in Rust a module
        // is a file: `parse_ast::parse` names `parse` in `parse_ast.rs`. That
        // is the language definition rather than a guess — measured on this
        // tree, 77 of 77 such calls resolve to a definition in exactly the
        // file the receiver names, while `Instant::now()` and its 84 siblings
        // are upper case and stay receivers.
        self.module_receiver = match self.last_word.as_deref() {
            Some(w) if !is_self && w.starts_with(|c: char| c.is_lowercase()) => Some(w.to_string()),
            _ => None,
        };
    }

    /// Record an identifier word.
    pub fn on_word(&mut self, word: &str) {
        self.last_word = Some(word.to_string());
    }

    /// Record a call edge from the current enclosing caller.
    pub fn record_call(&mut self, callee: &str, facts: &mut FileFacts) {
        let caller = self
            .enclosing
            .last()
            .map(|(n, _)| n.clone())
            .unwrap_or_else(|| "<module>".to_string());
        facts
            .calls
            .push((caller, callee.to_string(), self.had_receiver));
        facts.call_modules.push(self.module_receiver.take());
        self.had_receiver = false;
        self.module_receiver = None;
        self.pending_docs.clear();
    }

    /// Finish parsing the file: drain all remaining open definitions up to file_len,
    /// and sort docs/ranges by defines order for deterministic fact layout.
    pub fn finish(&mut self, file_len: usize, facts: &mut FileFacts) {
        for def in self.open.drain(..) {
            facts
                .ranges
                .push((def.name.clone(), (def.start_byte as u32, file_len as u32)));
            let cleaned = clean_doc(&def.doc_raw);
            if !cleaned.is_empty() {
                facts.docs.push((def.name, cleaned));
            }
        }
        if self.depth != 0 || self.saw_unbalanced {
            facts.had_errors = true;
        }

        let def_order: std::collections::HashMap<&str, usize> = facts
            .defines
            .iter()
            .enumerate()
            .map(|(i, name)| (name.as_str(), i))
            .collect();

        facts
            .docs
            .sort_by_key(|(name, _)| def_order.get(name.as_str()).copied().unwrap_or(usize::MAX));
        facts
            .ranges
            .sort_by_key(|(name, _)| def_order.get(name.as_str()).copied().unwrap_or(usize::MAX));
    }
}

/// Whether an identifier is written the way a language writes a constant:
/// all upper case, with digits and underscores allowed.
///
/// Python, Ruby and Bash have no keyword for one — `MAX_RETRIES = 3` is an
/// ordinary assignment and only the casing says otherwise. That convention is
/// what these languages actually use, and a constant's documentation is where
/// the reason for its value is written, which is what a question about a
/// tunable is phrased from.
pub fn is_screaming_case(name: &str) -> bool {
    let mut has_letter = false;
    for c in name.chars() {
        if c.is_ascii_uppercase() {
            has_letter = true;
        } else if !(c.is_ascii_digit() || c == '_') {
            return false;
        }
    }
    has_letter
}

/// Whether `name(…) {` at this position is a method definition rather than a
/// control-flow statement.
///
/// The two are lexically identical — `if (x) { … }` and `handle(x) { … }` differ
/// only in that the first word is a keyword. Four parsers carried a copy of this
/// check and only one of them excluded keywords, so C# put `if` in the graph 786
/// times, Java 72, and JavaScript enough for `cycles` to report a `.js` file
/// calling Rust: a bare `if` node collides with every other file that has one.
///
/// The keyword list is the union across the C-family languages that share this
/// shape. A word that is a keyword in one of them is not a method name in any.
pub fn is_control_keyword(word: &str) -> bool {
    matches!(
        word,
        "if" | "else"
            | "for"
            | "while"
            | "do"
            | "switch"
            | "case"
            | "catch"
            | "try"
            | "finally"
            | "return"
            | "throw"
            | "with"
            | "using"
            | "lock"
            | "yield"
            | "await"
            | "typeof"
            | "sizeof"
            | "new"
            | "delete"
            | "in"
            | "is"
            | "as"
            | "when"
            | "match"
            | "foreach"
            | "unless"
            | "until"
            | "function"
            | "fn"
            | "def"
            | "let"
            | "var"
            | "const"
    )
}
