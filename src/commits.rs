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
use std::process::{Command, Stdio};

use crate::parse_ast::{Lang, LangExt};

pub struct Change {
    pub sha: String,
    pub date: String,
    pub subject: String,
    pub body: String,
}

/// The commits among the last `window` touching `file` (relative to `root`)
/// that changed the definition `name`, newest first.
pub fn changes(root: &Path, file: &str, name: &str, window: usize) -> Result<Vec<Change>, String> {
    let lang = Lang::from_path(Path::new(file))
        .ok_or_else(|| format!("{file} is not a language this history reads"))?;
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "log",
            "--no-merges",
            "--no-renames",
            "--no-color",
            "--relative",
            "-U0",
            "-p",
            "--format=%x1e%H%x1f%cs%x1f%s%x1f%b%x1f",
        ])
        .arg(format!("-n{window}"))
        .arg("--")
        .arg(file)
        .output()
        .map_err(|e| format!("git could not run: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git could not read the history: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let log = String::from_utf8_lossy(&out.stdout);

    let mut candidates = Vec::new();
    for record in log.split('\u{1e}').filter(|r| !r.trim().is_empty()) {
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
        let hunks = crate::mcp::hunk_lines(patch);
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
        if crate::mcp::innermost_touched(&facts.ranges, &src, &hunks).contains(name) {
            out.push(change);
        }
    }
    Ok(out)
}

/// File contents at given revisions, `rev:path` each, through one
/// `git cat-file --batch`. `None` for an object that does not exist or is not
/// text.
fn cat_files(root: &Path, specs: impl Iterator<Item = String>) -> Vec<Option<String>> {
    let specs: Vec<String> = specs.collect();
    if specs.is_empty() {
        return Vec::new();
    }
    let Ok(mut child) = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["cat-file", "--batch"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return vec![None; specs.len()];
    };
    let mut stdin = child.stdin.take().expect("piped");
    let input = specs.join("\n") + "\n";
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
