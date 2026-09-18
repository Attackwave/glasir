//! Audit log: who asked what, when, and how much came back.
//!
//! In a regulated environment this is the condition for being allowed to run
//! the tool at all, so its promises are the ones that matter rather than its
//! features: **never blocking a request, and never taking the server down.**
//!
//! Both come from the same decision — a writer thread behind a bounded channel.
//! A request hands over a finished record and returns; the thread does the
//! file I/O. If the channel is full, or the disk is, the record is dropped and
//! counted. A dropped audit line is bad; an agent hanging on a full filesystem,
//! or a server dying on it, is worse.
//!
//! One JSON object per line, appended, so it is greppable and can be shipped
//! elsewhere without parsing state.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, mpsc};

pub const AUDIT_FILE: &str = ".glasir-audit.jsonl";

/// Rotate at this size, keeping one previous generation.
///
/// 10 MB is roughly 50,000 records here. One generation rather than five: a
/// site that must retain more ships the file elsewhere, and rotating in the
/// server is only there to keep an unattended one from filling a disk.
const MAX_BYTES: u64 = 10 * 1024 * 1024;

/// Records buffered before writes start being dropped.
///
/// The point of the bound is that it exists: an unbounded queue turns a slow
/// disk into unbounded memory, which is the same outage by a different route.
const QUEUE: usize = 1024;

pub fn audit_path(root: &Path) -> PathBuf {
    root.join(AUDIT_FILE)
}

/// One answered request.
pub struct Record {
    pub who: String,
    pub tool: String,
    /// The question as asked, so "which part of the code" is answerable. Kept
    /// verbatim rather than summarised — a paraphrase cannot be audited.
    pub args: String,
    /// Bytes of the answer, which is what "result size" means to a caller
    /// paying for tokens.
    pub bytes: usize,
    pub ok: bool,
}

pub struct Audit {
    tx: mpsc::SyncSender<Command>,
    /// Set by the self-check to wedge the writer, which is the only way to
    /// provoke the case the non-blocking promise is about: a real disk that
    /// stalls. A refused `open` returns in microseconds and never fills the
    /// queue, so without this the promise is untestable.
    stall: Arc<AtomicU64>,
    /// Records the queue could not take. Reported rather than hidden: a log
    /// with silent holes is worse than one that says where they are.
    dropped: Arc<AtomicU64>,
}

enum Command {
    Record(Record),
    Flush(mpsc::SyncSender<()>),
}

impl Audit {
    /// Starts the writer thread. The thread ends when the last sender drops.
    ///
    /// `tree` names which tree these records are about, and it is taken here
    /// rather than per record because a server holds exactly one for its whole
    /// life — `mcp.rs` has no `root` at all, the tree lives only in the served
    /// state. Taking it per record would let a caller write the wrong one; here
    /// it cannot be wrong.
    pub fn start(path: PathBuf, tree: String) -> Audit {
        let (tx, rx) = mpsc::sync_channel::<Command>(QUEUE);
        let dropped = Arc::new(AtomicU64::new(0));
        let counter = dropped.clone();
        let stall = Arc::new(AtomicU64::new(0));
        let stalled = stall.clone();
        std::thread::spawn(move || {
            let mut missed = 0u64;
            for command in rx {
                match command {
                    Command::Record(rec) => {
                        let ms = stalled.load(Ordering::Relaxed);
                        if ms > 0 {
                            std::thread::sleep(std::time::Duration::from_millis(ms));
                        }
                        // The count is read here rather than by the writer, so a burst
                        // of drops becomes one note in the file instead of a line each.
                        let now = counter.swap(0, Ordering::Relaxed);
                        missed += now;
                        if write_line(&path, &tree, &rec, missed).is_ok() {
                            missed = 0;
                        }
                    }
                    Command::Flush(done) => {
                        let _ = done.send(());
                    }
                }
            }
        });
        Audit { tx, stall, dropped }
    }

    /// Wedges the writer for `ms` per record. Self-check only: see the
    /// non-blocking assert, which has no other way to reach this case.
    pub fn stall_for_test(&self, ms: u64) {
        self.stall.store(ms, Ordering::Relaxed);
    }

    /// Records dropped so far, for the self-check and the operator note.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// Hands a record to the writer. Never blocks: a full queue drops.
    pub fn record(&self, rec: Record) {
        if self.tx.try_send(Command::Record(rec)).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Waits until records accepted before this call reached the writer.
    /// Bounded by the caller's shutdown budget; a stalled disk still cannot
    /// wedge process termination indefinitely.
    pub fn flush(&self, timeout: std::time::Duration) -> bool {
        let (done_tx, done_rx) = mpsc::sync_channel(0);
        self.tx.try_send(Command::Flush(done_tx)).is_ok() && done_rx.recv_timeout(timeout).is_ok()
    }
}

/// Appends one record, rotating first if the file has grown past `MAX_BYTES`.
///
/// Every failure is swallowed by the caller: a full or read-only filesystem
/// must not reach the request path. That is deliberate and is what the
/// acceptance criterion asks for.
fn write_line(path: &Path, tree: &str, rec: &Record, missed: u64) -> std::io::Result<()> {
    if std::fs::metadata(path).map(|m| m.len()).unwrap_or(0) >= MAX_BYTES {
        // One generation: the previous .1 is overwritten rather than shifted.
        let _ = std::fs::rename(path, path.with_extension("jsonl.1"));
    }

    let mut line = serde_json::json!({
        "ts": crate::auth::now(),
        // Which tree this was asked about. Implicit while one process serves
        // one tree and its log sits beside it — and unrecoverable the moment
        // several logs are read together, which is the whole point of a control
        // plane. Written now so records made before it exists are still usable
        // then; adding the field later leaves a gap nothing can fill.
        "tree": tree,
        "who": rec.who,
        "tool": rec.tool,
        "args": rec.args,
        "bytes": rec.bytes,
        "ok": rec.ok,
    });
    if missed > 0 {
        line["dropped_before"] = missed.into();
    }

    let mut opts = std::fs::OpenOptions::new();
    opts.create(true).append(true);
    // Same reasoning as the token file: the questions people ask about a
    // codebase are not public, and a file that is briefly world-readable is a
    // file that can be read.
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    writeln!(opts.open(path)?, "{line}")
}

/// What `serve` prints and `status` reports: how many records, and when the
/// last one was written.
pub fn summary(path: &Path) -> Option<(usize, u64)> {
    let text = std::fs::read_to_string(path).ok()?;
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let last = lines
        .last()
        .and_then(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .and_then(|v| v["ts"].as_u64())
        .unwrap_or(0);
    Some((lines.len(), last))
}
