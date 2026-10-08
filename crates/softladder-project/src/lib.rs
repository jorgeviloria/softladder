//! Project persistence for SoftLadder.
//!
//! Two storage layers live here:
//!
//! * [`native`] — the SoftLadder project format: deterministic, pretty-printed
//!   JSON in `.slprj`, optionally gzip-compressed as `.slprjz`. Loading always
//!   sniffs the gzip magic number, so both spellings load through one entry
//!   point.
//! * [`classicladder`] — the text container that ClassicLadder writes into
//!   `.clprj` files (`_FILES_CLASSICLADDER` … `_/FILES_CLASSICLADDER`). The
//!   container is parsed and serialized here; the element-level mapping between
//!   container parts and a [`softladder_core::Project`] lands in M3.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod classicladder;
pub mod native;

use thiserror::Error;

/// Errors produced while loading or saving projects.
#[derive(Debug, Error)]
pub enum ProjectError {
    /// Underlying I/O failure.
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    /// JSON serialization or deserialization failure.
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    /// The file is not a valid SoftLadder/ClassicLadder container.
    #[error("container error: {0}")]
    Container(String),
    /// The requested feature is scheduled for a later milestone.
    #[error("not yet implemented (planned for {0})")]
    NotYetImplemented(&'static str),
}
