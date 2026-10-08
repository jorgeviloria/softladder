//! ClassicLadder `.clp` / `.clprj` / `.clprjz` import and export.
//!
//! ClassicLadder stores a project as one text container that concatenates
//! several small parameter files. The layout is:
//!
//! ```text
//! _FILES_CLASSICLADDER
//! _FILE-general.txt
//! #VER=3.0
//! …
//! _/FILE-general.txt
//! _FILE-rung_0.csv
//! #VER=3.0
//! …
//! _/FILE-rung_0.csv
//! _/FILES_CLASSICLADDER
//! ```
//!
//! [`container`] parses and serializes that framing into an ordered list of
//! `(name, contents)` pairs, transparently decompressing the gzip streams that
//! `.clprjz` files contain. [`Document`] wraps those pairs in the API the rest
//! of the workspace uses.
//!
//! * [`import`] maps a document onto a [`Project`] and reports what had to be
//!   approximated; the parts SoftLadder does not model come back as a
//!   [`Document`] in [`ImportReport::extras`].
//! * [`export`] regenerates the reference files from a project and passes every
//!   part of the `extras` template through unchanged, so importing and
//!   immediately exporting a project preserves the parts SoftLadder does not
//!   model byte for byte.
//!
//! The normative description of the format is `docs/COMPAT.md` §8.

pub mod container;

mod document;
mod expr_map;
mod from_cl;
mod mapping;
mod to_cl;

use std::path::Path;

use softladder_core::{Diagnostic, Project};

use crate::ProjectError;

pub use container::{
    parse_container, serialize_container, CONTAINER_END, CONTAINER_START, FILE_END_PREFIX,
    FILE_PREFIX,
};
pub use document::Document;

/// What an import produced.
#[derive(Debug, Clone)]
pub struct ImportReport {
    /// The imported project.
    pub project: Project,
    /// Everything that had to be approximated or skipped, plus one `SL-W032`
    /// per part that is only passed through.
    pub diagnostics: Vec<Diagnostic>,
    /// The parts SoftLadder does not model, exactly as they appeared in the
    /// document; feed them back to [`export`] to keep them.
    pub extras: Document,
}

/// What an export produced.
#[derive(Debug, Clone)]
pub struct ExportReport {
    /// The generated container.
    pub document: Document,
    /// Everything the reference cannot express (`SL-W033`).
    pub diagnostics: Vec<Diagnostic>,
}

/// Converts a parsed ClassicLadder container into a SoftLadder project.
///
/// # Errors
///
/// Fails only when the container itself cannot be walked; a malformed part is
/// reported through [`ImportReport::diagnostics`] with `SL-E030`.
pub fn import(document: &Document) -> Result<ImportReport, ProjectError> {
    from_cl::import(document)
}

/// Converts a SoftLadder project into a ClassicLadder container.
///
/// `extras` is the template the exporter passes through: the
/// [`ImportReport::extras`] of the document the project came from, or
/// [`Document::empty`] for an authored project.
///
/// # Errors
///
/// Currently infallible; the signature keeps room for future I/O-backed
/// templates.
pub fn export(project: &Project, extras: &Document) -> Result<ExportReport, ProjectError> {
    to_cl::export(project, extras)
}

/// Parses `bytes` and imports them.
///
/// # Errors
///
/// Returns [`ProjectError::Container`] when the bytes are not a ClassicLadder
/// container (including a gzip stream that is not one).
pub fn import_bytes(bytes: &[u8]) -> Result<ImportReport, ProjectError> {
    import(&Document::parse(bytes)?)
}

/// Exports `project` to container bytes, optionally gzip-compressed.
///
/// # Errors
///
/// Returns [`ProjectError::Io`] when the gzip encoder fails.
pub fn export_bytes(
    project: &Project,
    extras: &Document,
    compress: bool,
) -> Result<Vec<u8>, ProjectError> {
    export(project, extras)?.document.to_bytes(compress)
}

/// Reads a ClassicLadder container file and imports it.
///
/// # Errors
///
/// Returns [`ProjectError::Io`] when the file cannot be read and
/// [`ProjectError::Container`] when it is not a container.
pub fn import_file(path: &Path) -> Result<ImportReport, ProjectError> {
    import_bytes(&std::fs::read(path)?)
}

/// Exports `project` to a ClassicLadder container file.
///
/// A `.clprjz` path is written gzip-compressed; every other extension is
/// written as plain text.
///
/// # Errors
///
/// Returns [`ProjectError::Io`] when the file cannot be written.
pub fn export_file(
    project: &Project,
    extras: &Document,
    path: &Path,
) -> Result<ExportReport, ProjectError> {
    let compress = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("clprjz"));
    let report = export(project, extras)?;
    std::fs::write(path, report.document.to_bytes(compress)?)?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documents_round_trip_through_the_container() {
        let text = concat!(
            "_FILES_CLASSICLADDER\n",
            "_FILE-general.txt\n",
            "#VER=3.0\n",
            "PERIODIC_REFRESH=50\n",
            "_/FILE-general.txt\n",
            "_/FILES_CLASSICLADDER\n",
        );
        let document = Document::parse(text.as_bytes()).expect("the container parses");
        assert_eq!(
            document.part("general.txt"),
            Some("#VER=3.0\nPERIODIC_REFRESH=50\n")
        );
        assert_eq!(document.serialize(), text);
    }

    #[test]
    fn an_empty_document_is_a_valid_template() {
        let project = Project::new("empty");
        let report = export(&project, &Document::empty()).expect("the export succeeds");
        assert!(report.document.part("general.txt").is_some());
        assert!(report.document.part("sections.csv").is_some());
        let reimported = import(&report.document).expect("the export re-imports");
        assert_eq!(reimported.project.name, "empty");
    }

    #[test]
    fn set_and_remove_keep_the_document_ordered() {
        let mut document = Document::empty();
        document.set_part("a.txt", "first\n".to_owned());
        document.set_part("b.txt", "second\n".to_owned());
        document.set_part("a.txt", "updated\n".to_owned());
        assert_eq!(document.parts().len(), 2);
        assert_eq!(document.part("a.txt"), Some("updated\n"));
        document.remove_part("a.txt");
        assert_eq!(document.part("a.txt"), None);
        assert_eq!(document.part("b.txt"), Some("second\n"));
    }

    #[test]
    fn a_gzip_document_can_be_exported_back_to_gzip() {
        let project = Project::new("gzip");
        let bytes = export_bytes(&project, &Document::empty(), true).expect("the export succeeds");
        assert!(bytes.starts_with(&[0x1f, 0x8b]));
        let report = import_bytes(&bytes).expect("the gzip container re-imports");
        assert_eq!(report.project.name, "gzip");
    }
}
