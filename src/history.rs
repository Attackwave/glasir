//! Change coupling: files that change in the same commits.
//!
//! The graph sees a dependency only where the code names one. A template and
//! the handler that fills it, a migration and the model it alters, a test in
//! another language — these change together without either naming the other,
//! and only the history records it. Counted from `git log`, so the answer is
//! as deterministic as the commits it reads.

/// Commits touching more files than this are left out: a formatting run, a
/// rename or a vendored update couples everything with everything and drowns
/// the pairs that mean something. The usual bound in change-coupling analysis.
pub const MAX_COMMIT_FILES: usize = 30;

/// A pair seen once is coincidence; the floor for reporting it.
pub const MIN_TOGETHER: usize = 2;

/// The files of each of the last `limit` non-merge commits, relative to `root`
/// and restricted to it.
pub fn commits(root: &std::path::Path, limit: usize) -> Result<Vec<Vec<String>>, String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "log",
            "--no-merges",
            "--relative",
            "--name-only",
            "--format=%x1e",
        ])
        .arg(format!("-n{limit}"))
        .output()
        .map_err(|e| format!("git could not run: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git could not read the history: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(parse(&String::from_utf8_lossy(&out.stdout)))
}

/// Splits `git log --name-only --format=%x1e` output into one file list per
/// commit, dropping empty commits and the oversized ones.
pub fn parse(log: &str) -> Vec<Vec<String>> {
    log.split('\u{1e}')
        .map(|chunk| {
            let mut files: Vec<String> = chunk
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_string)
                .collect();
            files.sort();
            files.dedup();
            files
        })
        .filter(|f| !f.is_empty() && f.len() <= MAX_COMMIT_FILES)
        .collect()
}

/// How many commits touched `target`, and every other file that changed with
/// it at least `MIN_TOGETHER` times, most frequent first. Ties break on the
/// path so the order is reproducible.
pub fn coupled(commits: &[Vec<String>], target: &str) -> (usize, Vec<(String, usize)>) {
    let mut together: std::collections::HashMap<&str, usize> = Default::default();
    let mut own = 0;
    for files in commits.iter().filter(|f| f.iter().any(|x| x == target)) {
        own += 1;
        for f in files.iter().filter(|f| *f != target) {
            *together.entry(f).or_default() += 1;
        }
    }
    let mut pairs: Vec<(String, usize)> = together
        .into_iter()
        .filter(|&(_, n)| n >= MIN_TOGETHER)
        .map(|(f, n)| (f.to_string(), n))
        .collect();
    pairs.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    (own, pairs)
}
