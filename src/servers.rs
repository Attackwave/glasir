//! Running `serve` processes, so the ones a client replaced can be found.
//!
//! A stdio server ends when its client closes the connection. A client that
//! starts a new server and keeps the old connection open leaves the old one
//! running for good, and only the client can tell the two apart: a server
//! that has had no request since a newer one for the same tree and the same
//! parent started is the replaced one.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// One running server as it describes itself.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Status {
    pub pid: u32,
    pub parent: u32,
    pub root: String,
    pub version: String,
    pub started: u64,
    pub last_request: u64,
}

/// Where the statuses live: per user, never shared.
pub fn directory() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("LOCALAPPDATA").map(PathBuf::from))
        .or_else(|| crate::files::home().map(|h| h.join(".local").join("state")))?;
    Some(base.join("glasir").join("servers"))
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn parent_pid() -> u32 {
    #[cfg(unix)]
    {
        std::os::unix::process::parent_id()
    }
    #[cfg(not(unix))]
    {
        0
    }
}

/// The status file of this process, while it serves.
static OWN: std::sync::OnceLock<(PathBuf, Status)> = std::sync::OnceLock::new();
/// When the status last recorded a request; written at most once a minute.
static RECORDED: AtomicU64 = AtomicU64::new(0);

/// Removes this server's status when serving ends.
pub struct Registration;

impl Drop for Registration {
    fn drop(&mut self) {
        if let Some((path, _)) = OWN.get() {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Records this process as serving `root`. Best effort: a server that cannot
/// write its status still serves.
pub fn register(root: &Path) -> Registration {
    let Some(dir) = directory() else {
        return Registration;
    };
    let started = now();
    let status = Status {
        pid: std::process::id(),
        parent: parent_pid(),
        root: root.to_string_lossy().into_owned(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        started,
        last_request: started,
    };
    LAST.store(started, Ordering::Relaxed);
    if write(&dir, &status).is_ok() {
        let path = dir.join(format!("{}.json", status.pid));
        let _ = OWN.set((path, status));
        RECORDED.store(started, Ordering::Relaxed);
    }
    Registration
}

fn write(dir: &Path, status: &Status) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    let path = dir.join(format!("{}.json", status.pid));
    let partial = dir.join(format!(".{}.json", status.pid));
    std::fs::write(&partial, serde_json::to_vec(status)?)?;
    std::fs::rename(partial, path)
}

/// Notes a request. Cheap on every call: the file is rewritten at most once a
/// minute, which is finer than any decision taken on it.
pub fn touch() {
    let now = now();
    LAST.store(now, Ordering::Relaxed);
    if let Some(wake) = SLEEPER.get() {
        wake();
    }
    let Some((path, status)) = OWN.get() else {
        return;
    };
    let last = RECORDED.load(Ordering::Relaxed);
    if now < last + 60
        || RECORDED
            .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
            .is_err()
    {
        return;
    }
    let mut status = status.clone();
    status.last_request = now;
    if let Some(dir) = path.parent() {
        let _ = write(dir, &status);
    }
}

/// Every status in `dir`, whatever state its process is in.
pub fn read(dir: &Path) -> Vec<Status> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<Status> = entries
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with(".json"))
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .filter_map(|e| std::fs::read(e.path()).ok())
        .filter_map(|bytes| serde_json::from_slice(&bytes).ok())
        .collect();
    out.sort_by_key(|s: &Status| (s.root.clone(), s.started, s.pid));
    out
}

/// The servers a newer one replaced: same tree, same parent, and not one
/// request since the newer one started. A second client still using its own
/// server keeps it.
pub fn replaced(live: &[Status]) -> Vec<u32> {
    let mut out = Vec::new();
    for status in live {
        let newer = live.iter().any(|other| {
            other.pid != status.pid
                && other.root == status.root
                && other.parent == status.parent
                && other.parent != 0
                && other.started > status.started
                && status.last_request < other.started
        });
        if newer {
            out.push(status.pid);
        }
    }
    out.sort_unstable();
    out
}

/// What the process `pid` is running, if it still runs: its command line.
#[cfg(unix)]
pub fn command_of(pid: u32) -> Option<String> {
    let out = std::process::Command::new("ps")
        .args(["-o", "args=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    let line = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !line.is_empty()).then_some(line)
}

#[cfg(not(unix))]
pub fn command_of(_pid: u32) -> Option<String> {
    None
}

/// Whether `pid` is still the server its status describes, and not another
/// process that took the number since.
pub fn is_server(status: &Status) -> bool {
    command_of(status.pid).is_some_and(|line| {
        line.contains("glasir") && line.contains(" serve") && line.contains(&status.root)
    })
}

/// When this process last answered a request, in seconds.
static LAST: AtomicU64 = AtomicU64::new(0);

/// Seconds since the last request this process answered.
pub fn idle_for(now: u64) -> u64 {
    now.saturating_sub(LAST.load(Ordering::Relaxed))
}

/// A served graph that leaves memory while no client asks and comes back on
/// the next request. The connection stays open: a stdio client does not start
/// a stopped server again, so stopping would cut the session off.
pub struct Sleeper<T> {
    state: std::sync::Arc<crate::published::Published<T>>,
    asleep: std::sync::atomic::AtomicBool,
    turn: std::sync::Mutex<()>,
    empty: Box<dyn Fn(&T) -> T + Send + Sync>,
    load: Box<dyn Fn() -> Option<T> + Send + Sync>,
}

impl<T> Sleeper<T> {
    pub fn new(
        state: std::sync::Arc<crate::published::Published<T>>,
        empty: impl Fn(&T) -> T + Send + Sync + 'static,
        load: impl Fn() -> Option<T> + Send + Sync + 'static,
    ) -> Self {
        Self {
            state,
            asleep: std::sync::atomic::AtomicBool::new(false),
            turn: std::sync::Mutex::new(()),
            empty: Box::new(empty),
            load: Box::new(load),
        }
    }

    pub fn is_asleep(&self) -> bool {
        self.asleep.load(Ordering::Acquire)
    }

    /// Drops the graph when nothing asked for `after` seconds. `idle` is read
    /// under the lock every `wake` takes, so a request that noted itself
    /// before is seen, and one after finds the server asleep and loads.
    pub fn sleep_if_idle(&self, idle: impl Fn() -> u64, after: u64) {
        let _turn = self.turn.lock().unwrap_or_else(|e| e.into_inner());
        if after == 0 || self.is_asleep() || idle() < after {
            return;
        }
        let empty = (self.empty)(&self.state.load());
        self.state.store(std::sync::Arc::new(empty));
        self.asleep.store(true, Ordering::Release);
    }

    /// Loads the graph again before a request is answered. A failed load keeps
    /// the server asleep, so the next request tries again.
    pub fn wake(&self) {
        let _turn = self.turn.lock().unwrap_or_else(|e| e.into_inner());
        if !self.is_asleep() {
            return;
        }
        if let Some(fresh) = (self.load)() {
            self.state.store(std::sync::Arc::new(fresh));
            self.asleep.store(false, Ordering::Release);
        }
    }
}

/// The sleeper `touch` wakes, set once per process.
static SLEEPER: std::sync::OnceLock<Box<dyn Fn() + Send + Sync>> = std::sync::OnceLock::new();

/// Wakes the served graph before each request.
pub fn on_request(wake: impl Fn() + Send + Sync + 'static) {
    let _ = SLEEPER.set(Box::new(wake));
}
