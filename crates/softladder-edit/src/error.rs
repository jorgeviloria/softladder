//! Errors produced while editing a project.

use softladder_project::ProjectError;
use thiserror::Error;

/// Everything [`crate::Editor`] and [`crate::Command`] can reject.
///
/// A rejected command never changes the project and never records a history
/// entry, so a caller may safely ignore an `Err` and continue.
#[derive(Debug, Error)]
pub enum EditError {
    /// No rung carries the requested id.
    #[error("unknown rung `{0}`")]
    UnknownRung(u32),
    /// No section carries the requested id.
    #[error("unknown section `{0}`")]
    UnknownSection(u32),
    /// The target cell already holds an element, so nothing can go there.
    #[error("cell (col {col}, row {row}) of rung {rung} is already occupied")]
    Occupied {
        /// Rung the cell belongs to.
        rung: u32,
        /// Column of the cell.
        col: u8,
        /// Row of the cell.
        row: u8,
    },
    /// The source cell holds no element, so there is nothing to edit, move or
    /// delete there.
    #[error("cell (col {col}, row {row}) of rung {rung} does not exist")]
    NoSuchCell {
        /// Rung the cell belongs to.
        rung: u32,
        /// Column of the cell.
        col: u8,
        /// Row of the cell.
        row: u8,
    },
    /// A position index is past the end of the collection it addresses.
    #[error("index {index} is out of range (length {len})")]
    IndexOutOfRange {
        /// Requested position.
        index: usize,
        /// Current length of the addressed collection.
        len: usize,
    },
    /// Another rung already uses this id.
    #[error("rung id `{0}` is already in use")]
    DuplicateRung(u32),
    /// Another section already uses this id.
    #[error("section id `{0}` is already in use")]
    DuplicateSection(u32),
    /// There is no file to save to yet; an explicit path is required first.
    #[error("the project has no file path: save it with an explicit path first")]
    NoPath,
    /// Loading, migrating or saving the project failed.
    #[error(transparent)]
    Project(#[from] ProjectError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_render_their_context() {
        assert_eq!(EditError::UnknownRung(7).to_string(), "unknown rung `7`");
        assert_eq!(
            EditError::UnknownSection(2).to_string(),
            "unknown section `2`"
        );
        assert_eq!(
            EditError::Occupied {
                rung: 1,
                col: 3,
                row: 0
            }
            .to_string(),
            "cell (col 3, row 0) of rung 1 is already occupied"
        );
        assert_eq!(
            EditError::NoSuchCell {
                rung: 1,
                col: 0,
                row: 1
            }
            .to_string(),
            "cell (col 0, row 1) of rung 1 does not exist"
        );
        assert_eq!(
            EditError::IndexOutOfRange { index: 9, len: 2 }.to_string(),
            "index 9 is out of range (length 2)"
        );
        assert_eq!(
            EditError::DuplicateRung(1).to_string(),
            "rung id `1` is already in use"
        );
        assert_eq!(
            EditError::DuplicateSection(1).to_string(),
            "section id `1` is already in use"
        );
        assert!(EditError::NoPath.to_string().contains("no file path"));
    }

    #[test]
    fn project_errors_are_wrapped_without_losing_the_message() {
        let error: EditError = ProjectError::UnsupportedSchema(99).into();
        assert!(error.to_string().contains("unsupported schema version 99"));
        assert!(matches!(error, EditError::Project(_)));
    }
}
