//! ClassicLadder `.clprj` container support.
//!
//! ClassicLadder stores a project as one text file that concatenates several
//! small parameter files. The layout is:
//!
//! ```text
//! _FILES_CLASSICLADDER
//! _FILE-general.txt
//! #VER=3.0
//! …
//! _/FILE-general.txt
//! _FILE-rungs.txt
//! #VER=3.0
//! …
//! _/FILE-rungs.txt
//! _/FILES_CLASSICLADDER
//! ```
//!
//! [`container`] parses and serializes that framing into an ordered list of
//! `(name, content)` pairs, transparently decompressing the gzip streams that
//! `.clprjz` files contain.
//!
//! **Milestone M3.** Mapping the individual files (`general.txt`, `rungs.txt`,
//! `symbols.txt`, `timers_iec.csv`, …) onto a
//! [`softladder_core::Project`] is not implemented yet. [`import_container`]
//! and [`export_container`] therefore return
//! [`ProjectError::NotYetImplemented`] with the milestone name instead of
//! guessing.

pub mod container;

use std::path::Path;

use softladder_core::Project;

use crate::ProjectError;

pub use container::{
    parse_container, serialize_container, CONTAINER_END, CONTAINER_START, FILE_END_PREFIX,
    FILE_PREFIX,
};

/// Milestone that implements the element-level `.clprj` <-> [`Project`] mapping.
pub const MAPPING_MILESTONE: &str = "M3";

/// Converts parsed ClassicLadder container parts into a SoftLadder project.
///
/// # Errors
///
/// Always returns [`ProjectError::NotYetImplemented`] until M3.
pub fn import_container(_parts: &[(String, String)]) -> Result<Project, ProjectError> {
    // TODO(M3): decode `general.txt` (configuration), `rungs.txt` (elements),
    // `symbols.txt` and the remaining parameter files into a `Project`.
    Err(ProjectError::NotYetImplemented(MAPPING_MILESTONE))
}

/// Converts a SoftLadder project into ClassicLadder container parts.
///
/// # Errors
///
/// Always returns [`ProjectError::NotYetImplemented`] until M3.
pub fn export_container(_project: &Project) -> Result<Vec<(String, String)>, ProjectError> {
    // TODO(M3): write the parameter files back out in the order ClassicLadder
    // expects, so that a round trip through a real ClassicLadder build works.
    Err(ProjectError::NotYetImplemented(MAPPING_MILESTONE))
}

/// Reads a ClassicLadder container file and imports it as a project.
///
/// The container itself is parsed first, so a corrupt file is reported as
/// [`ProjectError::Container`] even while the mapping is still a stub.
pub fn import_file(path: &Path) -> Result<Project, ProjectError> {
    let bytes = std::fs::read(path)?;
    let parts = parse_container(&bytes)?;
    import_container(&parts)
}

/// Exports `project` to a ClassicLadder container file.
pub fn export_file(project: &Project, path: &Path) -> Result<(), ProjectError> {
    let parts = export_container(project)?;
    std::fs::write(path, serialize_container(&parts))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn import_and_export_report_the_milestone() {
        let import = import_container(&[]).expect_err("stub");
        assert!(matches!(
            import,
            ProjectError::NotYetImplemented(MAPPING_MILESTONE)
        ));
        let project = Project::new("stub");
        let export = export_container(&project).expect_err("stub");
        assert!(matches!(
            export,
            ProjectError::NotYetImplemented(MAPPING_MILESTONE)
        ));
        assert!(export.to_string().contains("M3"));
    }
}
