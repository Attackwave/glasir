//! File system watcher.
//!
//! Editors emit a burst of writes while someone types. A portable polling
//! watcher compares a compact file stamp every `DEBOUNCE` and reports one
//! deduplicated batch. It has the same semantics on Linux, macOS and Windows,
//! and avoids a platform-specific watcher dependency tree.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Quiet period before a burst of file saves is reported as one batch, so
/// editing does not start one rebuild per save.
pub const DEBOUNCE: Duration = Duration::from_millis(250);

/// Whether a path is one the indexer must never read. See `ignored_under`,
/// which this defers to without a tree to be relative to.
pub fn is_ignored(path: &Path) -> bool {
    ignored_under(path, None)
}

/// `is_ignored`, but knowing which tree the path belongs to.
///
/// `tmp` is only scratch space *inside a repository*. Judged on the full path
/// it would also exclude `/tmp`, where every self-check builds its fixtures —
/// which is exactly what happened when this was first written, and eleven
/// checks failed at once. So the name-anywhere rules apply to the whole path,
/// and the root-relative ones only below `root`.
pub fn ignored_under(path: &Path, root: Option<&Path>) -> bool {
    /// Never source, wherever they appear: build output, dependencies, version
    /// control. A language server writes into `target/` as it indexes, so
    /// watching it feeds the watcher its own side effects — 154 events from one
    /// save, measured.
    const ANYWHERE: &[&str] = &["target", "node_modules", "__pycache__"];
    /// Scratch space and build output, and only at the top of the indexed tree.
    ///
    /// `tmp` is a real incident rather than a precaution: a checkout left under
    /// `tmp/` was indexed as part of this repository and the graph went from
    /// 780 to 10,003 nodes — every measurement taken against it was meaningless
    /// until it was noticed. `.gitignore` excluded it and the indexer did not.
    ///
    /// `vendor`, `build` and `out` were measured too and left out — zero
    /// candidate files across five trees, so a rule for them would be a guess
    /// rather than a finding. `dist` is real but is not a root rule; see
    /// `is_build_output` below.
    /// `vendor` holds grammar sources this repository builds but did not
    /// write — 30 KB of `scanner.c` per grammar, foreign code that competes
    /// with our own answers in the search index. Measured: indexing it took
    /// the tree from 1,293 to 1,363 nodes and cost 12 points of English
    /// documentation recall.
    /// `parsers` holds the language scanners. They are ours, but they are a
    /// separate concern from the graph engine: 10,000 lines of token loops
    /// whose vocabulary — `parse`, `scope`, `depth`, `ident` — collides with
    /// what a question about the engine is phrased in. Measured, indexing them
    /// took this tree from 1,349 to 1,596 nodes and cost 4 points of partition
    /// purity.
    const AT_ROOT: &[&str] = &["tmp", "vendor", "parsers"];
    /// Fixture trees that exist to be measured *as their own tree*, never as
    /// part of the one holding them. `bench/deep` is a second shape — layered,
    /// two languages, deep paths — and folding it in moves the document
    /// frequency of every word, so the four floors would score the fixture as
    /// much as the code: measured, this tree went from 1,200 to 1,305 nodes and
    /// `create contact` answered with the fixture's schema classes. Same
    /// reasoning as `tmp`, one level down, which is why it is a path and not a
    /// name — a directory called `deep` anywhere else is not this.
    ///
    /// `bench/langs` is the same idea for extraction rather than retrieval: 25
    /// files writing the same four functions in 25 languages, so that
    /// `langcheck --check` has something to measure that CI actually holds.
    /// Twenty-five `charge`, `refuse` and `commit` definitions would otherwise
    /// compete with this tree's own vocabulary on every question.
    const AT_ROOT_PATHS: &[&str] = &["bench/deep", "bench/langs"];

    if path.components().any(|c| {
        let name = c.as_os_str().to_string_lossy();
        ANYWHERE.contains(&name.as_ref()) || (name.starts_with('.') && name.len() > 1)
    }) {
        return true;
    }
    let Some(root) = root else {
        return false;
    };
    if path
        .strip_prefix(root)
        .ok()
        .and_then(|rel| rel.components().next())
        .is_some_and(|c| AT_ROOT.contains(&c.as_os_str().to_string_lossy().as_ref()))
    {
        return true;
    }
    if let Ok(rel) = path.strip_prefix(root) {
        let rel = rel.to_string_lossy().replace('\\', "/");
        if AT_ROOT_PATHS
            .iter()
            .any(|p| rel == *p || rel.starts_with(&format!("{p}/")))
        {
            return true;
        }
    }
    is_build_output(path, root)
}

/// Whether some ancestor of `path` under `root` is a `dist` beside a
/// `package.json` — the compiled copy of a JavaScript or TypeScript `src`.
///
/// This is the same failure as `tmp`, in the shape a TypeScript tree ships it,
/// and it is worth a rule because the copy is *in the languages we index*.
/// Measured on a NestJS repository: `dist` contributed **318 of 1,007 nodes
/// (32%), every one a duplicate**. `contact.service.ts#ContactService` appeared
/// three times — as `.ts`, `.d.ts` and `.js` — at BM25 scores within 0.08 of
/// each other, next to emitted noise like `contact_service_1`. That spends the
/// scarce resource, which is the 24 seed slots, on copies of the answer.
///
/// **The test is the sibling `package.json`, not the name and not the depth.**
/// A root-relative rule was written first and measured nothing: in a real
/// monorepo `dist` sits at `apps/api/dist`, never at the top. A name-anywhere
/// rule would instead swallow a source module someone named `dist`. The
/// manifest is what actually marks a package, so it is what marks its output.
fn is_build_output(path: &Path, root: &Path) -> bool {
    let Ok(rel) = path.strip_prefix(root) else {
        return false;
    };
    let mut dir = root.to_path_buf();
    for c in rel.components() {
        if c.as_os_str() == "dist" && dir.join("package.json").exists() {
            return true;
        }
        dir.push(c);
    }
    false
}

/// Follows a tree and reports what changed, one batch per burst of file
/// saves rather than one per save.
///
/// Watches `root` and calls `on_batch` with the deduplicated set of changed
/// paths. Blocks until the process ends, like the old native watcher did.
pub fn watch(root: &Path, mut on_batch: impl FnMut(Vec<PathBuf>)) -> std::io::Result<()> {
    let mut previous = file_stamps(root)?;
    loop {
        std::thread::sleep(DEBOUNCE);
        let current = file_stamps(root)?;
        let mut changed = HashSet::new();
        for (path, stamp) in &current {
            if previous.get(path) != Some(stamp) {
                changed.insert(path.clone());
            }
        }
        for path in previous.keys() {
            if !current.contains_key(path) {
                changed.insert(path.clone());
            }
        }
        if !changed.is_empty() {
            let mut batch: Vec<_> = changed.into_iter().collect();
            batch.sort();
            on_batch(batch);
        }
        previous = current;
    }
}

fn file_stamps(root: &Path) -> std::io::Result<std::collections::HashMap<PathBuf, (u64, u64)>> {
    let mut out = std::collections::HashMap::new();
    let mut dirs = vec![root.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if ignored_under(&path, Some(root)) {
                continue;
            }
            let ty = entry.file_type()?;
            if ty.is_dir() {
                dirs.push(path);
            } else if ty.is_file() {
                let metadata = entry.metadata()?;
                let modified = metadata
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |t| t.as_nanos() as u64);
                out.insert(path, (modified, metadata.len()));
            }
        }
    }
    Ok(out)
}
