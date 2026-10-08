//! Native SoftLadder project files: deterministic pretty-printed JSON.
//!
//! Writing a project always produces the same bytes for the same project —
//! `serde_json::to_string_pretty` plus a trailing newline — which keeps
//! projects diff-friendly and makes golden-file tests possible.
//!
//! A project is stored either uncompressed (`.slprj`) or as a gzip stream
//! (`.slprjz`). [`load`] sniffs the gzip magic number instead of trusting the
//! file extension, so a renamed file still loads.

use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use softladder_core::Project;

use crate::ProjectError;

/// Magic number that starts every gzip stream.
pub const GZIP_MAGIC: [u8; 2] = [0x1f, 0x8b];

/// Serializes `project` to deterministic pretty JSON with a trailing newline.
pub fn to_json(project: &Project) -> Result<String, ProjectError> {
    let mut text = serde_json::to_string_pretty(project)?;
    text.push('\n');
    Ok(text)
}

/// Parses a project from JSON text.
pub fn from_json(text: &str) -> Result<Project, ProjectError> {
    let project: Project = serde_json::from_str(text)?;
    Ok(project)
}

/// `true` when `bytes` starts with the gzip magic number.
pub fn is_gzip(bytes: &[u8]) -> bool {
    bytes.starts_with(&GZIP_MAGIC)
}

/// Saves `project` to `path` as uncompressed JSON.
pub fn save(project: &Project, path: &Path) -> Result<(), ProjectError> {
    let text = to_json(project)?;
    std::fs::write(path, text)?;
    Ok(())
}

/// Saves `project` to `path` as a gzip stream (`.slprjz`).
pub fn save_gz(project: &Project, path: &Path) -> Result<(), ProjectError> {
    let text = to_json(project)?;
    let file = File::create(path)?;
    let mut encoder = GzEncoder::new(file, Compression::default());
    encoder.write_all(text.as_bytes())?;
    encoder.finish()?;
    Ok(())
}

/// Loads a project, transparently decompressing gzip streams.
pub fn load(path: &Path) -> Result<Project, ProjectError> {
    let bytes = std::fs::read(path)?;
    if is_gzip(&bytes) {
        return load_gz(path);
    }
    let text = String::from_utf8(bytes).map_err(|error| {
        ProjectError::Container(format!("project file is not valid UTF-8: {error}"))
    })?;
    from_json(&text)
}

/// Loads a gzip-compressed project.
pub fn load_gz(path: &Path) -> Result<Project, ProjectError> {
    let bytes = std::fs::read(path)?;
    let mut decoder = GzDecoder::new(bytes.as_slice());
    let mut text = String::new();
    decoder.read_to_string(&mut text)?;
    from_json(&text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use softladder_core::{Rung, Section};

    fn sample_project() -> Project {
        let mut project = Project::new("round trip");
        project.author = "tests".to_owned();
        let mut section = Section::new(1, "Main");
        section.rungs.push(1);
        project.sections.push(section);
        project.rungs.push(Rung::new(1));
        project
    }

    #[test]
    fn json_is_deterministic_and_newline_terminated() {
        let project = sample_project();
        let first = to_json(&project).expect("serializes");
        let second = to_json(&project).expect("serializes");
        assert_eq!(first, second);
        assert!(first.ends_with("}\n"));
        assert_eq!(from_json(&first).expect("deserializes"), project);
    }

    #[test]
    fn round_trip_through_a_file() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("project.slprj");
        let project = sample_project();
        save(&project, &path).expect("saves");
        let loaded = load(&path).expect("loads");
        assert_eq!(loaded, project);
        let bytes = std::fs::read(&path).expect("reads back");
        assert!(!is_gzip(&bytes));
    }

    #[test]
    fn gzip_round_trip_is_detected_by_magic_bytes() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("project.slprjz");
        let project = sample_project();
        save_gz(&project, &path).expect("saves gz");
        let bytes = std::fs::read(&path).expect("reads back");
        assert!(is_gzip(&bytes));
        assert_eq!(load_gz(&path).expect("loads gz"), project);
        // `load` sniffs the magic number, not the extension.
        assert_eq!(load(&path).expect("loads"), project);
    }

    #[test]
    fn broken_json_is_reported() {
        let error = from_json("{ not json }").expect_err("must fail");
        assert!(matches!(error, ProjectError::Json(_)));
    }

    #[test]
    fn missing_files_are_reported() {
        let error = load(Path::new("/definitely/not/here.slprj")).expect_err("must fail");
        assert!(matches!(error, ProjectError::Io(_)));
    }
}
