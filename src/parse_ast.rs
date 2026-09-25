//! Tier 2 of the cascade parser: extraction by hand-written scanners.
//!
//! Runs when tier 1 (compiler-grade SCIP/LSP data) is unavailable — a partial
//! build, a missing toolchain, a file mid-edit. A scanner is error-tolerant by
//! construction, so a syntactically broken file still yields the facts it can.
//! Everything it produces is `Confidence::Inferred`: the shapes are real but
//! the resolution is by name, without a compiler's symbol table.
//!
//! The scanners live in `parsers/`, one per language, with no grammar and no
//! dependency of their own. This module is what remains here: the language
//! table, the fact contract, and the mapping from one to the other. Registering
//! a language is an extension and an arm in `native`.
//!
//! Deliberately syntactic. Resolving a call to a definition across files is
//! tier 3's job; here an unqualified callee name is emitted as-is and the
//! registry decides whether it names something already known.

use crate::csr::Confidence;
use std::path::Path;

/// What a file defines and what it references, both as plain names. Resolution
/// to node ids happens against the registry, not here.
#[derive(Debug, Default, PartialEq)]
pub struct FileFacts {
    /// Definitions in this file: functions, methods, classes, modules.
    pub defines: Vec<String>,
    /// Byte range per definition, by name, so a tool can hand back the source
    /// a symbol names instead of only its name. Parallel to `docs` rather than
    /// folded into `defines`, which a dozen asserts read as a list of names.
    pub ranges: Vec<(String, (u32, u32))>,
    /// Documentation text per definition, by name. What a symbol is *for* is
    /// written in prose above it, not in its identifier, and a question is
    /// asked in that prose — measured: seed discovery recalled 8% on questions
    /// phrased as a person asks them against 72% on ones naming the
    /// identifiers, and the gap is entirely this text.
    pub docs: Vec<(String, String)>,
    /// (caller, callee, has_receiver) triples. The caller is a name from
    /// `defines`; a callee is whatever the call site named.
    ///
    /// `has_receiver` is true when the call reached the name *through*
    /// something — `Instant::now()`, `registry.docs()`, `$o->get()`. The
    /// receiver's type is not resolvable syntactically, but its presence is,
    /// and that alone is what the same-file rule needs: a call with a receiver
    /// is not a call to the local function of that name. See `has_receiver`.
    pub calls: Vec<(String, String, bool)>,
    /// The module a `::` call went through, parallel to `calls`.
    ///
    /// A module is a file in Rust, so `parse_ast::parse` names `parse` in
    /// `parse_ast.rs` — the language definition, not a heuristic. Beside the
    /// callee rather than inside it, because every consumer reads the bare
    /// name and `charge -> process` must stay `process`.
    pub call_modules: Vec<Option<String>>,
    /// Import statements, for resolving a call through a module to the file it
    /// names. See `imports`.
    pub imports: Vec<crate::imports::Import>,
    /// (enclosing definition, name, qualifier) for every identifier outside
    /// comments and strings that is not in call position, once per triple. What a definition
    /// uses without calling it — a type, a constant, a base class; resolved
    /// against the whole tree later, since only then is it known what a name
    /// names. See `references`.
    pub refs: Vec<(String, String, Option<String>)>,
    /// True if the scanner reported malformed source. The facts are still usable —
    /// that is the point of this tier — but a caller may prefer tier 1 output.
    pub had_errors: bool,
}

/// A language we can extract from.
///
/// The scanner crate's own language, re-exported: it already carries the
/// extension table for all seventy-two, and a second copy here would be a table
/// that silently falls behind — measured, this one offered twenty while the
/// scanners covered seventy-two, so Julia, Perl, SQL and forty-nine others were
/// built but unreachable. Registering a language is an entry in
/// `parsers/src/lib.rs` and nothing else.
pub type Lang = native_parsers::Language;

/// The two things this codebase asks of a language beyond what the crate
/// offers: which one a path is, and how many there are.
///
/// A trait rather than free functions, so `Lang::from_path` and `Lang::count`
/// read as they always did at their eighty-odd call sites.
pub trait LangExt: Sized {
    fn from_path(path: &Path) -> Option<Self>;
    fn count() -> usize;
}

/// Formats the scanners parse but the graph does not want: they yield
/// definitions and no edges, which is a heap of nodes rather than a graph.
///
/// Measured on this repository: `Cargo.toml` alone contributes 22 nodes and
/// `ci.yml` 33, all of them named `name`, `run`, `with`, `steps` — generic
/// words that compete with real code in a BM25 index and cost 3 points of
/// partition purity for answering nothing. Markdown is handled by `docs.rs`,
/// which reads it as prose rather than as code.
/// Formats refused before any scanner sees them.
///
/// **The list used to hold every non-code format on one measurement, and that
/// measurement was too coarse.** Excluding all of them together cost 4 points
/// of partition purity, which was attributed to the group; measured one at a
/// time, **only TOML does it** — `Cargo.toml` contributes `name`, `version`
/// and `dependencies`, names every other manifest in a tree also carries, and
/// a shared foreign name is what welds unrelated files into one community.
/// JSON, YAML and XML each cost nothing, and CSS and HTML cost nothing either.
///
/// What that mistake hid is the question a front-end user actually asks. A
/// tree is not only code: measured on a four-file web project, "welche Farbe
/// hat der btn-primary Button" returned `app.js#onCheckout` at confidence
/// 1.000 — a confidently wrong answer, with nothing saying the stylesheet was
/// never read. It now returns `styles/main.css#.btn-primary` and
/// `get_code_snippet` hands back the rule with its declarations.
///
/// Markdown stays here because `docs.rs` reads it as prose instead, which is a
/// different and better treatment, not an exclusion.
const NOT_CODE: &[&str] = &["toml", "md", "markdown", "csv", "lock"];

/// Files a language identifies by *name* rather than by extension. Registering
/// `dockerfile` in the extension table looked right and could never fire —
/// `Dockerfile` has no extension at all, so `path.extension()` returns `None`
/// and the scanner was unreachable from every indexing path. Same class of
/// defect as the fifty-two languages that were built and never offered a file,
/// and invisible for the same reason: an unreachable scanner yields no error,
/// only silence.
const BY_FILENAME: &[(&str, &str)] = &[
    ("dockerfile", "dockerfile"),
    ("containerfile", "dockerfile"),
    ("makefile", "mk"),
    ("gnumakefile", "mk"),
    ("justfile", "just"),
    ("rakefile", "rb"),
    ("gemfile", "rb"),
    ("vagrantfile", "rb"),
    ("brewfile", "rb"),
    ("cmakelists.txt", "cmake"),
    // A requirements file and a Kconfig are identified by their name: `.txt`
    // is any text at all, and `Kconfig` has no extension. Registering `.txt`
    // instead made every text file a dependency list, which a self-check
    // caught immediately.
    ("requirements.txt", "requirements"),
    ("requirements-dev.txt", "requirements"),
    ("constraints.txt", "requirements"),
    ("kconfig", "kconfig"),
    ("meson.build", "build"),
];

impl LangExt for Lang {
    fn from_path(path: &Path) -> Option<Self> {
        // **The whole file name is checked first, extension or not.** Some
        // names identify a language while their extension identifies nothing:
        // `requirements.txt` is a dependency list and `.txt` is any text at
        // all. Registering the extension instead made every text file a
        // dependency list, which a self-check caught at once.
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            let lower = name.to_lowercase();
            if let Some((_, ext)) = BY_FILENAME.iter().find(|(n, _)| *n == lower) {
                return Lang::from_extension(ext);
            }
        }
        let ext = path.extension().and_then(|e| e.to_str())?;
        if NOT_CODE.contains(&ext.to_lowercase().as_str()) {
            return None;
        }
        Lang::from_extension(ext)
    }

    fn count() -> usize {
        native_parsers::Language::count()
    }
}

/// Extensions several languages claim, with a marker that proves the file is
/// *not* the language the table guessed.
///
/// Measured on the corpus: 196 files, every one misread. `.v` is Rocq far more
/// often than Verilog (163 files against 0), `.m` is Objective-C rather than
/// Matlab, `.d` is DTrace or a Make dependency. The scanner is not broken —
/// it is handed the wrong file, which no fixture can show and which
/// `langcheck` found in one run: five languages at 100% `<module>` with every
/// file yielding nothing.
///
/// A refusal rather than a re-guess: the marker says what the file is not, and
/// naming what it *is* would mean a second table of languages we do not scan.
/// A file dropped here contributes nothing, which is what it already did.
const AMBIGUOUS: &[(&str, &[&str])] = &[
    // Rocq/Coq vocabulary; Verilog has none of these words.
    (
        "v",
        &[
            "Definition ",
            "Theorem ",
            "Lemma ",
            "Require ",
            "Inductive ",
            "Proof.",
            "Variant ",
            "Notation ",
            "Fixpoint ",
            "Module ",
            "Axiom ",
            "Coercion ",
        ],
    ),
    // Objective-C; Matlab has no preprocessor and no `@interface`.
    (
        "m",
        &[
            "#import",
            "@import",
            "@interface",
            "@implementation",
            "#include",
        ],
    ),
    // DTrace, and a Make dependency file, neither of which is the language D.
    (
        "d",
        &[
            "provider ",
            "#pragma D",
            "dtrace:",
            "dtrace ",
            "::process-",
            "BEGIN\n",
            "self->",
            ".o:",
        ],
    ),
];

/// Extensions where the *language* must identify itself, because the formats
/// sharing the extension carry no marker of their own.
///
/// `.res` is the measured case: of 51 such files in a 1.8 GB corpus, **not one
/// is ReScript** — 34 are Godot's binary resources, 6 are Windows resource
/// files in Nim's tree, and 13 are Scala test expectations holding a list of
/// file names. A binary has no stable head to match on, so the test is turned
/// around: a file claiming to be ReScript must read like one.
const MUST_IDENTIFY: &[(&str, &[&str])] = &[(
    "res",
    &[
        "let ",
        "open ",
        "module ",
        "type ",
        "external ",
        "@react",
        "->",
    ],
)];

/// Whether the extension table's guess is contradicted by the file itself.
///
/// Only the head is read: a marker that identifies the file appears at the top
/// in every case measured, and scanning a megabyte to classify it would cost
/// more than the parse it is guarding.
fn misidentified(path: &Path, src: &str) -> bool {
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return false;
    };
    // **`get`, never an index.** A 4,096-byte cut lands inside a character
    // whenever one straddles it, and the corpus found that at once: a Latin-1
    // resource file decoded into a string whose 4,095th byte opens a `¢`.
    // Same class as the eleven crashes recorded under the corpus sweep, and
    // the reason every slice site here goes through a checked accessor.
    let head = src.get(..src.len().min(4096)).unwrap_or_else(|| {
        let mut end = src.len().min(4096);
        while end > 0 && !src.is_char_boundary(end) {
            end -= 1;
        }
        &src[..end]
    });
    if let Some((_, markers)) = MUST_IDENTIFY.iter().find(|(e, _)| *e == ext) {
        // Nothing that reads like the language: treat it as one of the other
        // formats sharing the extension and parse nothing.
        return !markers.iter().any(|m| head.contains(m));
    }
    let Some((_, markers)) = AMBIGUOUS.iter().find(|(e, _)| *e == ext) else {
        return false;
    };
    markers.iter().any(|m| head.contains(m))
}

pub fn parse_file(path: &Path, src: &str, lang: Lang) -> Option<FileFacts> {
    if misidentified(path, src) {
        return None;
    }
    parse(src, lang)
}

/// Parses a source file without a compiler: what it defines and what it
/// calls, by shape rather than by resolution.
pub fn parse(src: &str, lang: Lang) -> Option<FileFacts> {
    // Through the rule set, never `native_parsers::parse` directly: the rules
    // are what an operator's `--language-rules` override replaces, and calling
    // the crate's own dispatch bypasses them silently. Measured — a keyword
    // removed from `go.toml` changed nothing on any indexing path, while the
    // snapshot still discarded itself over the changed rule identity.
    let f = native_parsers::rules::active().parse(lang, src);
    let refs = references(lang, src, &f.ranges);
    Some(FileFacts {
        defines: f.defines,
        ranges: f.ranges,
        docs: f.docs,
        calls: f.calls,
        call_modules: f.call_modules,
        imports: crate::imports::read(lang, src),
        refs,
        had_errors: f.had_errors,
    })
}

/// Each identifier attributed to the innermost definition whose range holds
/// it, `<module>` outside all of them. One sweep: identifiers arrive in
/// order, and ranges sorted by start nest.
fn references(
    lang: Lang,
    src: &str,
    ranges: &[(String, (u32, u32))],
) -> Vec<(String, String, Option<String>)> {
    let idents = native_parsers::rules::active().identifiers(lang, src);
    let mut sorted: Vec<&(String, (u32, u32))> = ranges.iter().collect();
    sorted.sort_by_key(|(_, (start, end))| (*start, std::cmp::Reverse(*end)));
    let mut open: Vec<&(String, (u32, u32))> = Vec::new();
    let mut next = 0;
    let mut out: std::collections::HashSet<(&str, &str, Option<&str>)> = Default::default();
    for (at, name, qualifier) in idents {
        while next < sorted.len() && sorted[next].1.0 <= at {
            open.push(sorted[next]);
            next += 1;
        }
        open.retain(|(_, (_, end))| *end > at);
        let enclosing = open.last().map_or("<module>", |(n, _)| n.as_str());
        // Two letters name a loop variable, not a symbol worth an edge.
        if name != enclosing && name.len() > 2 {
            out.insert((enclosing, name, qualifier));
        }
    }
    let mut refs: Vec<(String, String, Option<String>)> = out
        .into_iter()
        .map(|(a, b, q)| (a.to_string(), b.to_string(), q.map(str::to_string)))
        .collect();
    refs.sort_unstable();
    refs
}

/// Tier 2 is syntactic: shapes are real, resolution is by name.
pub const TIER_CONFIDENCE: Confidence = Confidence::Inferred;
