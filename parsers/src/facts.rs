//! Extracted symbol facts, perfectly compatible with glasir's AST extraction contract.

#[derive(Debug, Default, PartialEq, Clone)]
pub struct FileFacts {
    /// Defined symbols: functions, structs, classes, modules, tables, types.
    pub defines: Vec<String>,
    /// Byte ranges per definition: (name, (start_byte, end_byte)).
    pub ranges: Vec<(String, (u32, u32))>,
    /// Documentation per definition: (name, doc_text).
    pub docs: Vec<(String, String)>,
    /// Call graph edges: (caller, callee, has_receiver).
    pub calls: Vec<(String, String, bool)>,
    /// The module a `::` call went through, parallel to `calls`.
    ///
    /// In Rust a module is a file, so `parse_ast::parse` names `parse` in
    /// `parse_ast.rs`. Kept beside the callee rather than folded into it: the
    /// callee is the bare name every consumer already reads, and `charge ->
    /// process` must stay `process` however the call site spelled it.
    pub call_modules: Vec<Option<String>>,
    /// Error flag for malformed source.
    pub had_errors: bool,
    /// Macros this file defines whose body defines their first argument —
    /// `\1 EQU …` or `\1:` — so that every invocation defines a name.
    pub macro_definers: Vec<String>,
    /// Macro invocations whose first argument is a name: (macro, argument,
    /// byte range of the line). Whether one defines that name is known only
    /// once every file is read, since the macro may live in another.
    pub macro_calls: Vec<(String, String, (u32, u32))>,
    /// (routine, where it goes on): an assembly routine that ends in an
    /// unconditional branch, or falls into the label after it, continues
    /// there. Entry points sharing one body are written that way.
    pub continues_into: Vec<(String, String)>,
    /// (field, label): a label's address stored into a field or a variable,
    /// `LEA ops,A0` / `MOVE.L A0,MN_OPS(A1)`. Paired with `dispatch` across
    /// files, since the table and the call through it rarely share one.
    pub stores: Vec<(String, String)>,
    /// (routine, label, fields): an indirect call through a value loaded
    /// along these fields, outermost first, from a label's address when the
    /// chain starts at one. `MOVE.L MN_OPS(A1),A2` / `MOVE.L OP_READ(A2),A2`
    /// / `JSR (A2)` is `(None, [MN_OPS, OP_READ])`; `LEA cmds,A3` / `MOVE.L
    /// CMD_RUN(A3),A1` / `JSR (A1)` is `(Some(cmds), [CMD_RUN])`. Whether the
    /// label is a table of routines is known only across files.
    pub dispatch: Vec<(String, Option<String>, Vec<String>)>,
    /// (routine, line): an indirect call whose target could not be traced.
    pub indirect: Vec<(String, u32)>,
    /// Labels holding pointer entries (`DC.L routine`): dispatch tables, and
    /// headers that list a module's entry points.
    pub tables: Vec<String>,
}

impl FileFacts {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_definition(
        &mut self,
        name: impl Into<String>,
        range: (u32, u32),
        doc: Option<String>,
    ) {
        let name_str = name.into();
        self.defines.push(name_str.clone());
        self.ranges.push((name_str.clone(), range));
        if let Some(d) = doc {
            if !d.trim().is_empty() {
                self.docs.push((name_str, d));
            }
        }
    }

    /// Folds in definitions from a second reading of the same file that the
    /// first cannot see, keeping this one's calls and ranges.
    ///
    /// One language needs it: Typst is a document format *and* a scripting
    /// language, so its headings come from a line scanner and its `#let`
    /// bindings from the rule path. Neither reading is wrong and neither is
    /// complete — merging is narrower than teaching one of them the other's
    /// job. A name already present is left alone, so the caller's own reading
    /// wins on a collision.
    pub fn merge_definitions_from(&mut self, other: FileFacts) {
        for name in other.defines {
            if self.defines.contains(&name) {
                continue;
            }
            if let Some((_, range)) = other.ranges.iter().find(|(n, _)| *n == name) {
                let doc = other
                    .docs
                    .iter()
                    .find(|(n, _)| *n == name)
                    .map(|(_, d)| d.clone());
                self.add_definition(name, *range, doc);
            }
        }
    }

    pub fn add_call(
        &mut self,
        caller: impl Into<String>,
        callee: impl Into<String>,
        has_receiver: bool,
    ) {
        self.calls
            .push((caller.into(), callee.into(), has_receiver));
        self.call_modules.push(None);
    }
}
