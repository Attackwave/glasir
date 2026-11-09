//! Reading and writing the tree's files.
//!
//! Writes never pass through a symbolic link: a tree can carry a link where
//! an output file goes, and writing through it would change a file outside
//! the tree.

use crate::parse_ast::LangExt;
use std::io::Write;
use std::path::Path;

fn refuse_link(path: &Path) -> std::io::Result<()> {
    if std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(std::io::Error::other(format!(
            "{} is a symbolic link; not written",
            path.display()
        )));
    }
    Ok(())
}

/// Replaces `path` with `bytes` through a sibling file and a rename, keeping
/// the old file's permissions. A reader sees the old or the new content.
pub fn replace(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    refuse_link(path)?;
    let name = path
        .file_name()
        .map_or("out".into(), |n| n.to_string_lossy());
    let tmp = path.with_file_name(format!(".{name}.{}.tmp", std::process::id()));
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    let written = opts.open(&tmp).and_then(|mut f| {
        f.write_all(bytes)?;
        if let Ok(old) = std::fs::metadata(path) {
            f.set_permissions(old.permissions())?;
        }
        f.sync_all()
    });
    match written.and_then(|()| std::fs::rename(&tmp, path)) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// Options for appending to `path`: created with `mode`, never opened through
/// a link.
pub fn append_options(mode: u32) -> std::fs::OpenOptions {
    let mut opts = std::fs::OpenOptions::new();
    opts.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(mode).custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(not(unix))]
    let _ = mode;
    opts
}

/// `replace` for a file inside `root`; elsewhere (a user-level file a dotfile
/// manager may link) the link is followed as before.
pub fn replace_in(root: &Path, path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if path.starts_with(root) {
        replace(path, bytes)
    } else {
        std::fs::write(path, bytes)
    }
}

/// Largest source file read, in bytes.
///
/// A generated file — protobuf output, bindings, a migration — can run to tens
/// of megabytes, and eight of those parse in parallel. Measured: a 12 MB file
/// did not finish in two minutes before the parser's reference attribution was
/// made linear, and even now the memory of eight simultaneous syntax trees is
/// the risk.
///
/// **1 MiB, and the 4 MiB it replaces was never measured.** What costs is a
/// deeply nested generated table, not size as such: parsing `i386-tbl.h`
/// (5.1 MB of one initialiser list) costs 2.45 s and **486 MB of resident
/// memory for two symbols**, against 113 ms and 49 MB for this whole
/// repository's 1,384. The parse pool runs eight files at once, so the worst
/// case *under the old limit* is eight of those: measured at **3.0 GB RSS and
/// zero symbols**. Cost is linear in file size (0.34/0.65/1.28/2.60 s over
/// 0.7/1.4/2.7/5.3 MB), so the limit is the only thing bounding it.
///
/// 1 MiB is where the two populations separate, measured over 14,294 source
/// files in five trees: the largest hand-written file is `tc-arm.c` at 793 KB
/// (1,112 symbols), and everything above 1 MB is a generated table —
/// `arc-tbl.h` 1.2 MB/**0** symbols, `m32c-opc.h` 1.1 MB/11, `m32c-opc.c`
/// 4.0 MB/23. The cap drops 0.04% of files and costs no hand-written code.
pub(crate) const MAX_SOURCE_BYTES: u64 = 1024 * 1024;

/// Reads a source file, reporting the reasons it might not be readable.
///
/// `read_to_string(..).ok()` treats three different situations as one: a file
/// that is not valid UTF-8 (Latin-1 sources are common in older C and Java
/// trees), a file the process may not read, and a file that is simply absent.
/// All three vanished from the graph without a word — measured, a tree of three
/// files yielded one symbol and reported success. A tool that silently indexes
/// half a repository is worse than one that refuses, because nobody knows to
/// look.
pub(crate) fn read_source(path: &std::path::Path) -> Option<String> {
    match std::fs::metadata(path) {
        Ok(m) if m.len() > MAX_SOURCE_BYTES => {
            eprintln!(
                "glasir: skipping {} — {:.1} MB exceeds the {} MB limit for one file",
                path.display(),
                m.len() as f64 / (1024.0 * 1024.0),
                MAX_SOURCE_BYTES / (1024 * 1024)
            );
            return None;
        }
        Ok(_) => {}
        Err(e) => {
            eprintln!("glasir: skipping {} — {e}", path.display());
            return None;
        }
    }
    match std::fs::read(path) {
        Ok(bytes) => match String::from_utf8(bytes) {
            // Without its byte order mark: no scanner reads it as part of
            // the first word, and every reader sees the same offsets.
            Ok(text) => Some(match text.strip_prefix('\u{feff}') {
                Some(rest) => rest.to_string(),
                None => text,
            }),
            // Latin-1 fallback. Refusing the file loses all of it over bytes
            // that are almost never in the code: measured on five real
            // non-UTF-8 files, every one
            // is broken by a single byte in a copyright header — `Kämpf`,
            // `Jean-François`, `©` — while the code below is plain ASCII.
            // Indexing them takes that fixture from **52 to 268 symbols**.
            //
            // Latin-1 is the right fallback because it cannot fail: every byte
            // maps to the code point of the same value, so this decodes any
            // input rather than refusing a second time. It may mis-read a
            // different 8-bit encoding's accents, but an identifier is ASCII
            // in every one of them, and a wrong accent in a comment costs a
            // term where refusing costs the file.
            Err(e) => {
                let text: String = e.as_bytes().iter().map(|&b| b as char).collect();
                eprintln!(
                    "glasir: {} is not valid UTF-8 — reading it as Latin-1",
                    path.display()
                );
                Some(text)
            }
        },
        Err(e) => {
            eprintln!("glasir: skipping {} — {e}", path.display());
            None
        }
    }
}

/// The user's home directory. Windows sets `USERPROFILE` and usually no
/// `HOME`, so reading `HOME` alone made `--user` refuse and every client's
/// home-directory marker invisible there. `HOME` first, which is what the
/// checks set.
pub(crate) fn home() -> Option<std::path::PathBuf> {
    home_from(std::env::var_os("HOME"), std::env::var_os("USERPROFILE"))
}

pub(crate) fn home_from(
    home: Option<std::ffi::OsString>,
    profile: Option<std::ffi::OsString>,
) -> Option<std::path::PathBuf> {
    home.or(profile).map(std::path::PathBuf::from)
}

/// Source files under `root` in the indexed scope (`watcher::files`), in
/// path order, which is what decides the ids a fresh build assigns.
pub(crate) fn walk(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    crate::watcher::files(root, |e| eprintln!("glasir: {e}"))
        .unwrap_or_default()
        .into_iter()
        .filter(|p| {
            crate::parse_ast::Lang::from_path(p).is_some()
                || crate::docs::is_markdown(p)
                    && (!crate::docs::is_instructions(p) || crate::watcher::opted_in(root, p))
        })
        .collect()
}
