//! Diagnostics emitted by project loading, linting and the scan engine.
//!
//! Every diagnostic carries a stable machine readable `code` so that tooling
//! and tests never have to match on human readable prose. The codes currently
//! in use are:
//!
//! Errors and warnings are documented in `docs/SEMANTICS.md` §5 and
//! `docs/ELEMENTS.md`; `softladder-core::lint` and the scan engine emit:
//!
//! | code | severity | meaning |
//! |------|----------|---------|
//! | `SL-E001` | error | a variable is unknown or out of range in the store |
//! | `SL-E002` | error | expression parsing/evaluation failed (divide by zero, …) |
//! | `SL-E003` | error | an invalid block parameter or block variable |
//! | `SL-E004` | error | an element is missing its variable |
//! | `SL-E005` | error | a jump target (rung index or label) does not exist |
//! | `SL-E006` | error | the mad-loop jump guard tripped |
//! | `SL-E007` | error | a call to an undefined or non-subroutine section |
//! | `SL-E008` | error | the subroutine call stack overflowed |
//! | `SL-E009` | error | two elements are placed on the same cell |
//! | `SL-E011` | error | a section references a rung, step or page that does not exist |
//! | `SL-W001` | warning | a live row has no path to power (empty column 0, no vertical link), or an SFC step no transition can activate |
//! | `SL-W002` | warning | an SFC section cannot run: it has no page, or a transition names a step the page does not define |
//! | `SL-W011` | warning | an SFC transition has no condition, so it always fires |
//!
//! The CLI adds the project-level `SL-E010` (duplicate id), `SL-W010` (no rungs)
//! and `SL-W011` (empty rung); `softladder-edit` adds `SL-W020` for a
//! simulation-bench widget that addresses the wrong kind of variable; and the
//! ClassicLadder importer/exporter adds `SL-E030` (malformed document) and
//! `SL-W030`–`SL-W033` (skipped/approximated element, unmodelled variable
//! family or expression, passed-through part, non-exportable feature).

use std::fmt;

use serde::{Deserialize, Serialize};

/// Severity of a [`Diagnostic`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Severity {
    /// Purely informational.
    Info,
    /// Suspicious but recoverable.
    Warning,
    /// The scan result is not trustworthy.
    Error,
}

/// A single diagnostic message.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Diagnostic {
    /// Severity level.
    pub severity: Severity,
    /// Stable machine readable code, e.g. `SL-E002`.
    pub code: &'static str,
    /// Index of the section this diagnostic belongs to, if any.
    pub section: Option<usize>,
    /// Index of the rung this diagnostic belongs to, if any.
    pub rung: Option<usize>,
    /// Human readable message.
    pub message: String,
}

impl Diagnostic {
    /// Creates a diagnostic without section or rung context.
    pub fn new(severity: Severity, code: &'static str, message: String) -> Self {
        Self {
            severity,
            code,
            section: None,
            rung: None,
            message,
        }
    }

    /// Attaches a section index.
    pub fn with_section(mut self, section: usize) -> Self {
        self.section = Some(section);
        self
    }

    /// Attaches a rung index.
    pub fn with_rung(mut self, rung: usize) -> Self {
        self.rung = Some(rung);
        self
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let severity = match self.severity {
            Severity::Info => "info",
            Severity::Warning => "warning",
            Severity::Error => "error",
        };
        write!(f, "{severity}[{}]", self.code)?;
        if let Some(section) = self.section {
            write!(f, " section {section}")?;
        }
        if let Some(rung) = self.rung {
            write!(f, " rung {rung}")?;
        }
        write!(f, ": {}", self.message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_includes_context() {
        let diagnostic = Diagnostic::new(Severity::Error, "SL-E002", "division by zero".to_owned())
            .with_section(0)
            .with_rung(3);
        assert_eq!(
            diagnostic.to_string(),
            "error[SL-E002] section 0 rung 3: division by zero"
        );
    }

    #[test]
    fn severities_are_ordered() {
        assert!(Severity::Info < Severity::Warning);
        assert!(Severity::Warning < Severity::Error);
    }
}
