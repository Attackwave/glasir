//! Wires the parser to the graph: file change -> facts -> delta edges.
//!
//! The base CSR is immutable, so a symbol first seen at runtime cannot get a
//! node there. `SymbolRegistry` hands out ids past `base.node_count()`; those
//! nodes live only in the delta until a compaction folds them into a new base.
//! That is why lookups go through the registry rather than through the CSR.

use crate::arena::SymbolArena;
use crate::csr::{Edge, NodeId};
use crate::delta::DeltaStore;
use crate::graph::{Graph, GraphSnapshot};
use crate::parse_ast::{self, Lang, LangExt};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

/// Canonical graph paths are root-relative and always use `/`.
///
/// Symbol identities, snapshot metadata and contracts are persisted across
/// operating systems. Keeping this conversion at the ingestion boundary
/// prevents Windows separators from creating a second identity for a file.
fn relative_path(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// What one file declares and sends over HTTP, resolved to symbols.
#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct FileHttp {
    pub routes: Vec<(String, crate::routes::Verb, String)>,
    pub requests: Vec<(NodeId, crate::routes::Verb, String)>,
}

/// (caller, callee, register, label, fields): a known value handed on.
pub type Pass = (NodeId, String, String, Option<String>, Vec<String>);

/// What one file stores into fields and calls through them, for resolving
/// indirect calls across files. See `mcp::References`.
#[derive(Debug, Default, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FileDispatch {
    /// (field, label): a label's address stored into a field.
    pub stores: Vec<(String, String)>,
    /// (routine, label, fields, line): an indirect call and what it goes
    /// through.
    pub calls: Vec<(NodeId, Option<String>, Vec<String>, u32)>,
    /// (routine, line): an indirect call nothing here could trace.
    pub unresolved: Vec<(NodeId, u32)>,
    /// Labels of this file that are tables of routines.
    pub tables: Vec<NodeId>,
    /// (name, value as written) of every constant.
    pub constants: Vec<(String, String)>,
    /// (table, offset, routine): table entries at known offsets.
    pub slots: Vec<(NodeId, u32, String)>,
    /// (caller, callee, register, label, fields): a known value handed on.
    pub passes: Vec<Pass>,
    /// (caller, callee, registers): parameters handed on unchanged.
    pub forwards: Vec<(NodeId, String, u16)>,
    /// (routine, register, fields, line): a call through a parameter.
    pub param_calls: Vec<(NodeId, String, Vec<String>, u32)>,
    /// (routine, register, field): a parameter stored into a field.
    pub param_stores: Vec<(NodeId, String, String)>,
    /// (routine, vector, line): a call into a library through A6.
    pub library: Vec<(NodeId, String, u32)>,
}

impl FileDispatch {
    fn is_empty(&self) -> bool {
        *self == FileDispatch::default()
    }
}

/// What the tree says about its classes. Ordered, since it is stored with the
/// graph and a rebuild must write the same bytes.
#[derive(Debug, Default, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ClassFacts {
    /// Classes named as a base that this tree defines, and those it does not:
    /// one from outside says nothing a bare name would not, at a node each.
    bases_here: BTreeSet<String>,
    bases_elsewhere: BTreeSet<String>,
    bases_twice: BTreeSet<String>,
    /// The bases each class names, by `file#class`: several classes may share
    /// a name, and each has its own chain.
    class_bases: BTreeMap<String, Vec<String>>,
    /// Interfaces and protocols, by `file#name`: a class beside one in a base
    /// list holds the method the interface only declares.
    contracts: BTreeSet<String>,
    /// Traits, modules and mixins, by `file#name`.
    mixins: BTreeSet<String>,
    /// Swift extensions and Kotlin `actual` classes, by `file#name`: defined
    /// as symbols, yet not the type a caller means.
    extensions: BTreeSet<String>,
    /// Go methods by `directory#type`: the files holding each name.
    methods: BTreeMap<String, BTreeMap<String, Vec<String>>>,
    /// The files naming each base the tree does not hold.
    named_by: BTreeMap<String, BTreeSet<String>>,
}

impl ClassFacts {
    /// Forgets what `file` said, ahead of reading it again.
    fn forget(&mut self, file: &str) {
        let prefix = format!("{file}#");
        self.class_bases.retain(|k, _| !k.starts_with(&prefix));
        for names in self.methods.values_mut() {
            names.retain(|_, files| {
                files.retain(|f| f != file);
                !files.is_empty()
            });
        }
        self.contracts.retain(|k| !k.starts_with(&prefix));
        self.mixins.retain(|k| !k.starts_with(&prefix));
        self.extensions.retain(|k| !k.starts_with(&prefix));
    }
}

/// Maps a qualified symbol name to a node id, minting ids for symbols the base
/// graph has never seen.
pub struct SymbolRegistry {
    by_name: HashMap<String, NodeId>,
    /// Documentation per node, when the parser found any. Kept here rather
    /// than threaded through `ingest_file`'s return value because it belongs
    /// to the symbol, exactly like its name — and the search index is built
    /// from the registry.
    docs: HashMap<NodeId, String>,
    /// Byte range per node, for the tool that returns source rather than a
    /// name. Only definitions have one — a placeholder names a call site in
    /// somebody else's crate and has no text here to point at.
    spans: HashMap<NodeId, (u32, u32)>,
    /// The language a placeholder was minted from, when it was minted from one.
    ///
    /// A placeholder is a bare name with no file, so nothing about it says
    /// which language its call site was written in — and tier 3 links by name
    /// alone. Measured on this tree, `console.error` in a `.js` file reached
    /// `src/mcp.rs#error` that way, and 18 of the 20 cycles `cycles` reported
    /// ran through that one edge. SCIP solves the same problem by putting the
    /// scheme in the symbol itself; this keeps it beside the symbol instead, so
    /// the name a caller sees is unchanged.
    ///
    /// A name reached from two languages has none: it is then shared, and the
    /// safe reading is to link it nowhere rather than to guess a side.
    placeholder_lang: HashMap<NodeId, Vec<Lang>>,
    /// Per file, the names each of its definitions uses without calling:
    /// (definition, name). Resolved against the whole tree when served, since
    /// only then is it known which names are defined exactly once. Keyed by
    /// file so a re-parse replaces them in one insert.
    refs: HashMap<String, Vec<(NodeId, String)>>,
    /// Per file, the HTTP routes it declares — handler symbol, verb, path —
    /// and the requests it sends — sending node, verb, path. Matched against
    /// each other when served. See `routes`.
    http: HashMap<String, FileHttp>,
    dispatch: HashMap<String, FileDispatch>,
    /// Per file, the macros it defines whose invocations define their first
    /// argument. Kept so a partial re-parse knows the whole tree's set.
    definers: HashMap<String, Vec<String>>,
    /// What resolving through classes needs, stored with the graph.
    classes: ClassFacts,
    base_arrived: BTreeSet<String>,
    called_here: Option<HashSet<String>>,
    next: NodeId,
}

impl SymbolRegistry {
    /// Sorts the bases these files name into defined here or elsewhere, by
    /// these files and what is already registered.
    pub fn find_bases<'f>(
        &mut self,
        files: impl Iterator<Item = &'f parse_ast::FileFacts> + Clone,
    ) {
        let mut wanted: HashSet<&str> = HashSet::new();
        for file in files.clone() {
            for import in &file.imports {
                if let crate::imports::Import::Base { module, .. } = import
                    && !self.classes.bases_here.contains(module)
                    && !self.classes.bases_elsewhere.contains(module)
                {
                    wanted.insert(module);
                }
            }
        }
        for file in files.clone() {
            for name in &file.defines {
                if self.classes.bases_elsewhere.contains(name) {
                    wanted.insert(name);
                }
            }
        }
        if wanted.is_empty() {
            return;
        }
        let in_files = files.flat_map(|f| f.defines.iter().map(String::as_str));
        let registered = self
            .by_name
            .keys()
            .filter_map(|s| s.split_once('#').map(|(_, name)| name));
        let mut here: HashMap<&str, usize> = HashMap::new();
        for name in in_files.chain(registered) {
            if wanted.contains(name) {
                *here.entry(name).or_default() += 1;
            }
        }
        for (name, count) in &here {
            if *count > 1 {
                self.classes.bases_twice.insert((*name).to_string());
            }
        }
        let arrived = here
            .keys()
            .filter(|name| self.classes.bases_elsewhere.contains(**name))
            .map(|name| name.to_string())
            .collect::<Vec<_>>();
        for name in arrived {
            self.classes.bases_elsewhere.remove(&name);
            self.classes.bases_here.insert(name.clone());
            self.base_arrived
                .extend(self.classes.named_by.remove(&name).unwrap_or_default());
        }
        for module in wanted {
            let set = if here.contains_key(module) {
                &mut self.classes.bases_here
            } else {
                &mut self.classes.bases_elsewhere
            };
            set.insert(module.to_string());
        }
    }

    /// What the tree says about its classes.
    pub fn classes(&self) -> &ClassFacts {
        &self.classes
    }

    /// Restores what a stored graph said about its classes.
    pub fn set_classes(&mut self, classes: ClassFacts) {
        self.classes = classes;
    }

    /// Whether the class `symbol` is mixed into others.
    pub fn is_mixin(&self, symbol: &str) -> bool {
        self.classes.mixins.contains(symbol)
    }

    /// The files built without a base that has since arrived. Asking clears it.
    pub fn take_base_arrived(&mut self) -> BTreeSet<String> {
        std::mem::take(&mut self.base_arrived)
    }

    /// Notes which names these files call and the tree defines: only those are
    /// worth a placeholder of their own, the rest stay one shared bare name.
    pub fn find_called<'f>(
        &mut self,
        files: impl Iterator<Item = &'f parse_ast::FileFacts> + Clone,
    ) {
        let wanted: HashSet<&str> = files
            .clone()
            .filter(|f| {
                f.imports
                    .iter()
                    .any(|i| matches!(i, crate::imports::Import::Base { .. }))
            })
            .flat_map(|f| f.calls.iter().map(|(_, callee, _)| callee.as_str()))
            .collect();
        let in_files = files.flat_map(|f| f.defines.iter().map(String::as_str));
        let registered = self
            .by_name
            .keys()
            .filter_map(|s| s.split_once('#').map(|(_, name)| name));
        let here = in_files
            .chain(registered)
            .filter(|name| wanted.contains(name))
            .map(str::to_string)
            .collect();
        self.called_here = Some(here);
    }

    /// Ends what `find_called` said, once its files are read.
    pub fn forget_called(&mut self) {
        self.called_here = None;
    }

    /// Whether a call of `name` may reach a definition in this tree.
    fn may_define(&self, name: &str) -> bool {
        self.called_here.as_ref().is_none_or(|h| h.contains(name))
    }

    /// The files of `directory` that define `name` as a method of `class`.
    /// Each type's methods, in every directory that holds an `impl` of it:
    /// Rust may put one anywhere in the crate.
    pub fn methods_by_type(&self) -> HashMap<&str, Vec<&BTreeMap<String, Vec<String>>>> {
        let mut out: HashMap<&str, Vec<_>> = HashMap::new();
        for (key, names) in &self.classes.methods {
            if let Some((_, class)) = key.split_once('#') {
                out.entry(class).or_default().push(names);
            }
        }
        out
    }

    pub fn method_files(&self, directory: &str, class: &str, name: &str) -> &[String] {
        self.classes
            .methods
            .get(&format!("{directory}#{class}"))
            .and_then(|names| names.get(name))
            .map_or(&[], Vec::as_slice)
    }

    /// Whether `file`, read again as `facts`, no longer names a base it did.
    /// Links are only ever added, so one through that base would stay.
    pub fn base_dropped(&self, file: &str, facts: &parse_ast::FileFacts) -> bool {
        let named: HashSet<&str> = facts
            .imports
            .iter()
            .filter_map(|import| match import {
                crate::imports::Import::Base { module, .. } => Some(module.as_str()),
                _ => None,
            })
            .collect();
        let prefix = format!("{file}#");
        self.classes
            .class_bases
            .range(prefix.clone()..)
            .take_while(|(class, _)| class.starts_with(&prefix))
            .flat_map(|(_, bases)| bases)
            .any(|base| !named.contains(base.split('^').next().unwrap_or(base)))
    }

    /// Whether classes in the tree name `class` as a base it does not hold.
    pub fn base_was_elsewhere(&self, class: &str) -> bool {
        self.classes.bases_elsewhere.contains(class)
    }

    /// Whether `symbol` extends a type rather than defining it.
    pub fn is_extension(&self, symbol: &str) -> bool {
        self.classes.extensions.contains(symbol)
    }

    /// Whether the class `symbol` is an interface or a protocol.
    pub fn is_contract(&self, symbol: &str) -> bool {
        self.classes.contracts.contains(symbol)
    }

    /// The bases the class `symbol` names.
    pub fn bases_of(&self, symbol: &str) -> &[String] {
        self.classes
            .class_bases
            .get(symbol)
            .map_or(&[], Vec::as_slice)
    }

    /// Starts allocating past the base graph's nodes, so ids never collide with
    /// the ones the CSR already owns.
    pub fn new(base_node_count: usize) -> Self {
        Self {
            classes: ClassFacts::default(),
            base_arrived: BTreeSet::new(),
            called_here: None,
            by_name: HashMap::new(),
            docs: HashMap::new(),
            spans: HashMap::new(),
            placeholder_lang: HashMap::new(),
            refs: HashMap::new(),
            http: HashMap::new(),
            dispatch: HashMap::new(),
            definers: HashMap::new(),
            next: base_node_count as NodeId,
        }
    }

    /// Pre-registers the symbols an imported graph already contains.
    pub fn insert(&mut self, name: String, node: NodeId) {
        self.by_name.insert(name, node);
    }

    /// Every (symbol, node) pair. Tier 3 needs the whole table to match
    /// placeholders against definitions.
    pub fn entries(&self) -> impl Iterator<Item = (&String, &NodeId)> {
        self.by_name.iter()
    }

    /// Attaches documentation to a symbol. A re-parse overwrites it, so an
    /// edited comment does not leave the old text behind.
    pub fn set_doc(&mut self, node: NodeId, doc: String) {
        self.docs.insert(node, doc);
    }

    /// Documentation per node, for the search index to fold in.
    pub fn docs(&self) -> &HashMap<NodeId, String> {
        &self.docs
    }

    pub fn set_span(&mut self, node: NodeId, span: (u32, u32)) {
        self.spans.insert(node, span);
    }

    /// Byte range of a definition, when the parser recorded one.
    pub fn span(&self, node: NodeId) -> Option<(u32, u32)> {
        self.spans.get(&node).copied()
    }

    /// Every recorded span, for the snapshot to store.
    pub fn spans(&self) -> &HashMap<NodeId, (u32, u32)> {
        &self.spans
    }

    pub fn set_refs(&mut self, file: &str, refs: Vec<(NodeId, String)>) {
        if refs.is_empty() {
            self.refs.remove(file);
        } else {
            self.refs.insert(file.to_owned(), refs);
        }
    }

    pub fn refs(&self) -> &HashMap<String, Vec<(NodeId, String)>> {
        &self.refs
    }

    pub fn set_http(&mut self, file: &str, http: FileHttp) {
        if http.routes.is_empty() && http.requests.is_empty() {
            self.http.remove(file);
        } else {
            self.http.insert(file.to_owned(), http);
        }
    }

    pub fn http(&self) -> &HashMap<String, FileHttp> {
        &self.http
    }

    pub fn set_dispatch(&mut self, file: &str, dispatch: FileDispatch) {
        if dispatch.is_empty() {
            self.dispatch.remove(file);
        } else {
            self.dispatch.insert(file.to_owned(), dispatch);
        }
    }

    pub fn dispatch(&self) -> &HashMap<String, FileDispatch> {
        &self.dispatch
    }

    pub fn set_definers(&mut self, file: &str, definers: Vec<String>) {
        if definers.is_empty() {
            self.definers.remove(file);
        } else {
            self.definers.insert(file.to_owned(), definers);
        }
    }

    pub fn definers(&self) -> &HashMap<String, Vec<String>> {
        &self.definers
    }

    /// Every argument-defining macro in the tree, leaving out `except`.
    pub fn definer_set(&self, except: &HashSet<String>) -> HashSet<String> {
        self.definers
            .iter()
            .filter(|(f, _)| !except.contains(*f))
            .flat_map(|(_, d)| d.iter().cloned())
            .collect()
    }

    /// Keeps only references to a name the tree defines exactly once, the only
    /// ones that can be resolved: locals, parameters, the standard library and
    /// names defined twice are most of them. Run once the batch's definitions
    /// are all known. A Markdown section is not a definition.
    pub fn prune_refs(&mut self) {
        // Languages that read each other's names: a script and its typings, C
        // and C++, the JVM. A name elsewhere is a coincidence, not a use.
        let family = |file: &str| {
            use crate::parse_ast::LangExt;
            let lang = Lang::from_path(Path::new(file));
            match lang {
                Some(Lang::TypeScript) => Some(Lang::JavaScript),
                Some(Lang::Cpp) => Some(Lang::C),
                Some(Lang::Kotlin | Lang::Scala) => Some(Lang::Java),
                other => other,
            }
        };
        let mut defs: HashMap<&str, (u32, Option<Lang>)> = HashMap::new();
        for key in self.by_name.keys() {
            if let Some((file, name)) = key.split_once('#')
                && !crate::docs::is_markdown(Path::new(file))
            {
                let entry = defs.entry(name).or_insert((0, family(file)));
                entry.0 += 1;
            }
        }
        for (file, refs) in self.refs.iter_mut() {
            let own = family(file);
            for (_, name) in refs.iter_mut() {
                // An import naming a file that does not define the name — a
                // re-export — decides nothing; the plain rule applies.
                if let Some((bare, target)) = crate::imports::unscope(name)
                    && !target.ends_with('/')
                    && !self.by_name.contains_key(&format!("{target}#{bare}"))
                {
                    *name = bare.to_string();
                }
            }
            refs.retain(|(_, name)| match crate::imports::unscope(name) {
                Some(_) => true,
                None => defs.get(name.as_str()) == Some(&(1, own)),
            });
        }
        self.refs.retain(|_, refs| !refs.is_empty());
    }

    /// Node for an exact symbol, without minting one if it is absent.
    pub fn node_of(&self, name: &str) -> Option<NodeId> {
        self.by_name.get(name).copied()
    }

    /// Records which language a placeholder was reached from.
    ///
    /// Called once per call site, so a name reached from two languages ends up
    /// with `None` — shared, and not safely linkable to either side.
    pub fn note_placeholder_lang(&mut self, node: NodeId, lang: Lang) {
        let langs = self.placeholder_lang.entry(node).or_default();
        if !langs.contains(&lang) {
            langs.push(lang);
        }
    }

    /// The languages a placeholder was reached from; empty when unknown.
    pub fn placeholder_langs(&self, node: NodeId) -> &[Lang] {
        self.placeholder_lang.get(&node).map_or(&[], Vec::as_slice)
    }

    pub fn contains(&self, name: &str) -> bool {
        self.by_name.contains_key(name)
    }

    pub fn get_or_mint(&mut self, name: &str) -> NodeId {
        if let Some(&id) = self.by_name.get(name) {
            return id;
        }
        let id = self.next;
        self.next += 1;
        self.by_name.insert(name.to_owned(), id);
        id
    }

    /// (unqualified, qualified) symbol counts. An unqualified name is a
    /// reference we could not attach to a definition.
    pub fn split_counts(&self) -> (usize, usize) {
        let bare = self.by_name.keys().filter(|k| !k.contains('#')).count();
        (bare, self.by_name.len() - bare)
    }

    /// Drops every symbol a vanished file defined.
    ///
    /// **A moved file is a whole new set of nodes, and the old set used to
    /// stay.** The path is part of the key, so `src/old/x.rs#f` and
    /// `src/new/x.rs#f` are different symbols; the incremental path invalidated
    /// the old file's *edges* and kept its names, on the reasoning that
    /// something might still reference them. Nothing can: a qualified name
    /// names a path that is no longer there. Measured on a 60-file fixture,
    /// moving five files per round: 120 nodes grew to 180 over six rounds while
    /// the tree still held 120 symbols, and `src/mod3.rs#work3` — moved away in
    /// round one — seeded at 1.000 *above* its own replacement.
    ///
    /// The id is not reused, only the name is dropped: ids are handed out
    /// monotonically and a recycled one would attach to whatever edges the old
    /// node still had.
    pub fn forget_file(&mut self, file: &str) {
        let prefix = format!("{file}#");
        let gone: Vec<NodeId> = self
            .by_name
            .iter()
            .filter(|(k, _)| k.starts_with(&prefix))
            .map(|(_, &n)| n)
            .collect();
        self.by_name.retain(|k, _| !k.starts_with(&prefix));
        self.refs.remove(file);
        self.http.remove(file);
        self.dispatch.remove(file);
        self.definers.remove(file);
        self.classes.forget(file);
        for n in gone {
            self.docs.remove(&n);
            // A span into a file that is gone points at bytes that are not
            // there; the same reasoning as the docs beside it.
            self.spans.remove(&n);
        }
    }

    /// Drops the names of definitions a re-parsed file no longer has. A file
    /// still on disk goes through `get_or_mint` only, which never removes a
    /// key, so a renamed definition kept its old name, and a lookup of that
    /// name cut its stale byte range out of the current text. Called with the
    /// file's previous definitions that the parse did not produce again, which
    /// is empty unless something was renamed or removed, so the scan over the
    /// registry runs only then. The ids that remain are untouched.
    pub fn forget_nodes(&mut self, gone: &HashSet<NodeId>) {
        if gone.is_empty() {
            return;
        }
        self.by_name.retain(|_, n| !gone.contains(n));
        for n in gone {
            self.docs.remove(n);
            self.spans.remove(n);
        }
    }

    /// Drops bare placeholders nothing references any more.
    ///
    /// `forget_file` drops a vanished file's `file#name` keys, but the *bare*
    /// nodes that file alone minted survive it: a placeholder's key carries no
    /// path, so nothing connects it to the file that caused it. Measured on a
    /// 60-file fixture deleting five files per round, the leak is linear —
    /// five orphans per round, thirty after six, while the tree held 61
    /// symbols and the registry 91. They are not inert: `unique_call_1`
    /// through `_9` all seeded a search for their own name, every one of them
    /// naming a call site that no longer exists.
    ///
    /// Only a placeholder with **no edge in either direction** goes. One that
    /// is still called, or that tier 3 resolved onto a definition, is live.
    /// Ids are not reused, exactly as in `forget_file`.
    pub fn forget_orphans(&mut self, referenced: &std::collections::HashSet<NodeId>) -> usize {
        let before = self.by_name.len();
        self.by_name
            .retain(|k, n| k.contains('#') || referenced.contains(n));
        before - self.by_name.len()
    }

    pub fn len(&self) -> usize {
        self.by_name.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }
}

/// Qualified symbol name. Definitions are file-scoped so two files may each
/// define `main` without colliding.
fn qualify(file: &str, name: &str) -> String {
    format!("{file}#{name}")
}

/// Bare name -> the one symbol defining it; `None` marks a name defined more
/// than once, refused for the reason tier 3 refuses it.
type MarkdownNames = HashMap<String, Option<NodeId>>;

/// Built once per batch of documents and extended as they mint sections.
/// Rebuilt per document it was O(documents x symbols): 30 of 34 seconds on a
/// tree with 401 documents and 222,585 symbols.
fn markdown_names(registry: &SymbolRegistry) -> MarkdownNames {
    let mut by_tail = MarkdownNames::new();
    for (symbol, &node) in registry.entries() {
        note_tail(&mut by_tail, symbol, node);
    }
    by_tail
}

fn note_tail(by_tail: &mut MarkdownNames, symbol: &str, node: NodeId) {
    if let Some((_, tail)) = symbol.split_once('#') {
        by_tail
            .entry(tail.to_owned())
            .and_modify(|slot| *slot = None)
            .or_insert(Some(node));
    }
}

/// Re-parses one file and replaces its edges in the delta.
///
/// A callee is resolved against the registry by qualified name first (same
/// file), then by bare name (already seen anywhere). An unresolved callee still
/// gets a node: it is a real reference to something we have not parsed yet, and
/// dropping it would silently lose the edge. Tier 3 is what later merges those
/// placeholders onto real definitions.
/// A Markdown document as graph nodes: one per section, edged to the code it
/// names.
///
/// Sections rather than whole files, because a document answers several
/// questions and a reader needs the paragraph that answers theirs. The
/// section's prose becomes its documentation, which is what makes it findable —
/// measured on code, prose is what carries a plain-language question.
///
/// **A mention only becomes an edge when the name already exists in the graph.**
/// A document naming `HashMap` must not mint a node for it, and one naming a
/// symbol this tree does not define is describing something else. Ordering
/// therefore matters: markdown is ingested after the code, so the symbols are
/// there to be found.
///
/// Runs inside the caller's `update_batch`, after the code, so a whole tree's
/// documents cost no publish of their own: one `update` each copied the delta
/// per document, 401 copies of a million edges.
#[allow(clippy::too_many_arguments)]
pub fn ingest_markdown_into(
    snap: &GraphSnapshot,
    d: &mut DeltaStore,
    arena: &mut SymbolArena,
    registry: &mut SymbolRegistry,
    root: &Path,
    documents: &[(std::path::PathBuf, String)],
    now: u64,
) {
    let mut names = markdown_names(registry);
    for (path, src) in documents {
        let doc = markdown_facts(arena, registry, path, root, src, now, &mut names);
        registry.forget_nodes(&dropped(d.file_nodes(doc.file_key), &doc.nodes));
        apply_markdown(snap, d, doc);
    }
}

/// One document's sections and edges, resolved but not yet in the graph.
struct MarkdownDoc {
    file_key: u32,
    nodes: Vec<NodeId>,
    edges: Vec<(NodeId, Edge)>,
}

fn markdown_facts(
    arena: &mut SymbolArena,
    registry: &mut SymbolRegistry,
    path: &Path,
    root: &Path,
    source: &str,
    now: u64,
    by_tail: &mut MarkdownNames,
) -> MarkdownDoc {
    let file = relative_path(path, root);
    let mut nodes = Vec::new();
    let mut edges: Vec<(NodeId, Edge)> = Vec::new();
    let mut minted = Vec::new();
    for section in crate::docs::sections(source, crate::docs::stem(&file)) {
        let key = qualify(&file, &section.title);
        if registry.node_of(&key).is_none() {
            minted.push(key.clone());
        }
        let node = registry.get_or_mint(&key);
        nodes.push(node);
        registry.set_doc(node, section.prose.clone());

        for name in &section.mentions {
            let Some(target) = registry
                .node_of(name)
                .or_else(|| by_tail.get(name.as_str()).copied().flatten())
            else {
                continue;
            };
            if target == node {
                continue;
            }
            edges.push((
                node,
                Edge {
                    target,
                    timestamp: now,
                    // Precise — the document named the symbol outright — but
                    // from a source that describes intent rather than fact.
                    authority: crate::physics::SOURCE_DOC,
                    edge_kind: 0,
                    confidence: crate::csr::Confidence::Inferred,
                },
            ));
        }
    }

    // A section minted here becomes visible to the next document, not to this
    // one — the order a table rebuilt per document produced.
    for key in minted {
        if let Some(node) = registry.node_of(&key) {
            note_tail(by_tail, &key, node);
        }
    }

    MarkdownDoc {
        file_key: arena.intern(&file),
        nodes,
        edges,
    }
}

/// The graph half: replaces the document's previous sections and edges.
fn apply_markdown(snap: &GraphSnapshot, d: &mut DeltaStore, doc: MarkdownDoc) {
    let targets: HashMap<NodeId, Vec<NodeId>> = d
        .file_nodes(doc.file_key)
        .iter()
        .map(|&n| (n, snap.base_targets(n)))
        .collect();
    d.replace_file_edges(
        doc.file_key,
        |n| targets.get(&n).cloned().unwrap_or_default(),
        doc.edges,
        &doc.nodes,
    );
}

/// Attaches documentation to symbols a higher tier already defined.
///
/// Tier 1 resolves edges through the compiler and skips tier 2 for those files,
/// but a SCIP index carries no prose. This parses the file for documentation
/// only, touching no edges: the symbols already exist, so it looks them up
/// rather than minting, and a name the index did not define is left alone.
pub fn ingest_docs(registry: &mut SymbolRegistry, path: &Path, root: &Path, source: &str) {
    let facts = Lang::from_path(path).and_then(|lang| parse_ast::parse_file(path, source, lang));
    ingest_docs_from(registry, path, root, facts);
}

/// `ingest_docs` for facts someone else parsed — see `apply_facts`.
pub fn ingest_docs_from(
    registry: &mut SymbolRegistry,
    path: &Path,
    root: &Path,
    facts: Option<parse_ast::FileFacts>,
) {
    let Some(facts) = facts else {
        return;
    };
    let file = relative_path(path, root);
    for (name, doc) in &facts.docs {
        if let Some(node) = registry.node_of(&qualify(&file, name)) {
            registry.set_doc(node, doc.clone());
        }
    }
    // Tier 1 supersedes tier 2 for edges but carries no byte ranges, so an
    // indexed tree would have no source to show without this — the same gap
    // the documentation had, for the same reason.
    for (name, span) in &facts.ranges {
        if let Some(node) = registry.node_of(&qualify(&file, name)) {
            registry.set_span(node, *span);
        }
    }
}

/// Links a call to the function it calls, and turns one changed file into
/// the edges that replace its previous ones.
pub fn ingest_file(
    graph: &Arc<Graph>,
    arena: &mut SymbolArena,
    registry: &mut SymbolRegistry,
    path: &Path,
    root: &Path,
    source: &str,
    now: u64,
) -> Option<usize> {
    let lang = Lang::from_path(path)?;
    let mut facts = parse_ast::parse_file(path, source, lang)?;
    registry.find_bases(std::iter::once(&facts));
    if !facts.macro_calls.is_empty() {
        // The rest of the tree's argument-defining macros as last read, plus
        // this file's own. An edit that changes another file's set is picked
        // up when that file is saved or the tree is analysed again.
        let rel = path.strip_prefix(root).unwrap_or(path).to_string_lossy();
        let mut definers = registry.definer_set(&HashSet::from([rel.to_string()]));
        definers.extend(facts.macro_definers.iter().cloned());
        parse_ast::apply_defining_macros(&mut facts, lang, source, &definers);
    }
    Some(apply_facts(graph, arena, registry, path, root, facts, now))
}

/// The graph half of `ingest_file`, taking facts someone else parsed.
///
/// Split out so a whole tree can be parsed in parallel and folded in
/// afterwards: parsing is stateless per file and takes 63% of a cold start,
/// while this half touches the registry, the arena and the delta, all of which
/// are shared. Keeping the split at exactly this line is what lets the parallel
/// path stay bit-identical to the serial one — the order facts arrive here is
/// still `walk()`'s order, so every minted id is the same.
/// Everything `apply_facts` does before touching the delta: qualify the file's
/// symbols, attach their documentation, and resolve each callee.
///
/// Split out so the batch path can reuse it verbatim — the two must mint the
/// same ids in the same order, or a full build and a re-parse would disagree.
fn prepare_facts(
    arena: &mut SymbolArena,
    registry: &mut SymbolRegistry,
    path: &Path,
    root: &Path,
    facts: parse_ast::FileFacts,
    now: u64,
) -> (u32, Vec<(NodeId, Edge)>, Vec<NodeId>) {
    // Relative to the indexed root: an absolute path bloats every symbol name
    // in a served answer and leaks where the tree happens to live.
    let file = relative_path(path, root);

    // Definitions first, so a call inside the file resolves to the local
    // definition rather than minting a placeholder for it.
    let mut nodes = Vec::with_capacity(facts.defines.len());
    for name in &facts.defines {
        nodes.push(registry.get_or_mint(&qualify(&file, name)));
    }
    for (name, doc) in &facts.docs {
        let node = registry.get_or_mint(&qualify(&file, name));
        registry.set_doc(node, doc.clone());
    }
    for (name, span) in &facts.ranges {
        let node = registry.get_or_mint(&qualify(&file, name));
        registry.set_span(node, *span);
    }

    registry.classes.forget(&file);

    // What the file's imports say its names refer to: `alias.f()` and a bare
    // imported `f()`, each mapped to the file or package that defines them.
    let lang = Lang::from_path(path);
    let mut modules: HashMap<&str, String> = HashMap::new();
    let mut names: HashMap<&str, (String, &str)> = HashMap::new();
    let mut bases: Vec<(&str, u32)> = Vec::new();
    let mut packages: HashMap<&str, &str> = HashMap::new();
    let mut open_packages: Vec<&str> = Vec::new();
    let mut mixed: Vec<(u32, u32)> = Vec::new();
    let mut receivers: HashMap<u32, (&String, &String)> = HashMap::new();
    if let Some(lang) = lang {
        for import in &facts.imports {
            match import {
                crate::imports::Import::Module { alias, spec } => {
                    if let Some(t) = crate::imports::resolve(lang, &file, spec, root) {
                        modules.entry(alias).or_insert(t);
                    }
                }
                crate::imports::Import::Name { local, spec, name } => {
                    let target = match lang {
                        Lang::Rust => crate::imports::rust_path_target(&file, spec, root),
                        _ => crate::imports::resolve(lang, &file, spec, root),
                    };
                    if let Some(t) = target {
                        names.entry(local).or_insert((t, name));
                    }
                }
                crate::imports::Import::Base { module, at } => {
                    if registry.classes.bases_here.contains(module) {
                        bases.push((module, *at));
                    } else {
                        registry
                            .classes
                            .named_by
                            .entry(module.clone())
                            .or_default()
                            .insert(file.clone());
                    }
                }
                crate::imports::Import::Contract { name } => {
                    registry.classes.contracts.insert(qualify(&file, name));
                }
                crate::imports::Import::Mixin { name } => {
                    registry.classes.mixins.insert(qualify(&file, name));
                    mixed.extend(
                        facts
                            .ranges
                            .iter()
                            .filter(|(n, _)| n == name)
                            .map(|(_, span)| *span),
                    );
                }
                crate::imports::Import::Qualified { class, package } if class == "*" => {
                    open_packages.push(package);
                }
                crate::imports::Import::Qualified { class, package } => {
                    packages.insert(class, package);
                }
                crate::imports::Import::Method {
                    class,
                    name,
                    receiver,
                    at,
                } => {
                    let directory = file.rsplit_once('/').map_or("", |(d, _)| d);
                    let files = registry
                        .classes
                        .methods
                        .entry(format!("{directory}#{class}"))
                        .or_default()
                        .entry(name.clone())
                        .or_default();
                    if !files.contains(&file) {
                        files.push(file.clone());
                    }
                    receivers.insert(*at, (receiver, class));
                }
                crate::imports::Import::Extension { name } => {
                    registry.classes.extensions.insert(qualify(&file, name));
                }
            }
        }
    }
    // A base defined twice means the one beside this file. Where a file must
    // import what is not in its own package, the import or that package decides
    // even for a single candidate: `^/a/b|/c` lists the packages to look in.
    let dir = file.rsplit_once('/').map_or("", |(d, _)| d);
    let by_package = matches!(lang, Some(Lang::Java | Lang::Kotlin | Lang::Php));
    let bases: Vec<(String, u32)> = bases
        .into_iter()
        .map(|(module, at)| {
            if by_package {
                let looked_in = match packages.get(module) {
                    Some(package) => (*package).to_string(),
                    None => std::iter::once(packages.get(".").copied().unwrap_or(dir))
                        .chain(open_packages.iter().copied().take(8))
                        .collect::<Vec<_>>()
                        .join("|/"),
                };
                return (format!("{module}^/{looked_in}"), at);
            }
            let shared = registry.classes.bases_twice.contains(module) && !dir.contains('+');
            let hinted = format!("{module}^{dir}");
            (if shared { hinted } else { module.to_string() }, at)
        })
        .collect();
    // Who declares each base: the innermost definition around the declaration.
    // No real file comes near this many, and each one costs a walk of `ranges`.
    const MOST_BASES: usize = 4096;
    let owners: Vec<(&str, (u32, u32))> = bases
        .iter()
        .take(MOST_BASES)
        .map(|(module, at)| {
            let (module, at) = (module.as_str(), *at);
            let around = facts
                .ranges
                .iter()
                .map(|(_, span)| *span)
                .filter(|s| s.0 <= at && at <= s.1)
                .min_by_key(|s| s.1 - s.0);
            (module, around.unwrap_or((0, u32::MAX)))
        })
        .collect();
    for &(module, span) in &owners {
        if let Some((class, _)) = facts.ranges.iter().find(|(_, s)| *s == span) {
            let key = qualify(&file, class);
            let bases = registry.classes.class_bases.entry(key).or_default();
            if !bases.iter().any(|b| b == module) {
                bases.push(module.to_string());
            }
        }
    }
    let mut spans_of: HashMap<&str, Vec<(u32, u32)>> = HashMap::new();
    if !owners.is_empty() {
        for (name, span) in &facts.ranges {
            spans_of.entry(name).or_default().push(*span);
        }
    }
    // A caller's bases, for the definition making the call. Scanners that do
    // not say which one leave every definition of the name in play: tier 3
    // links only a name exactly one of them defines.
    let inherited_by = |caller: &str, owner: Option<u32>| -> Option<String> {
        let spans = spans_of.get(caller)?;
        let mut modules: Vec<&str> = owners
            .iter()
            .filter(|(_, o)| {
                spans
                    .iter()
                    .filter(|s| owner.is_none_or(|start| s.0 == start))
                    .any(|s| o.0 <= s.0 && s.1 <= o.1)
            })
            .map(|&(module, _)| module)
            .collect();
        modules.sort_unstable();
        modules.dedup();
        (!modules.is_empty()).then(|| format!("@{}", modules.join("+")))
    };
    let mut inherited_of: HashMap<(&str, Option<u32>), Option<String>> = HashMap::new();

    // What this file defines now. The registry may still hold a name the file
    // defined before it was read again: a call to it is no local call.
    let defined_here: HashSet<&str> = facts.defines.iter().map(String::as_str).collect();
    let mut edges: Vec<(NodeId, Edge)> = Vec::with_capacity(facts.calls.len());
    for (i, (caller, callee, has_receiver)) in facts.calls.iter().enumerate() {
        let source_node = registry.get_or_mint(&qualify(&file, caller));
        let local = qualify(&file, callee);
        // A call through an import names its file, so its placeholder says
        // which file: tier 3 then links it there instead of refusing a name
        // several files define.
        let module = facts.call_modules.get(i).and_then(Option::as_deref);
        let owner = facts.call_owners.get(i).copied().flatten();
        let inherited = inherited_of
            .entry((caller, owner))
            .or_insert_with(|| inherited_by(caller, owner))
            .clone();
        let names_base = matches!(module, Some("super" | "base" | "parent"));
        let in_base = names_base && inherited.is_some();
        // `super.run()` in `run`, with the base in this file: one node holds
        // both, and the call says nothing the node does not.
        if in_base && caller == callee && !bases.is_empty() {
            let here = |m: &&str| {
                let m = m.split('^').next().unwrap_or(m);
                facts.defines.iter().any(|d| d == m)
            };
            if inherited
                .as_deref()
                .is_some_and(|t| t[1..].split('+').all(|m| here(&m)))
            {
                continue;
            }
        }
        // Named through the object, or in a language without free functions
        // and not built with `new`: only the class chain can hold it.
        let through_object = module.is_some_and(|m| m != "new");
        let in_mixin = owner.is_some_and(|at| mixed.iter().any(|s| s.0 <= at && at < s.1));
        let chain_only = !in_mixin
            && (through_object
                || (matches!(lang, Some(Lang::Java | Lang::CSharp)) && module != Some("new")));
        let imported = if in_base {
            // `!`: the base or nothing. The bare name would be this very
            // method when the base holds it outside the tree.
            inherited
                .as_deref()
                .map(|t| crate::imports::scoped(callee, &t.replacen('@', "@!", 1)))
        } else if names_base && caller == callee {
            Some(crate::imports::scoped(callee, "@!"))
        } else if let Some((_, class)) = owner
            .and_then(|at| receivers.get(&at))
            .filter(|(receiver, _)| Some(receiver.as_str()) == module)
        {
            Some(crate::imports::scoped(callee, &format!("@?{class}^{dir}")))
        } else if *has_receiver {
            module
                .and_then(|m| {
                    let value = m.strip_prefix('.');
                    // A Rust path names a module, never a typed value of
                    // that name: `physics::expand` beside `physics: Physics`.
                    let path = lang == Some(Lang::Rust) && value.is_none();
                    modules
                        .get(value.unwrap_or(m))
                        .filter(|_| !path)
                        .cloned()
                        .or_else(|| {
                            // `alias Shop.Billing` then `Billing.Tax.rate()`.
                            let (head, tail) = m.split_once('.')?;
                            let base = modules.get(head)?;
                            base.starts_with('@').then(|| format!("{base}.{tail}"))
                        })
                        .or_else(|| {
                            lang.and_then(|l| crate::imports::module_target(l, &file, m, root))
                        })
                })
                // `.`: the receiver's own type, which a type outside the
                // tree holds rather than a definition sharing the name.
                .map(|t| match t.strip_prefix('@') {
                    Some(rest) if !rest.starts_with(['!', '?']) => {
                        crate::imports::scoped(callee, &format!("@.{rest}"))
                    }
                    _ => crate::imports::scoped(callee, &t),
                })
        } else if !defined_here.contains(callee.as_str()) {
            names
                .get(callee.as_str())
                .map(|(t, n)| crate::imports::scoped(n, t))
                .or_else(|| {
                    // There a method is reached through the object only: a
                    // bare name is a global, whatever the base defines.
                    let bare_is_global = matches!(
                        lang,
                        Some(Lang::Python | Lang::JavaScript | Lang::TypeScript)
                    );
                    inherited
                        .as_deref()
                        .filter(|_| !bare_is_global || through_object)
                        .filter(|_| registry.may_define(callee))
                        .map(|t| {
                            if chain_only {
                                crate::imports::scoped(callee, &t.replacen('@', "@?", 1))
                            } else {
                                crate::imports::scoped(callee, t)
                            }
                        })
                })
        } else {
            None
        };
        // A method on a receiver whose type is unknown may be any type's,
        // the standard library's first: the name alone decides nothing.
        // A Rust path into the tree (`regs::bpu()`, `crate::a::f()`, `Type::new()`)
        // names a module or type, not a value; `std::fs::read_dir` names none.
        let path_call = lang == Some(Lang::Rust)
            && module.is_some_and(|m| {
                let head = m.split("::").next().unwrap_or(m);
                !m.starts_with('.')
                    && (["crate", "self", "super"].contains(&head)
                        || head.starts_with(|c: char| c.is_ascii_uppercase())
                        || crate::imports::module_target(Lang::Rust, &file, head, root).is_some())
            });
        let unknown_receiver = *has_receiver && imported.is_none() && !path_call;
        // `super.run()` in `run` never means this `run`.
        let target = if !*has_receiver && !names_base && defined_here.contains(callee.as_str()) {
            registry.get_or_mint(&local)
        } else if let Some(scoped) = imported {
            let node = registry.get_or_mint(&scoped);
            if let Some(lang) = lang {
                registry.note_placeholder_lang(node, lang);
            }
            node
        } else {
            // Not defined here: a bare name, shared across files, so a later
            // file defining it lands on the same node. Which language reached
            // it is recorded beside the node, because tier 3 links by name
            // alone and a name means different things in different languages.
            let node = registry.get_or_mint(callee);
            if let Some(lang) = Lang::from_path(path) {
                registry.note_placeholder_lang(node, lang);
            }
            node
        };
        edges.push((
            source_node,
            Edge {
                target,
                timestamp: now,
                authority: crate::physics::SOURCE_CODE,
                edge_kind: 0,
                confidence: if unknown_receiver {
                    crate::csr::Confidence::Ambiguous
                } else {
                    parse_ast::TIER_CONFIDENCE
                },
            },
        ));
    }

    // A shell script is run as a whole, so its top level is a node even
    // when it calls nothing: a workflow step that runs it refers to that.
    if matches!(
        lang,
        Some(Lang::Bash | Lang::PowerShell | Lang::Batch | Lang::Fish)
    ) {
        registry.get_or_mint(&qualify(&file, "<module>"));
    }

    // A step that runs or uses a file depends on it: the reference goes to
    // that file's top level, through a placeholder tier 3 links there.
    let mut seen_refs = std::collections::HashSet::new();
    for (caller, written) in &facts.file_refs {
        let Some(target) = crate::workflow::target(root, written) else {
            continue;
        };
        if !seen_refs.insert((caller, target.clone())) {
            continue;
        }
        let source_node = registry.get_or_mint(&qualify(&file, caller));
        let placeholder = crate::imports::scoped("<module>", &target);
        edges.push((
            source_node,
            Edge {
                target: registry.get_or_mint(&placeholder),
                timestamp: now,
                authority: crate::physics::SOURCE_CODE,
                edge_kind: 0,
                confidence: parse_ast::TIER_CONFIDENCE,
            },
        ));
    }

    // After the calls, which mint `<module>` when the top level calls
    // anything. Only from a definition that exists: minting `<module>` for a file whose
    // top level merely names something would add a node nothing calls.
    let mut node_of: HashMap<u32, Option<NodeId>> = HashMap::new();
    let refs = facts
        .refs
        .iter()
        .filter_map(|(from, name, qualifier)| {
            let node = (*node_of.entry(*from).or_insert_with(|| {
                let def = match *from {
                    parse_ast::MODULE => "<module>",
                    k => facts
                        .ranges
                        .get(k as usize)
                        .map_or("<module>", |(n, _)| n.as_str()),
                };
                registry.node_of(&qualify(&file, def))
            }))?;
            let name: &str = name;
            // Through an import the name is decided, however many files
            // define it: `util.Primitive` after `import * as util from
            // "./util.js"`, or a name imported directly.
            let through = match qualifier {
                Some(q) => modules.get(&**q).map(|t| crate::imports::scoped(name, t)),
                None => names.get(name).map(|(t, n)| crate::imports::scoped(n, t)),
            };
            Some((node, through.unwrap_or_else(|| name.to_string())))
        })
        .collect::<Vec<_>>();
    // Overloads share a node, so their uses are one set.
    let mut refs = refs;
    refs.sort_unstable();
    refs.dedup();
    registry.set_refs(&file, refs);
    registry.set_definers(&file, facts.macro_definers.clone());

    // Handlers are kept as symbols: one passed by name may live in another
    // file, and only the served tree knows which.
    let def_name = |k: u32| match k {
        crate::routes::MODULE => "<module>",
        k => facts
            .ranges
            .get(k as usize)
            .map_or("<module>", |(n, _)| n.as_str()),
    };
    let http = FileHttp {
        routes: facts
            .http
            .routes
            .iter()
            .map(|(handler, verb, path)| {
                let symbol = match handler {
                    crate::routes::Handler::Def(k) => qualify(&file, def_name(*k)),
                    crate::routes::Handler::Name(n) => n.clone(),
                };
                (symbol, *verb, path.clone())
            })
            .collect(),
        requests: facts
            .http
            .requests
            .iter()
            .filter_map(|(k, verb, path)| {
                Some((
                    registry.node_of(&qualify(&file, def_name(*k)))?,
                    *verb,
                    path.clone(),
                ))
            })
            .collect(),
    };
    registry.set_http(&file, http);
    let in_file = |registry: &SymbolRegistry, name: &str| registry.node_of(&qualify(&file, name));
    let dispatch = FileDispatch {
        stores: facts.stores.clone(),
        calls: facts
            .dispatch
            .iter()
            .filter_map(|(r, label, fields, line)| {
                Some((in_file(registry, r)?, label.clone(), fields.clone(), *line))
            })
            .collect(),
        unresolved: facts
            .indirect
            .iter()
            .filter_map(|(r, line)| Some((in_file(registry, r)?, *line)))
            .collect(),
        tables: facts
            .tables
            .iter()
            .filter_map(|t| in_file(registry, t))
            .collect(),
        constants: facts.constants.clone(),
        slots: facts
            .slots
            .iter()
            .filter_map(|(t, at, to)| Some((in_file(registry, t)?, *at, to.clone())))
            .collect(),
        passes: facts
            .passes
            .iter()
            .filter_map(|(from, to, reg, label, fields)| {
                Some((
                    in_file(registry, from)?,
                    to.clone(),
                    reg.clone(),
                    label.clone(),
                    fields.clone(),
                ))
            })
            .collect(),
        forwards: facts
            .forwards
            .iter()
            .filter_map(|(from, to, mask)| Some((in_file(registry, from)?, to.clone(), *mask)))
            .collect(),
        param_calls: facts
            .param_calls
            .iter()
            .filter_map(|(r, reg, fields, line)| {
                Some((in_file(registry, r)?, reg.clone(), fields.clone(), *line))
            })
            .collect(),
        param_stores: facts
            .param_stores
            .iter()
            .filter_map(|(r, reg, field)| Some((in_file(registry, r)?, reg.clone(), field.clone())))
            .collect(),
        library: facts
            .library_calls
            .iter()
            .filter_map(|(r, vector, line)| Some((in_file(registry, r)?, vector.clone(), *line)))
            .collect(),
    };
    registry.set_dispatch(&file, dispatch);

    let file_key = arena.intern(&file);
    (file_key, edges, nodes)
}

/// The file's previous definitions that this parse did not produce again.
fn dropped(previous: &[NodeId], now: &[NodeId]) -> HashSet<NodeId> {
    let now: HashSet<NodeId> = now.iter().copied().collect();
    previous
        .iter()
        .copied()
        .filter(|n| !now.contains(n))
        .collect()
}

/// The delta mutation one parsed file makes: drop what the file owned, then
/// add what it now defines and calls.
///
/// Separate so a whole-tree build can run thousands of these against one
/// delta — see `Graph::update_batch`. The logic is identical either way; only
/// how often the snapshot is republished differs.
pub fn scope_file(
    snap: &GraphSnapshot,
    d: &mut DeltaStore,
    file_key: u32,
    edges: &[(NodeId, Edge)],
    nodes: &[NodeId],
) {
    let targets: HashMap<NodeId, Vec<NodeId>> = d
        .file_nodes(file_key)
        .iter()
        .map(|&n| (n, snap.base_targets(n)))
        .collect();
    d.replace_file_edges(
        file_key,
        |n| targets.get(&n).cloned().unwrap_or_default(),
        edges.to_vec(),
        nodes,
    );
}

/// `apply_facts` without publishing: the caller supplies the delta.
///
/// Used by `build_graph`, where nothing can read the graph yet, so one copy
/// for the whole tree replaces one per file.
#[allow(clippy::too_many_arguments)]
pub fn apply_facts_into(
    snap: &GraphSnapshot,
    d: &mut DeltaStore,
    arena: &mut SymbolArena,
    registry: &mut SymbolRegistry,
    path: &Path,
    root: &Path,
    facts: parse_ast::FileFacts,
    now: u64,
) {
    let (file_key, edges, nodes) = prepare_facts(arena, registry, path, root, facts, now);
    registry.forget_nodes(&dropped(d.file_nodes(file_key), &nodes));
    scope_file(snap, d, file_key, &edges, &nodes);
}

pub fn apply_facts(
    graph: &Arc<Graph>,
    arena: &mut SymbolArena,
    registry: &mut SymbolRegistry,
    path: &Path,
    root: &Path,
    facts: parse_ast::FileFacts,
    now: u64,
) -> usize {
    let (file_key, edges, nodes) = prepare_facts(arena, registry, path, root, facts, now);
    registry.forget_nodes(&dropped(graph.load().delta.file_nodes(file_key), &nodes));
    let count = edges.len();
    graph.update(|snap, d| scope_file(snap, d, file_key, &edges, &nodes));
    count
}
