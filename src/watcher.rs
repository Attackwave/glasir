//! File system watcher.
//!
//! Editors emit a burst of writes while someone types. A portable polling
//! watcher compares a compact file stamp every `DEBOUNCE` and reports one
//! deduplicated batch. It has the same semantics on Linux, macOS and Windows,
//! and avoids a platform-specific watcher dependency tree.

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
    /// Anything a tree wants out for its own reasons belongs in its
    /// `.glasirignore`, not here: a name here is taken from every project.
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
    /// write — 30 KB of `scanner.c` per grammar, code that competes
    /// with our own answers in the search index. Measured: indexing it took
    /// the tree from 1,293 to 1,363 nodes and cost 12 points of English
    /// documentation recall.
    const AT_ROOT: &[&str] = &["tmp", "vendor"];

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
    is_build_output(path, root)
}

/// Whether some ancestor of `path` under `root` is a `dist` beside a
/// `package.json` — the compiled copy of a JavaScript or TypeScript `src`.
///
/// This is the same failure as `tmp`, in the shape a TypeScript tree ships it,
/// and it is worth a rule because the copy is *in the languages we index*.
/// Measured: `dist` contributed **318 of 1,007 nodes
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

/// The per-directory file that narrows or widens what `.gitignore` admits,
/// in the same syntax. It takes precedence, so `!plan.md` indexes a file the
/// repository does not track.
pub const IGNORE_FILE: &str = ".glasirignore";

/// Every file under `root` the indexer may read, sorted.
///
/// The fixed rules of `ignored_under` first, then every `.gitignore` and
/// `.glasirignore` in the tree, the latter winning. Nothing outside the tree
/// takes part — no global excludes file, no `.git/info/exclude`, no ignore
/// file above `root` — so the same commit yields the same graph on every
/// machine. Symbolic links are skipped, as a linked directory would index the
/// same sources twice. A broken pattern is reported through `on_error` and
/// the rest of its file still applies.
pub fn files(root: &Path, mut on_error: impl FnMut(String)) -> std::io::Result<Vec<PathBuf>> {
    // An unreadable root is an error, never an empty tree: an empty answer
    // would read as every file deleted.
    std::fs::read_dir(root)?;
    let base = root.to_path_buf();
    let walk = ignore::WalkBuilder::new(root)
        .standard_filters(false)
        .git_ignore(true)
        .require_git(false)
        .add_custom_ignore_filename(IGNORE_FILE)
        .follow_links(false)
        .filter_entry(move |e| e.depth() == 0 || !ignored_under(e.path(), Some(&base)))
        .build();
    let mut out = Vec::new();
    for entry in walk {
        match entry {
            Ok(e) if e.file_type().is_some_and(|t| t.is_file()) => out.push(e.into_path()),
            Ok(_) => {}
            Err(e) => on_error(e.to_string()),
        }
    }
    // `read_dir` makes no ordering promise, and ids are assigned in this order.
    out.sort();
    Ok(out)
}

/// Whether a `.glasirignore` names `path` with `!`: the explicit opt-in a
/// file left out by default (`docs::is_instructions`) needs. The deepest
/// `.glasirignore` that matches decides, as in git.
pub fn opted_in(root: &Path, path: &Path) -> bool {
    let Ok(rel) = path.strip_prefix(root) else {
        return false;
    };
    let mut dirs = vec![root.to_path_buf()];
    for c in rel.parent().into_iter().flat_map(Path::components) {
        let next = dirs[dirs.len() - 1].join(c);
        dirs.push(next);
    }
    let mut verdict = false;
    for dir in dirs {
        let file = dir.join(IGNORE_FILE);
        if !file.is_file() {
            continue;
        }
        let mut builder = ignore::gitignore::GitignoreBuilder::new(&dir);
        builder.add(&file);
        let Ok(rules) = builder.build() else { continue };
        match rules.matched(path, false) {
            ignore::Match::Whitelist(_) => verdict = true,
            ignore::Match::Ignore(_) => verdict = false,
            ignore::Match::None => {}
        }
    }
    verdict
}

/// Follows a tree and reports what changed, one batch per burst of file
/// saves rather than one per save.
///
/// Calls `on_batch` with each changed path and whether it is still in scope.
/// A file that leaves the scope — deleted, or newly ignored — is reported as
/// gone even though it may still exist, so a caller must not read it. Blocks
/// until the process ends.
pub fn watch(root: &Path, mut on_batch: impl FnMut(Vec<(PathBuf, bool)>)) -> std::io::Result<()> {
    let mut previous = file_stamps(root)?;
    loop {
        std::thread::sleep(DEBOUNCE);
        // A tree that cannot be read right now is skipped, not reported empty.
        let Ok(current) = file_stamps(root) else {
            continue;
        };
        let changed = changes(&previous, &current);
        if !changed.is_empty() {
            on_batch(changed);
        }
        previous = current;
    }
}

type Stamps = std::collections::HashMap<PathBuf, (u64, u64)>;

/// What moved between two polls: a changed or new path with `true`, one that
/// left the scope with `false`. Sorted.
pub(crate) fn changes(previous: &Stamps, current: &Stamps) -> Vec<(PathBuf, bool)> {
    let mut changed: Vec<(PathBuf, bool)> = current
        .iter()
        .filter(|(path, stamp)| previous.get(*path) != Some(*stamp))
        .map(|(path, _)| (path.clone(), true))
        .chain(
            previous
                .keys()
                .filter(|path| !current.contains_key(*path))
                .map(|path| (path.clone(), false)),
        )
        .collect();
    changed.sort();
    changed
}

pub(crate) fn file_stamps(root: &Path) -> std::io::Result<Stamps> {
    let mut out = Stamps::new();
    for path in files(root, |_| {})? {
        // Vanished between the walk and this call: left out, so the next
        // poll reports it.
        let Ok(metadata) = std::fs::metadata(&path) else {
            continue;
        };
        let modified = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |t| t.as_nanos() as u64);
        out.insert(path, (modified, metadata.len()));
    }
    Ok(out)
}
