//! The git repositories a served tree holds.
//!
//! A tree is usually one repository. It may also be a directory holding
//! several — a client and the server it talks to — served as one graph, so
//! that a request in one reaches its handler in the other. The graph needs
//! nothing for that; git does: a diff, a history or a hook belongs to a
//! repository, and the directory above them is none.

use std::path::{Path, PathBuf};

/// One repository under a served tree.
#[derive(Debug, Clone, PartialEq)]
pub struct Repo {
    /// Its path below the tree with a trailing `/`, or empty when the tree is
    /// the repository. What a path git prints is prefixed with.
    pub prefix: String,
    pub root: PathBuf,
}

/// Whether `dir` is the top of a repository or a linked worktree.
fn is_repo(dir: &Path) -> bool {
    let git = dir.join(".git");
    git.is_dir() || git.is_file()
}

/// The repositories under `root`: the tree itself when it is one, else every
/// repository up to two directories down, in path order. Empty when there is
/// none, which the callers report rather than answer "nothing changed".
pub fn repos(root: &Path) -> Vec<Repo> {
    if is_repo(root) {
        return vec![Repo {
            prefix: String::new(),
            root: root.to_path_buf(),
        }];
    }
    let mut out = Vec::new();
    let mut dirs: Vec<(PathBuf, usize)> = vec![(root.to_path_buf(), 0)];
    while let Some((dir, depth)) = dirs.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !entry.file_type().is_ok_and(|t| t.is_dir())
                || crate::watcher::ignored_under(&path, Some(root))
            {
                continue;
            }
            if is_repo(&path) {
                let rel = path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push(Repo {
                    prefix: format!("{rel}/"),
                    root: path,
                });
            } else if depth < 1 {
                dirs.push((path, depth + 1));
            }
        }
    }
    out.sort_by(|a, b| a.prefix.cmp(&b.prefix));
    out
}

/// The repository a tree-relative path belongs to, and the path inside it.
pub fn owner<'r>(repos: &'r [Repo], file: &str) -> Option<(&'r Repo, String)> {
    repos
        .iter()
        .filter(|r| file.starts_with(&r.prefix))
        .max_by_key(|r| r.prefix.len())
        .map(|r| (r, file[r.prefix.len()..].to_string()))
}
