//! Why a definition changed: the commits that changed it, with their messages.
//!
//! Answered when asked, for one file, rather than indexed. Measured: 300
//! commit messages in the search index took this tree's analysis from 0.08 s
//! to 2.3 s and six of seven recall floors down, some by 20 points, because
//! messages describe the code in the words questions use. As a tool they cost
//! nothing until called and never compete with the code.
//!
//! A commit changed a definition when one of its hunks falls in the
//! definition's range as the file was at that commit, innermost first — the
//! rule `detect_changes` applies to the working tree. Local and deterministic:
//! `git log` and `git cat-file`, no network.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::Stdio;

use crate::parse_ast::{Lang, LangExt};

pub struct Change {
    pub sha: String,
    pub date: String,
    pub subject: String,
    pub body: String,
}

/// The commits among the last `window` touching `file` (relative to `root`)
/// that changed the definition `name`, newest first.
/// Also returns how many commits of the file git yielded, which is at most
/// `window` and less when the file has fewer.
pub fn changes(
    root: &Path,
    file: &str,
    name: &str,
    window: usize,
) -> Result<(Vec<Change>, usize), String> {
    let lang = Lang::from_path(Path::new(file))
        .ok_or_else(|| format!("{file} is not a language this history reads"))?;
    // The repository the file is in, which may sit below the served tree.
    let repos = crate::repos::repos(root)?;
    let (repo, local) = crate::repos::owner(&repos, file)
        .ok_or_else(|| format!("{file} is in no git repository under {}", root.display()))?;
    let (root, file) = (repo.root.as_path(), local.as_str());
    let out = crate::repos::output(
        crate::repos::git(root)
            .args([
                "log",
                "--no-merges",
                "--no-renames",
                "--no-color",
                "--no-ext-diff",
                "--no-textconv",
                "--relative",
                "-U0",
                "-p",
                "--format=%x1e%H%x1f%cs%x1f%s%x1f%b%x1f",
            ])
            .arg(format!("-n{window}"))
            .arg("--")
            .arg(file),
    )
    .map_err(|e| format!("git could not run: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git could not read the history: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let log = String::from_utf8_lossy(&out.stdout);

    let mut candidates = Vec::new();
    let mut read = 0;
    for record in log.split('\u{1e}').filter(|r| !r.trim().is_empty()) {
        read += 1;
        let mut parts = record.splitn(5, '\u{1f}');
        let (Some(sha), Some(date), Some(subject), Some(body), Some(patch)) = (
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
        ) else {
            continue;
        };
        let hunks = hunk_lines(patch);
        if hunks.is_empty() {
            continue;
        }
        candidates.push((
            Change {
                sha: sha.trim().to_string(),
                date: date.trim().to_string(),
                subject: subject.trim().to_string(),
                body: without_trailers(body),
            },
            hunks,
        ));
    }

    let contents = cat_files(
        root,
        candidates
            .iter()
            .map(|(c, _)| format!("{}:./{file}", c.sha)),
    );
    let mut out = Vec::new();
    for ((change, hunks), src) in candidates.into_iter().zip(contents) {
        let Some(src) = src else { continue };
        let Some(facts) = crate::parse_ast::parse_file(Path::new(file), &src, lang) else {
            continue;
        };
        if innermost_touched(&facts.ranges, &src, &hunks).contains(name) {
            out.push(change);
        }
    }
    Ok((out, read))
}

/// Largest file read back from history: a source file, not a generated blob.
const MAX_BLOB: usize = 8 << 20;

/// File contents at given revisions, `rev:path` each, through one
/// `git cat-file --batch`. `None` for an object that does not exist or is not
/// text.
fn cat_files(root: &Path, specs: impl Iterator<Item = String>) -> Vec<Option<String>> {
    let specs: Vec<String> = specs.collect();
    if specs.is_empty() {
        return Vec::new();
    }
    let Ok(mut child) = crate::repos::git(root)
        .args(["cat-file", "--batch"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return vec![None; specs.len()];
    };
    let mut stdin = child.stdin.take().expect("piped");
    // A path holding a newline would split into two requests and shift every
    // later answer; it is asked as an object that cannot exist instead.
    let input: String = specs
        .iter()
        .map(|s| if s.contains('\n') { "0".repeat(40) } else { s.clone() } + "\n")
        .collect();
    // Written from a thread: git answers while it reads, and a large request
    // would otherwise fill both pipes and stop both processes.
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(input.as_bytes());
    });
    let mut out = BufReader::new(child.stdout.take().expect("piped"));
    let mut result = Vec::with_capacity(specs.len());
    for _ in &specs {
        let mut header = String::new();
        if out.read_line(&mut header).unwrap_or(0) == 0 {
            break;
        }
        let mut fields = header.split_whitespace();
        let (Some(_), Some(kind), Some(size)) = (fields.next(), fields.next(), fields.next())
        else {
            result.push(None);
            continue;
        };
        let Ok(size) = size.parse::<usize>() else {
            result.push(None);
            continue;
        };
        if size > MAX_BLOB {
            let skipped =
                std::io::copy(&mut (&mut out).take(size as u64 + 1), &mut std::io::sink());
            if skipped.is_err() {
                break;
            }
            result.push(None);
            continue;
        }
        let mut body = vec![0u8; size + 1];
        if out.read_exact(&mut body).is_err() {
            break;
        }
        body.pop();
        result.push(
            (kind == "blob")
                .then(|| String::from_utf8(body).ok())
                .flatten(),
        );
    }
    // Stopped early, git may still be writing: it would never exit.
    let _ = child.kill();
    let _ = writer.join();
    let _ = child.wait();
    result.resize(specs.len(), None);
    result
}

/// The body without its trailing `Key: value` lines — `Signed-off-by`,
/// `Co-authored-by`, a session link — which every commit repeats.
fn without_trailers(body: &str) -> String {
    let mut lines: Vec<&str> = body.trim().lines().collect();
    while let Some(last) = lines.last() {
        let trailer = last.split_once(": ").is_some_and(|(key, _)| {
            !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        });
        if trailer || last.trim().is_empty() {
            lines.pop();
        } else {
            break;
        }
    }
    lines.join("\n")
}

/// New-side line ranges of `git diff -U0`, 1-based and inclusive. A pure
/// deletion has no new lines and is a point between two: it comes back as
/// `(line after, line before)`, start past end, and touches only a definition
/// that contains the point — not the one that happens to begin right there.
pub(crate) fn hunk_lines(diff: &str) -> Vec<(u32, u32)> {
    diff.lines()
        .filter_map(|l| l.strip_prefix("@@ "))
        .filter_map(|l| l.split_whitespace().find(|w| w.starts_with('+')))
        .filter_map(|w| {
            let (start, count) = w[1..].split_once(',').unwrap_or((&w[1..], "1"));
            let (start, count): (u32, u32) = (start.parse().ok()?, count.parse().ok()?);
            Some(if count == 0 {
                (start + 1, start)
            } else {
                (start, start + count - 1)
            })
        })
        .collect()
}

/// The innermost definitions a diff's hunks fall in, by their ranges in the
/// new content, and `<module>` for a hunk outside every definition. Shared by
/// `detect_changes` and the commit history, so both read a change alike.
pub(crate) fn innermost_touched(
    ranges: &[(String, (u32, u32))],
    src: &str,
    hunks: &[(u32, u32)],
) -> std::collections::HashSet<String> {
    let mut line_start = vec![0u32];
    line_start.extend(
        src.bytes()
            .enumerate()
            .filter(|&(_, b)| b == b'\n')
            .map(|(i, _)| i as u32 + 1),
    );
    let end = src.len() as u32;
    let bytes = |(a, b): (u32, u32)| {
        let at = |line: u32| line_start.get(line as usize - 1).copied().unwrap_or(end);
        (at(a.max(1)), at(b + 1))
    };
    let mut names = std::collections::HashSet::new();
    for &(a, b) in hunks {
        let (hs, he) = if a > b {
            let at = bytes((a, a)).0;
            (at, at)
        } else {
            bytes((a, b))
        };
        let hit: Vec<&(String, (u32, u32))> = ranges
            .iter()
            .filter(|(_, (s, e))| {
                if hs == he {
                    *s < hs && hs < *e
                } else {
                    *s < he && hs < *e
                }
            })
            .collect();
        if hit.is_empty() {
            names.insert("<module>".to_string());
        }
        // Only the innermost definition a hunk falls in changed. A class
        // around an edited method is not itself changed — its users are not
        // the method's callers — and a function around an edited inner one is
        // reached anyway, as the inner one's caller.
        for (name, (s, e)) in &hit {
            let holds_another = hit
                .iter()
                .any(|(_, (s2, e2))| (s, e) != (s2, e2) && s <= s2 && e2 <= e);
            if !holds_another {
                names.insert(name.clone());
            }
        }
    }
    names
}
