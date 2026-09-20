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
    let mut opts = std::fs::OpenOptions::new();
    opts.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    opts.open(path)?.write_all(entry.line().as_bytes())
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
        std::fs::write(path, body)?;
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

/// Fills `out` from the operating system's entropy source.
///
/// `/dev/urandom` on Unix, `RtlGenRandom` on Windows. Written out rather than
/// taking a dependency for thirty lines — and it has to exist, because
/// `token add` is an enterprise feature and reading `/dev/urandom` made it fail
/// on the one platform where an operator is most likely to run it.
#[cfg(unix)]
fn system_random(out: &mut [u8]) -> std::io::Result<()> {
    use std::io::Read;
    std::fs::File::open("/dev/urandom")?.read_exact(out)
}

#[cfg(windows)]
fn system_random(out: &mut [u8]) -> std::io::Result<()> {
    // `SystemFunction036` is `RtlGenRandom`, present since Windows XP and the
    // documented way to get entropy without pulling in the whole CryptoAPI.
    #[link(name = "advapi32")]
    unsafe extern "system" {
        #[link_name = "SystemFunction036"]
        fn rtl_gen_random(buffer: *mut u8, length: u32) -> u8;
    }
    // SAFETY: `out` is a valid, uniquely borrowed slice of exactly `len` bytes.
    let ok = unsafe { rtl_gen_random(out.as_mut_ptr(), out.len() as u32) };
    if ok == 0 {
        return Err(std::io::Error::other("RtlGenRandom failed"));
    }
    Ok(())
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

/// SHA-256, per FIPS 180-4.
///
/// Written out rather than pulled in: it is fifty lines of fully specified
/// arithmetic against a dependency in a binary that has none for this, and the
/// published test vectors make it verifiable rather than merely plausible.
pub fn sha256_hex(msg: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];

    let mut data = msg.to_vec();
    let bits = (msg.len() as u64) * 8;
    data.push(0x80);
    while data.len() % 64 != 56 {
        data.push(0);
    }
    data.extend_from_slice(&bits.to_be_bytes());

    for chunk in data.chunks(64) {
        let mut w = [0u32; 64];
        for (i, word) in chunk.chunks(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (slot, v) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *slot = slot.wrapping_add(v);
        }
    }

    hex(&h.iter().flat_map(|w| w.to_be_bytes()).collect::<Vec<u8>>())
}
