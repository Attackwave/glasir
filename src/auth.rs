//! Per-user tokens: who may ask, until when, and under which name.
//!
//! A single shared `--token` is right for one machine and too coarse for a
//! team — it cannot be attributed, revoked or expired. This adds a file of
//! tokens, one per person, that the server re-reads whenever it changes, so a
//! revocation takes effect without a restart. That is the acceptance criterion
//! for B.2, and re-reading on mtime is what makes it true.
//!
//! **The file never holds a usable secret.** Only the SHA-256 of each token is
//! stored; the plaintext is shown once, when it is minted, and cannot be
//! recovered. A backup, a stray `git add` or a glance over a shoulder then
//! leaks nothing that can be used to connect. The cost is deliberate: a lost
//! token is re-issued, never looked up.
//!
//! Format is one record per line, tab-separated, so it is greppable and
//! editable by hand — revoking someone is deleting their line:
//!
//! ```text
//! <sha256-hex>\t<name>\t<expiry-unix-or-0>
//! ```

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// Where the tokens live, beside the stored graph and by the same convention.
pub const TOKEN_FILE: &str = ".glasir-tokens";

pub fn token_path(root: &Path) -> PathBuf {
    root.join(TOKEN_FILE)
}

/// One person's token, as stored.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub hash: String,
    pub name: String,
    /// Unix seconds, or 0 for a token that does not expire.
    pub expires: u64,
}

impl Entry {
    fn parse(line: &str) -> Option<Entry> {
        let mut f = line.split('\t');
        let hash = f.next()?.trim().to_string();
        let name = f.next()?.trim().to_string();
        let expires = f.next().unwrap_or("0").trim().parse().unwrap_or(0);
        // A line missing either half is corrupt, not a token that matches
        // everything — refuse it rather than letting it into the set.
        if hash.len() != 64 || name.is_empty() {
            return None;
        }
        Some(Entry {
            hash,
            name,
            expires,
        })
    }

    fn line(&self) -> String {
        format!("{}\t{}\t{}\n", self.hash, self.name, self.expires)
    }
}

/// Reads the token file, skipping blank lines, comments and corrupt records.
///
/// A missing file is an empty set, not an error: a server with no token file is
/// simply one that has no per-user tokens configured.
pub fn read(path: &Path) -> Vec<Entry> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(Entry::parse)
        .collect()
}

/// Appends an entry, creating the file with owner-only permissions.
///
/// Owner-only from the moment it exists, not set afterwards: a file that is
/// briefly world-readable is a file that can be read.
pub fn append(path: &Path, entry: &Entry) -> std::io::Result<()> {
    use std::io::Write;
    crate::files::append_options(0o600)
        .open(path)?
        .write_all(entry.line().as_bytes())
}

/// Removes every entry with this name, returning how many went.
///
/// By name rather than by token: revoking is something an operator does about a
/// person, and they do not have the token — that is the point of storing only
/// its hash.
pub fn revoke(path: &Path, name: &str) -> std::io::Result<usize> {
    let kept: Vec<Entry> = read(path).into_iter().filter(|e| e.name != name).collect();
    let gone = read(path).len() - kept.len();
    if gone > 0 {
        let body: String = kept.iter().map(|e| e.line()).collect();
        crate::files::replace(path, body.as_bytes())?;
    }
    Ok(gone)
}

/// Replaces every token for `name` with one freshly minted credential.
///
/// The completed file is synced and atomically renamed into place, so a server
/// reloading tokens sees either the old complete set or the new complete set,
/// never a truncate-in-progress file. This is the service-token primitive used
/// when a control-plane backend credential must be changed.
pub fn rotate(path: &Path, name: &str, expires: u64) -> std::io::Result<String> {
    use std::io::Write;

    let secret = mint()?;
    let mut entries: Vec<Entry> = read(path).into_iter().filter(|e| e.name != name).collect();
    entries.push(Entry {
        hash: sha256_hex(secret.as_bytes()),
        name: name.to_string(),
        expires,
    });
    let body: String = entries.iter().map(Entry::line).collect();
    let tmp = path.with_extension(format!("tokens-{}.tmp", std::process::id()));
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut file = opts.open(&tmp)?;
    if let Err(e) = file
        .write_all(body.as_bytes())
        .and_then(|_| file.sync_all())
    {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    #[cfg(windows)]
    {
        let _ = std::fs::remove_file(path);
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(secret)
}

/// Who this token belongs to, or `None` if it is unknown or expired.
///
/// Compared by hash, so an expired or unknown token is refused without the
/// plaintext ever being stored anywhere to compare against.
pub fn identify(entries: &[Entry], token: &str, now: u64) -> Option<String> {
    let hash = sha256_hex(token.as_bytes());
    entries
        .iter()
        .find(|e| e.hash == hash && (e.expires == 0 || e.expires > now))
        .map(|e| e.name.clone())
}

/// The token file, re-read when its identity or contents change.
///
/// Revoking without a restart is the acceptance criterion. Metadata catches
/// normal edits cheaply, while a content fingerprint closes the Windows case
/// where an atomic replacement can preserve both file length and timestamp.
/// Token files are intentionally small, so hashing them per request is a
/// bounded cost for a stronger revocation guarantee.
pub struct Tokens {
    path: PathBuf,
    cache: Mutex<(Option<FileStamp>, Vec<Entry>)>,
}

/// Identity of the token file, not merely its modification time. Atomic token
/// rotation replaces the inode; relying on mtime alone can miss that replace
/// on a filesystem with coarse timestamp resolution.
#[derive(Clone, Debug, PartialEq, Eq)]
struct FileStamp {
    modified: Option<SystemTime>,
    len: u64,
    content_hash: String,
    #[cfg(unix)]
    inode: u64,
}

fn stamp(path: &Path) -> Option<FileStamp> {
    let metadata = std::fs::metadata(path).ok()?;
    let content_hash = sha256_hex(&std::fs::read(path).ok()?);
    #[cfg(unix)]
    use std::os::unix::fs::MetadataExt;
    Some(FileStamp {
        modified: metadata.modified().ok(),
        len: metadata.len(),
        content_hash,
        #[cfg(unix)]
        inode: metadata.ino(),
    })
}

impl Tokens {
    pub fn new(path: PathBuf) -> Tokens {
        Tokens {
            path,
            cache: Mutex::new((None, Vec::new())),
        }
    }

    /// True when the file holds at least one token, so the server knows whether
    /// per-user auth is configured at all.
    pub fn configured(&self) -> bool {
        !self.current().is_empty()
    }

    pub fn current(&self) -> Vec<Entry> {
        let current = stamp(&self.path);
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        if cache.0 != current {
            *cache = (current, read(&self.path));
        }
        cache.1.clone()
    }

    /// Who is asking, or `None` if the token is unknown, expired or absent.
    pub fn identify(&self, token: Option<&str>, now: u64) -> Option<String> {
        identify(&self.current(), token?, now)
    }
}

/// A token to hand out: 32 bytes of system randomness, hex-encoded.
///
/// From the operating system rather than a seeded generator — everything else
/// in this codebase is deterministic on purpose, and a predictable credential
/// is the one place where that would be a hole rather than a feature.
pub fn mint() -> std::io::Result<String> {
    let mut bytes = [0u8; 32];
    system_random(&mut bytes)?;
    Ok(hex(&bytes))
}

/// The operating system's random source, through ring: `getrandom` on
/// Linux, `BCryptGenRandom` on Windows, `SecRandomCopyBytes` on macOS.
fn system_random(out: &mut [u8]) -> std::io::Result<()> {
    use ring::rand::SecureRandom;
    ring::rand::SystemRandom::new()
        .fill(out)
        .map_err(|_| std::io::Error::other("the system random source failed"))
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// SHA-256 as lowercase hex, from ring.
pub fn sha256_hex(msg: &[u8]) -> String {
    hex(ring::digest::digest(&ring::digest::SHA256, msg).as_ref())
}
