//! One server per tree, whatever number of clients ask.
//!
//! `serve` over stdio is a thin relay: it connects to the tree's backend on a
//! local socket, starting it when none runs, and passes lines both ways. The
//! backend holds the graph, the watcher and the sleep, answers every client,
//! and ends a minute after the last one leaves. A relay a client forgets to
//! close costs a few megabytes and no polling.
//!
//! Unix only: elsewhere `serve` keeps one server per client.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

/// What decides that two clients may share a backend: the binary, the tree
/// and the options that change what it answers. The binary by its file
/// identity beside the version: a build installed over one of the same
/// version must not reach a backend still running the old code. A copy may
/// keep the source's time (macOS does), so the inode counts too; an install
/// by rename always makes a new one.
pub fn key(root: &Path, options: &[String]) -> String {
    let binary = std::env::current_exe()
        .and_then(std::fs::metadata)
        .map(|m| {
            let at = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok());
            #[cfg(unix)]
            let inode = std::os::unix::fs::MetadataExt::ino(&m);
            #[cfg(not(unix))]
            let inode = 0;
            format!("{inode}:{}:{}", m.len(), at.map_or(0, |d| d.as_nanos()))
        })
        .unwrap_or_default();
    let mut key = format!(
        "{}\0{binary}\0{}",
        env!("CARGO_PKG_VERSION"),
        root.display()
    );
    for option in options {
        key.push('\0');
        key.push_str(option);
    }
    key
}

/// The backend's socket for `key`, in the per-user state directory. `None`
/// where the path would not fit a socket address.
pub fn socket(key: &str) -> Option<PathBuf> {
    let mut hash: u64 = 0x9e37_79b9_7f4a_7c15;
    for byte in key.bytes() {
        hash = splitmix(hash ^ u64::from(byte));
    }
    let dir = crate::servers::directory()?.parent()?.join("sockets");
    let path = dir.join(format!("{hash:016x}.sock"));
    // `sun_path` holds 108 bytes on Linux, 104 on macOS.
    (path.as_os_str().len() < 100).then_some(path)
}

fn splitmix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// What answers one line of a session, `None` for a notification.
pub type Answer = std::sync::Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

#[cfg(unix)]
mod unix {
    use super::*;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    fn private_dir(dir: &Path) -> std::io::Result<()> {
        use std::os::unix::fs::PermissionsExt;
        std::fs::create_dir_all(dir)?;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
    }

    /// Connects to the backend, starting one when none answers. A lock beside
    /// the socket lets one relay start it while the others wait.
    pub fn connect(
        socket: &Path,
        start: &mut dyn FnMut() -> std::io::Result<()>,
    ) -> std::io::Result<UnixStream> {
        if let Ok(stream) = UnixStream::connect(socket) {
            return Ok(stream);
        }
        let dir = socket
            .parent()
            .ok_or_else(|| std::io::Error::other("no socket directory"))?;
        private_dir(dir)?;
        let lock = std::fs::File::create(socket.with_extension("lock"))?;
        lock.lock()?;
        if let Ok(stream) = UnixStream::connect(socket) {
            return Ok(stream);
        }
        // Nothing answers: what is there was left by a backend that died.
        let _ = std::fs::remove_file(socket);
        start()?;
        // The backend binds before it reads the tree, so this waits for a
        // process to start, not for a graph to build.
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match UnixStream::connect(socket) {
                Ok(stream) => return Ok(stream),
                Err(e) if Instant::now() > deadline => return Err(e),
                Err(_) => std::thread::sleep(Duration::from_millis(20)),
            }
        }
    }

    /// Relays stdin to the backend and its answers to stdout until the client
    /// closes stdin. A backend that goes away mid-session is started again
    /// and the line that found it gone is sent anew.
    pub fn relay(
        socket: &Path,
        first: UnixStream,
        mut start: impl FnMut() -> std::io::Result<()>,
    ) -> std::io::Result<()> {
        let stdout = Arc::new(std::sync::Mutex::new(std::io::stdout()));
        let open =
            |stream: UnixStream| -> std::io::Result<(UnixStream, std::thread::JoinHandle<()>)> {
                let reader = stream.try_clone()?;
                let stdout = stdout.clone();
                let answers = std::thread::spawn(move || {
                    for line in BufReader::new(reader).lines().map_while(Result::ok) {
                        let mut out = stdout.lock().unwrap_or_else(|e| e.into_inner());
                        if writeln!(out, "{line}").and_then(|()| out.flush()).is_err() {
                            break;
                        }
                    }
                });
                Ok((stream, answers))
            };
        let (mut stream, mut answers) = open(first)?;
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            let line = line?;
            if writeln!(stream, "{line}").is_err() {
                (stream, answers) = open(connect(socket, &mut start)?)?;
                writeln!(stream, "{line}")?;
            }
        }
        // The backend answers what it has read, then closes: wait for that.
        let _ = stream.shutdown(std::net::Shutdown::Write);
        let _ = answers.join();
        Ok(())
    }

    /// Takes the backend's place at `socket`, before the tree is read, so a
    /// relay can connect at once. `None` when another backend holds it.
    pub fn bind(socket: &Path) -> std::io::Result<Option<UnixListener>> {
        use std::os::unix::fs::PermissionsExt;
        if let Some(dir) = socket.parent() {
            private_dir(dir)?;
        }
        match UnixListener::bind(socket) {
            Ok(listener) => {
                std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600))?;
                Ok(Some(listener))
            }
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Serves every client of one tree, and ends once none has been connected
    /// for `linger`.
    pub fn serve(
        listener: UnixListener,
        socket: &Path,
        linger: Duration,
        answer: Answer,
    ) -> std::io::Result<()> {
        let clients = Arc::new(AtomicUsize::new(0));
        let last_left = Arc::new(std::sync::Mutex::new(Instant::now()));
        {
            let (clients, last_left, socket) =
                (clients.clone(), last_left.clone(), socket.to_path_buf());
            std::thread::spawn(move || {
                let mut unlinked = false;
                loop {
                    std::thread::sleep(Duration::from_millis(250).min(linger));
                    let idle = last_left
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .elapsed();
                    if clients.load(Ordering::SeqCst) != 0 || idle < linger {
                        continue;
                    }
                    // Gone from the path first: a relay arriving now starts a
                    // new backend instead of reaching this one as it ends. Only
                    // once, since a new backend may hold the path afterwards.
                    if !unlinked {
                        let _ = std::fs::remove_file(&socket);
                        unlinked = true;
                        // ponytail: a fixed grace for connections made before
                        // the unlink to be accepted; a stall longer than this
                        // still ends one of them.
                        std::thread::sleep(Duration::from_secs(1));
                    }
                    if clients.load(Ordering::SeqCst) == 0 {
                        std::process::exit(0);
                    }
                }
            });
        }
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            clients.fetch_add(1, Ordering::SeqCst);
            let (clients, last_left, answer) = (clients.clone(), last_left.clone(), answer.clone());
            std::thread::spawn(move || {
                let _ = session(stream, answer.as_ref());
                *last_left.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
                clients.fetch_sub(1, Ordering::SeqCst);
            });
        }
        Ok(())
    }

    fn session(
        stream: UnixStream,
        answer: &(dyn Fn(&str) -> Option<String> + Send + Sync),
    ) -> std::io::Result<()> {
        let mut writer = stream.try_clone()?;
        for line in BufReader::new(stream).lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            if let Some(reply) = answer(&line) {
                writeln!(writer, "{reply}")?;
                writer.flush()?;
            }
        }
        Ok(())
    }
}

#[cfg(unix)]
pub use unix::{bind, connect, relay, serve};
