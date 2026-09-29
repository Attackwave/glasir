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
