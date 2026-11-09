//! The git repositories a served tree holds.
//!
//! A tree is usually one repository. It may also be a directory holding
//! several — a client and the server it talks to — served as one graph, so
//! that a request in one reaches its handler in the other. The graph needs
//! nothing for that; git does: a diff, a history or a hook belongs to a
//! repository, and the directory above them is none.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

/// Longest a git command may run. A diff or a bounded log answers in
/// seconds; a longer one is a hung helper, not a slow repository.
const TIMEOUT: Duration = Duration::from_secs(120);

/// Most output a git command may produce. Histories are read in windows and
/// diffs per file, so more than this is a loop rather than an answer.
const MAX_OUTPUT: usize = 64 << 20;

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

/// A `git` command in `repo`. Optional locks are off: a served tree may be
/// read-only, or shared with the hook that refreshes it.
pub fn git(repo: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(repo).env("GIT_OPTIONAL_LOCKS", "0");
    cmd
}

/// Runs `cmd` to completion and collects its output, killing it at `TIMEOUT`
/// and refusing more than `MAX_OUTPUT` bytes on either stream. The child
/// is drained throughout, so it never blocks on a full pipe.
pub fn output(cmd: &mut Command) -> std::io::Result<Output> {
    output_within(cmd, TIMEOUT)
}

fn output_within(cmd: &mut Command, limit: Duration) -> std::io::Result<Output> {
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    // Each pipe is read on a thread that reports through a channel, so the
    // wait for it has the same deadline: a helper git started can keep a pipe
    // open after git itself has exited.
    let drain = |pipe: Option<Box<dyn Read + Send>>| {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut kept = Vec::new();
            let mut over = false;
            let mut buf = [0u8; 64 << 10];
            if let Some(mut pipe) = pipe {
                while let Ok(n) = pipe.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    if kept.len() + n <= MAX_OUTPUT {
                        kept.extend_from_slice(&buf[..n]);
                    } else {
                        over = true;
                    }
                }
            }
            let _ = tx.send((kept, over));
        });
        rx
    };
    let stdout = drain(child.stdout.take().map(|p| Box::new(p) as _));
    let stderr = drain(child.stderr.take().map(|p| Box::new(p) as _));
    let deadline = Instant::now() + limit;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                format!("git did not finish within {}s", limit.as_secs()),
            ));
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let collect = |rx: std::sync::mpsc::Receiver<(Vec<u8>, bool)>| {
        rx.recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "git exited but its output stayed open",
                )
            })
    };
    let (stdout, over_out) = collect(stdout)?;
    let (stderr, over_err) = collect(stderr)?;
    if over_out || over_err {
        return Err(std::io::Error::other(format!(
            "git produced more than {} MiB",
            MAX_OUTPUT >> 20
        )));
    }
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

/// A configuration key whose value git runs as a command on the operations
/// used here: diffs, logs and the index. A served tree is not trusted to run
/// anything, so a repository that sets one is refused rather than read.
fn runs_commands(key: &str, value: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    let (section, name) = match (lower.split_once('.'), lower.rsplit_once('.')) {
        (Some((section, _)), Some((_, name))) => (section, name),
        _ => return false,
    };
    match section {
        // `true` and friends run git's own daemon; anything else is a hook.
        "core" => {
            name == "fsmonitor"
                && !matches!(
                    value.to_ascii_lowercase().as_str(),
                    "true" | "false" | "yes" | "no" | "on" | "off" | "1" | "0" | ""
                )
        }
        "diff" => matches!(name, "external" | "command" | "textconv"),
        "filter" => matches!(name, "clean" | "smudge" | "process"),
        _ => false,
    }
}

/// Refuses a repository whose own configuration names a command.
fn check(repo: &Path) -> Result<(), String> {
    let out = output(git(repo).args(["config", "--local", "--list", "-z"]))
        .map_err(|e| format!("git could not run in {}: {e}", repo.display()))?;
    if !out.status.success() {
        return Err(format!(
            "git could not read the configuration of {}: {}",
            repo.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    for entry in out.stdout.split(|b| *b == 0) {
        let entry = String::from_utf8_lossy(entry);
        let (key, value) = entry.split_once('\n').unwrap_or((&entry, ""));
        if runs_commands(key, value) {
            return Err(format!(
                "{} sets `{key}` in its own configuration, which git would run as a command; \
                 this tree is not trusted to run anything",
                repo.display()
            ));
        }
    }
    Ok(())
}

/// The repositories under `root`: the tree itself when it is one, else every
/// repository up to two directories down, in path order. Empty when there is
/// none, which the callers report rather than answer "nothing changed". An
/// error names a repository that would run commands (`check`).
pub fn repos(root: &Path) -> Result<Vec<Repo>, String> {
    let found = find(root);
    for repo in &found {
        check(&repo.root)?;
    }
    Ok(found)
}

fn find(root: &Path) -> Vec<Repo> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_keys_are_refused_and_plain_ones_pass() {
        assert!(runs_commands("core.fsmonitor", "/tmp/hook"));
        assert!(!runs_commands("core.fsmonitor", "true"));
        assert!(runs_commands("diff.external", "x"));
        assert!(runs_commands("diff.pdf.textconv", "x"));
        assert!(runs_commands("diff.Mixed.command", "x"));
        assert!(runs_commands("filter.lfs.clean", "x"));
        assert!(!runs_commands("filter.lfs.required", "true"));
        assert!(!runs_commands("core.hooksPath", "x"));
        assert!(!runs_commands("remote.origin.url", "x"));
        assert!(!runs_commands("nodot", "x"));
    }

    #[cfg(unix)]
    #[test]
    fn a_hung_command_is_killed_at_the_deadline() {
        let err =
            output_within(Command::new("sleep").arg("30"), Duration::from_millis(100)).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::TimedOut);
    }

    /// A helper left running in the background keeps the pipe open after
    /// the command exits; the deadline holds for that wait too.
    #[cfg(unix)]
    #[test]
    fn an_inherited_pipe_does_not_outlast_the_deadline() {
        let started = Instant::now();
        let err = output_within(
            Command::new("sh").args(["-c", "sleep 3 & echo x"]),
            Duration::from_millis(300),
        )
        .unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[cfg(unix)]
    #[test]
    fn output_is_collected() {
        let out = output(Command::new("sh").args(["-c", "echo out; echo err >&2"])).unwrap();
        assert!(out.status.success());
        assert_eq!(out.stdout, b"out\n");
        assert_eq!(out.stderr, b"err\n");
    }
}
