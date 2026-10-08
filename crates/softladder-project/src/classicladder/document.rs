//! The in-memory representation of a ClassicLadder container.
//!
//! A [`Document`] is an ordered list of `(name, contents)` parameter files,
//! exactly as they appear in the container. Keeping the order and the original
//! bytes of every part is what makes passthrough possible: the importer hands
//! the parts SoftLadder does not model back to the caller, and the exporter
//! writes them out again untouched.

use std::io::Write;

use flate2::write::GzEncoder;
use flate2::Compression;

use crate::classicladder::container::{parse_container, serialize_container};
use crate::ProjectError;

/// An ordered ClassicLadder container: the parameter files it holds, in file
/// order.
///
/// The contents of a part keep their original line terminators, so serializing
/// a document that was only parsed reproduces its input byte for byte.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Document {
    parts: Vec<(String, String)>,
}

impl Document {
    /// Parses `bytes` as a ClassicLadder container, sniffing the gzip magic
    /// number first so that `.clprj` and `.clprjz` share one entry point.
    ///
    /// # Errors
    ///
    /// Returns [`ProjectError::Container`] when the bytes are not a container,
    /// are not valid UTF-8, or are a gzip stream that cannot be decompressed.
    pub fn parse(bytes: &[u8]) -> Result<Self, ProjectError> {
        Ok(Self {
            parts: parse_container(bytes)?,
        })
    }

    /// Builds a document from already parsed `(name, contents)` pairs.
    pub fn from_parts(parts: Vec<(String, String)>) -> Self {
        Self { parts }
    }

    /// Builds an empty document, which the exporter accepts as "no template".
    pub fn empty() -> Self {
        Self::default()
    }

    /// Renders the container as text.
    pub fn serialize(&self) -> String {
        serialize_container(&self.parts)
    }

    /// Renders the container as bytes, optionally gzip-compressed.
    ///
    /// # Errors
    ///
    /// Returns [`ProjectError::Io`] when the gzip encoder fails.
    pub fn to_bytes(&self, compress: bool) -> Result<Vec<u8>, ProjectError> {
        let text = self.serialize();
        if !compress {
            return Ok(text.into_bytes());
        }
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(text.as_bytes())?;
        Ok(encoder.finish()?)
    }

    /// The parts of the document, in file order.
    pub fn parts(&self) -> &[(String, String)] {
        &self.parts
    }

    /// The contents of the first part named `name`, if any.
    pub fn part(&self, name: &str) -> Option<&str> {
        self.parts
            .iter()
            .find(|(part, _)| part == name)
            .map(|(_, contents)| contents.as_str())
    }

    /// Replaces the contents of the first part named `name`, or appends a new
    /// part when the document has none.
    pub fn set_part(&mut self, name: &str, contents: String) {
        match self.parts.iter_mut().find(|(part, _)| part == name) {
            Some(slot) => slot.1 = contents,
            None => self.parts.push((name.to_owned(), contents)),
        }
    }

    /// Removes every part named `name`.
    pub fn remove_part(&mut self, name: &str) {
        self.parts.retain(|(part, _)| part != name);
    }
}
