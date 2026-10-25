//! GitHub workflows and composite actions: the jobs a workflow runs, and the
//! files its steps run.
//!
//! Read as plain YAML a workflow is one node per key — `name`, `on`, `steps`,
//! `with` — and no edge, which is why `.github` stayed out of the graph. What a
//! workflow does express is a dependency: `needs: build` orders jobs,
//! `uses: ./.github/actions/setup` runs a local action, and `run: scripts/ci.sh`
//! runs a file of the tree. Those are the edges a change to a script or an
//! action is judged by, so a workflow gets its own reading: jobs are the
//! definitions, `needs` the calls between them, and every path a step names
//! is a reference to that file, checked against the tree by the indexer.
//!
//! Line based, like the YAML scanner: a workflow is written by hand in the
//! block style, and a flow-style `jobs: {a: …}` is not a shape anyone writes.

use std::path::Path;

use crate::parse_ast::{Lang, LangExt};

/// Whether `path` is a workflow or an action definition: `.github/workflows/*.yml`
/// and `action.yml` anywhere, since a local action lives in its own directory.
pub fn is_workflow(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    if matches!(name, "action.yml" | "action.yaml") {
        return true;
    }
    (name.ends_with(".yml") || name.ends_with(".yaml"))
        && path
            .parent()
            .and_then(|d| d.file_name())
            .is_some_and(|d| d == "workflows")
        && path
            .parent()
            .and_then(Path::parent)
            .and_then(|d| d.file_name())
            .is_some_and(|d| d == ".github")
}

/// What a workflow file defines and runs.
#[derive(Debug, Default, PartialEq)]
pub struct Facts {
    /// Job ids, in order.
    pub defines: Vec<String>,
    /// (job, byte range of the id).
    pub ranges: Vec<(String, (u32, u32))>,
    /// (caller, callee): the file's top level runs every job, a job `needs`
    /// another, and a step runs a command.
    pub calls: Vec<(String, String)>,
    /// (caller, path): a file of the tree a step runs or uses, as written.
    pub file_refs: Vec<(String, String)>,
}

const TOP: &str = "<module>";

/// Words at the start of a shell command that are not the command.
const SHELL_WORDS: &[&str] = &[
    "if", "then", "else", "elif", "fi", "for", "while", "until", "do", "done", "case", "esac",
    "in", "return", "exit", "local", "export", "echo", "cd", "set", "source", "true", "false",
    "shift", "read", "printf", "test", "sudo", "time", "exec", "env", "mkdir", "rm", "cp", "mv",
    "cat", "ls", "touch", "chmod",
];

pub fn extract(src: &str) -> Facts {
    let mut facts = Facts::default();
    let mut offset = 0usize;
    // Indent of the `jobs:` key, and of its job ids once the first is seen.
    let mut jobs_indent: Option<usize> = None;
    let mut job_indent: Option<usize> = None;
    let mut job: Option<String> = None;
    // A `run: |` block or a `needs:` list continues while lines stay deeper
    // than the key that opened it.
    let mut block: Option<(usize, Block)> = None;
    // The previous command line ended in `\\`: this one continues it.
    let mut continued = false;

    for line in src.lines() {
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        let content = trimmed.trim_end();
        if let Some((key_indent, kind)) = &block {
            if content.is_empty() || indent > *key_indent {
                let caller = job.clone().unwrap_or_else(|| TOP.to_string());
                match kind {
                    Block::Run => {
                        command(&caller, content, continued, &mut facts);
                        continued = content.ends_with('\\');
                    }
                    Block::Needs => {
                        if let Some(item) = content.strip_prefix("- ") {
                            facts.calls.push((caller, unquote(item).to_string()));
                        }
                    }
                }
                offset += line.len() + 1;
                continue;
            }
            block = None;
        }
        if content.is_empty() || content.starts_with('#') {
            offset += line.len() + 1;
            continue;
        }
        let item = content.strip_prefix("- ").map(str::trim_start);
        let key_line = item.unwrap_or(content);
        let Some((key, value)) = split_key(key_line) else {
            offset += line.len() + 1;
            continue;
        };
        let value_indent = indent + (content.len() - key_line.len());

        if indent == 0 {
            job = None;
            jobs_indent = (key == "jobs").then_some(0);
            job_indent = None;
        } else if let Some(ji) = jobs_indent
            && indent > ji
            && item.is_none()
            && job_indent.is_none_or(|j| j == indent)
        {
            job_indent = Some(indent);
            job = Some(key.to_string());
            let start = offset + value_indent;
            facts.defines.push(key.to_string());
            facts
                .ranges
                .push((key.to_string(), (start as u32, (start + key.len()) as u32)));
            facts.calls.push((TOP.to_string(), key.to_string()));
            offset += line.len() + 1;
            continue;
        }

        let caller = job.clone().unwrap_or_else(|| TOP.to_string());
        match key {
            "needs" => {
                if value.is_empty() {
                    block = Some((value_indent, Block::Needs));
                } else {
                    for name in value.trim_matches(['[', ']']).split(',') {
                        let name = unquote(name.trim());
                        if !name.is_empty() {
                            facts.calls.push((caller.clone(), name.to_string()));
                        }
                    }
                }
            }
            "uses" | "main" | "pre" | "post" => {
                let value = unquote(value);
                // A local action or workflow is a path; a marketplace action
                // `owner/repo@v4` is not in the tree.
                if value.starts_with("./") || key != "uses" {
                    facts.file_refs.push((caller, value.to_string()));
                }
            }
            "run" => {
                if value.is_empty()
                    || value == "|"
                    || value == ">"
                    || value.starts_with("|-")
                    || value.starts_with(">-")
                {
                    block = Some((value_indent, Block::Run));
                } else {
                    command(&caller, value, false, &mut facts);
                }
            }
            _ => {}
        }
        offset += line.len() + 1;
    }
    facts
}

enum Block {
    Run,
    Needs,
}

/// `key: value` with a plain-word key; `None` for list items and prose.
fn split_key(line: &str) -> Option<(&str, &str)> {
    let (key, value) = line.split_once(':')?;
    let key = key.trim();
    if key.is_empty()
        || !key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
    {
        return None;
    }
    Some((key, value.trim()))
}

fn unquote(s: &str) -> &str {
    s.trim().trim_matches(['"', '\''])
}

/// One line of a shell command: the first word of each pipeline stage is a
/// call, and every word that names a file of the tree is a reference to it. A
/// line continuing the previous one holds arguments, not a command.
fn command(caller: &str, line: &str, continued: bool, facts: &mut Facts) {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return;
    }
    // `aarch64|arm64) …`: a case pattern, not a pipeline.
    let pattern = line.contains(')') && !line.contains('(');
    // `${{ matrix.os }}` is one value; `$(cmd …)` runs `cmd`.
    let line = strip_expressions(&line.replace("${{ github.workspace }}", "."))
        .replace("$(", " ")
        .replace(['`', ')'], " ");
    for (n, stage) in line
        .split(['|', ';'])
        .flat_map(|s| s.split("&&"))
        .enumerate()
    {
        let mut words = stage.split_whitespace().peekable();
        // `CI=1 cmd`: assignments before the command.
        while words.peek().is_some_and(|w| is_assignment(w)) {
            words.next();
        }
        let mut first = n > 0 || !continued;
        for word in words {
            if let Some(path) = file_path(word) {
                facts.file_refs.push((caller.to_string(), path.to_string()));
            } else if first && !pattern && is_command(word) && !SHELL_WORDS.contains(&word) {
                facts.calls.push((caller.to_string(), word.to_string()));
            }
            first = false;
        }
    }
}

/// The line without its `${{ … }}` expressions.
fn strip_expressions(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(start) = rest.find("${{") {
        out.push_str(&rest[..start]);
        out.push('X');
        rest = match rest[start..].find("}}") {
            Some(end) => &rest[start + end + 2..],
            None => "",
        };
    }
    out.push_str(rest);
    out
}

fn is_assignment(word: &str) -> bool {
    word.split_once('=')
        .is_some_and(|(name, _)| !name.is_empty() && is_word(name))
}

/// A command is a lower-case word: `cargo`, `python3`, `gh`. Flags, variables
/// and `${{ … }}` expressions are not.
fn is_command(word: &str) -> bool {
    is_word(word) && word.starts_with(|c: char| c.is_ascii_lowercase())
}

fn is_word(word: &str) -> bool {
    word.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
        && word.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
}

/// A word that names a file of the tree: a relative path with a directory in
/// it, or a script by extension. The workspace variable is the tree root.
fn file_path(word: &str) -> Option<&str> {
    let word = word.trim_matches(['"', '\'']);
    let word = word.strip_prefix("$GITHUB_WORKSPACE/").unwrap_or(word);
    let path = word.strip_prefix("./").unwrap_or(word);
    if path.is_empty()
        || path.starts_with(['-', '$', '/', '~'])
        || path.contains(['$', '*', '{', '}', '(', ')', '=', ':', '`', '\\'])
        || path.starts_with("http")
        || path.split('/').any(|c| c == "..")
    {
        return None;
    }
    let script = path.rsplit_once('.').is_some_and(|(_, ext)| {
        matches!(
            ext,
            "sh" | "bash" | "py" | "js" | "mjs" | "cjs" | "ts" | "rb" | "pl" | "ps1"
        )
    });
    (path.contains('/') || script).then_some(path)
}

/// The file a reference names, relative to `root` with `/`, when it exists
/// and is indexed: the path itself, or the `action.yml` of a directory.
pub fn target(root: &Path, written: &str) -> Option<String> {
    let rel = written
        .strip_prefix("./")
        .unwrap_or(written)
        .trim_end_matches('/');
    if rel.is_empty() || rel.split('/').any(|c| c == "..") {
        return None;
    }
    let full = root.join(rel);
    if full.is_file() {
        // A manifest or a lock file named on a command line is not code
        // the graph holds, so a reference to it would never link.
        return Lang::from_path(&full).map(|_| rel.to_string());
    }
    if full.is_dir() {
        for name in ["action.yml", "action.yaml"] {
            if full.join(name).is_file() {
                return Some(format!("{rel}/{name}"));
            }
        }
    }
    None
}
