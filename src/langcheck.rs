//! Per-language yield against real code, as a repeatable command.
//!
//! The floors in `bench/baseline.txt` measure retrieval on *this* tree, which
//! is Rust. They cannot see a scanner that stopped finding definitions in
//! Julia — measured, six of the seven read only this repository. What found
//! the thirteen defects the fixtures missed was a sweep over 5.2 million lines
//! of foreign code, counting definitions and edges per 1,000 lines per
//! language; it existed once, as throwaway code, and is what this file makes
//! repeatable.
//!
//! Three numbers per language, all per 1,000 lines so trees of different sizes
//! compare: definitions, edges, and the share of references attributed to
//! `<module>` rather than to an enclosing definition. The third is the one that
//! catches a silent break — `<module>` is a valid node, so a scanner that stops
//! recognising a language's function keyword reports no error at all, it just
//! attributes everything to the file.
//!
//! A number that looks wrong is worth *splitting* before it is worth fixing.
//! Ruby read 76% `<module>` until the corpus was split: 0% in `lib/` and 100%
//! in `spec/`, where RSpec's blocks genuinely have no enclosing method. Perl
//! read 74% for scripts against 25% for modules. Hence `--by-dir`.

use crate::parse_ast::{Lang, LangExt};
use std::collections::BTreeMap;

/// What one language yielded across every file of that language.
#[derive(Default, Clone)]
pub struct Yield {
    pub files: usize,
    pub lines: usize,
    pub defines: usize,
    pub calls: usize,
    /// Calls whose caller is `<module>` rather than a named definition.
    pub module_calls: usize,
    /// Files that produced no definition at all. A language where this is most
    /// of them is broken even when the totals look plausible.
    pub empty_files: usize,
}

impl Yield {
    pub fn defs_per_kloc(&self) -> f32 {
        per_kloc(self.defines, self.lines)
    }
    pub fn edges_per_kloc(&self) -> f32 {
        per_kloc(self.calls, self.lines)
    }
    /// Share of references landing on `<module>`, in percent.
    pub fn module_share(&self) -> f32 {
        if self.calls == 0 {
            return 0.0;
        }
        100.0 * self.module_calls as f32 / self.calls as f32
    }
}

fn per_kloc(count: usize, lines: usize) -> f32 {
    if lines == 0 {
        return 0.0;
    }
    1000.0 * count as f32 / lines as f32
}

/// The name the scanners use for a call with no enclosing definition. Must
/// match what `ScopeStack` emits, or the share reads zero and says nothing.
const MODULE: &str = "<module>";

/// Parses every source file under `root`, grouped by language.
///
/// Deliberately not the ingest path: no registry, no graph, no snapshot. What
/// is being checked is the scanner, and routing through ingestion would let a
/// resolution bug mask an extraction one.
pub fn measure(root: &std::path::Path, by_dir: bool) -> BTreeMap<String, Yield> {
    let mut out: BTreeMap<String, Yield> = BTreeMap::new();
    for path in crate::walk(root) {
        let Some(lang) = Lang::from_path(&path) else {
            continue;
        };
        let Some(src) = crate::read_source(&path) else {
            continue;
        };
        let Some(facts) = crate::parse_ast::parse_file(&path, &src, lang) else {
            continue;
        };
        let key = if by_dir {
            format!("{:?} {}", lang, top_dir(root, &path))
        } else {
            format!("{lang:?}")
        };
        let e = out.entry(key).or_default();
        e.files += 1;
        e.lines += src.lines().count();
        e.defines += facts.defines.len();
        e.calls += facts.calls.len();
        e.module_calls += facts.calls.iter().filter(|(c, _, _)| c == MODULE).count();
        if facts.defines.is_empty() {
            e.empty_files += 1;
        }
    }
    out
}

/// The first path segment below the root — `lib` against `spec`, which is what
/// splitting Ruby's `<module>` rate needed.
fn top_dir(root: &std::path::Path, path: &std::path::Path) -> String {
    path.strip_prefix(root)
        .ok()
        .and_then(|r| {
            r.components()
                .next()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| ".".to_string())
}

/// One expectation: a lower bound per language, in the same shape as
/// `bench/baseline.txt` — a measured value, not a goal.
pub struct Expectation {
    pub lang: String,
    pub min_defs: f32,
    pub min_edges: f32,
    /// Upper bound on the `<module>` share. C headers legitimately sit at 70%,
    /// so this is per language rather than one global limit.
    pub max_module: f32,
}

/// Reads `bench/languages.txt`: `<lang> <min_defs> <min_edges> <max_module>`.
pub fn load_expectations(path: &std::path::Path) -> std::io::Result<Vec<Expectation>> {
    let text = std::fs::read_to_string(path)?;
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split_whitespace().collect();
        if let [lang, d, e, m] = f.as_slice()
            && let (Ok(min_defs), Ok(min_edges), Ok(max_module)) = (d.parse(), e.parse(), m.parse())
        {
            out.push(Expectation {
                lang: lang.to_string(),
                min_defs,
                min_edges,
                max_module,
            });
        }
    }
    Ok(out)
}

/// Compares a measurement against the expectations, returning what broke.
///
/// A language present in the file but absent from the tree is *not* a failure:
/// the expectations describe a corpus, and a caller may point this at one
/// holding a subset. A language in the tree with no expectation is reported as
/// unchecked, which is the honest state for the fifty-odd that have none.
pub fn check(measured: &BTreeMap<String, Yield>, expected: &[Expectation]) -> Vec<String> {
    let mut broken = Vec::new();
    for e in expected {
        let Some(y) = measured.get(&e.lang) else {
            continue;
        };
        if y.defs_per_kloc() < e.min_defs {
            broken.push(format!(
                "{}: {:.0} definitions per 1,000 lines, floor {:.0}",
                e.lang,
                y.defs_per_kloc(),
                e.min_defs
            ));
        }
        if y.edges_per_kloc() < e.min_edges {
            broken.push(format!(
                "{}: {:.0} edges per 1,000 lines, floor {:.0}",
                e.lang,
                y.edges_per_kloc(),
                e.min_edges
            ));
        }
        if y.module_share() > e.max_module {
            broken.push(format!(
                "{}: {:.0}% of references on <module>, ceiling {:.0}%",
                e.lang,
                y.module_share(),
                e.max_module
            ));
        }
    }
    broken
}
