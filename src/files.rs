//! Writes that never pass through a symbolic link.
//!
//! A tree can carry a link where an output file goes; writing through it
//! would change a file outside the tree.

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
