//! The on-disk graph format.
//!
//! This is the primary representation, not a cache of one: the bytes on disk
//! are a compact binary representation, validated before any traversal uses
//! them. The server snapshot is the normal production path; this lower-level
//! import helper keeps owned bytes too, so external file replacement cannot
//! invalidate live references.
//!
//! Any text format — JSON included — is an import path into this one, never a
//! thing a normal start reads.

use crate::csr::BaseCsr;
use std::path::Path;

/// True if the stored graph is at least as new as the input it was built from.
/// Only an import path needs this; once a producer writes the CSR directly
/// there is no second file to compare against.
///
/// Modification times are the cheap check, not a content hash: a false positive
/// costs one rebuild, and the graph is rebuilt on change anyway.
pub fn is_fresh(graph: &Path, source: &Path) -> bool {
    let (Ok(g), Ok(s)) = (std::fs::metadata(graph), std::fs::metadata(source)) else {
        return false;
    };
    match (g.modified(), s.modified()) {
        (Ok(g), Ok(s)) => g >= s,
        _ => false,
    }
}

pub fn write(csr: &BaseCsr, path: &Path) -> std::io::Result<()> {
    let bytes = serde_json::to_vec(csr).map_err(std::io::Error::other)?;
    std::fs::write(path, &bytes)
}

/// A validated CSR block held in owned memory.
///
/// Avoiding an mmap is deliberate: an external truncate after open makes an
/// mmap dereference undefined behaviour. Opening is rare, while this owned
/// graph remains safe and stable for every subsequent traversal.
pub struct MappedCsr {
    graph: BaseCsr,
}

impl MappedCsr {
    pub fn open(path: &Path) -> std::io::Result<Self> {
        let bytes = std::fs::read(path)?;
        let graph = serde_json::from_slice(&bytes).map_err(std::io::Error::other)?;
        Ok(Self { graph })
    }

    /// Access to the owned CSR after deserialization succeeded.
    pub fn graph(&self) -> std::io::Result<&BaseCsr> {
        Ok(&self.graph)
    }
}
