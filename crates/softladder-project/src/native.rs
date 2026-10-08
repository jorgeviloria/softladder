//! Native SoftLadder project files: deterministic pretty-printed JSON.
//!
//! Writing a project always produces the same bytes for the same project —
//! `serde_json::to_string_pretty` plus a trailing newline — which keeps
//! projects diff-friendly and makes golden-file tests possible.
//!
//! A project is stored either uncompressed (`.slprj`) or as a gzip stream
//! (`.slprjz`). [`load`] sniffs the gzip magic number instead of trusting the
//! file extension, so a renamed file still loads.
//!
//! # Schema versions
//!
//! Reading goes through a migration chain *before* the document is deserialized,
//! so an older file never has to satisfy the current Rust types. The chain is
//! driven by the top-level `schema_version` field (missing means version 1) and
//! steps from there up to [`CURRENT_SCHEMA_VERSION`]:
//!
//! * [`MIGRATIONS`] holds one function per step; `MIGRATIONS[n - 1]` upgrades a
//!   version `n` document to version `n + 1`.
//! * A document from the future is rejected with
//!   [`ProjectError::UnsupportedSchema`] instead of being reinterpreted.
//! * A document that is not a JSON object is rejected with
//!   [`ProjectError::MalformedDocument`].
//!
//! Saving always emits `schema_version = CURRENT_SCHEMA_VERSION`, so a load /
//! save cycle cannot leave an old version behind.

use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use serde_json::{json, Value};
use softladder_core::Project;

use crate::ProjectError;

/// Magic number that starts every gzip stream.
pub const GZIP_MAGIC: [u8; 2] = [0x1f, 0x8b];

/// Schema version this build reads and writes.
///
/// Loading a document tagged with an older version migrates it up to this
/// version before deserializing; saving always emits this version.
pub const CURRENT_SCHEMA_VERSION: u32 = 2;

/// Signature of a single migration step.
///
/// A migration receives the raw JSON document and returns the document for the
/// next schema version. It must not panic, whatever the input looks like.
pub type Migration = fn(Value) -> Result<Value, ProjectError>;

/// Ordered migration chain: `MIGRATIONS[n - 1]` upgrades a version `n` document
/// to version `n + 1`.
///
/// The chain, not the loader, is the list of steps that exist: adding a schema
/// version means adding one function here and bumping
/// [`CURRENT_SCHEMA_VERSION`].
pub const MIGRATIONS: &[Migration] = &[migrate_v1_to_v2];

/// Serializes `project` to deterministic pretty JSON with a trailing newline.
///
/// The emitted document always carries `schema_version =
/// CURRENT_SCHEMA_VERSION`, whatever version the in-memory project was tagged
/// with, so saving never writes a stale version back to disk.
pub fn to_json(project: &Project) -> Result<String, ProjectError> {
    let mut project = project.clone();
    project.schema_version = CURRENT_SCHEMA_VERSION;
    let mut text = serde_json::to_string_pretty(&project)?;
    text.push('\n');
    Ok(text)
}

/// Parses a project from JSON text, migrating older documents first.
pub fn from_json(text: &str) -> Result<Project, ProjectError> {
    let document: Value = serde_json::from_str(text)?;
    let document = migrate(document)?;
    let project: Project = serde_json::from_value(document)?;
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

// -- migrations ---------------------------------------------------------------

/// Applies every migration from the document's `schema_version` up to
/// [`CURRENT_SCHEMA_VERSION`].
///
/// A missing or `null` `schema_version` means version 1. A version greater than
/// [`CURRENT_SCHEMA_VERSION`] is rejected rather than reinterpreted, and the
/// final document always carries the current version.
fn migrate(mut document: Value) -> Result<Value, ProjectError> {
    let mut version = document_version(&document)?;

    if version > CURRENT_SCHEMA_VERSION {
        return Err(ProjectError::UnsupportedSchema(version));
    }

    while version < CURRENT_SCHEMA_VERSION {
        let step = MIGRATIONS
            .get((version - 1) as usize)
            .copied()
            .ok_or(ProjectError::UnsupportedSchema(version))?;
        document = step(document)?;
        version += 1;
    }

    match document.as_object_mut() {
        Some(object) => {
            object.insert("schema_version".to_owned(), json!(CURRENT_SCHEMA_VERSION));
            Ok(document)
        }
        None => Err(ProjectError::MalformedDocument(
            "the document must be a JSON object".to_owned(),
        )),
    }
}

/// Reads `schema_version` from a raw document without deserializing it.
///
/// Missing and `null` both mean version 1, the oldest version of this format.
/// Anything that is not a positive integer is a malformed document, not a
/// version to guess at.
fn document_version(document: &Value) -> Result<u32, ProjectError> {
    let object = document.as_object().ok_or_else(|| {
        ProjectError::MalformedDocument("the document must be a JSON object".to_owned())
    })?;

    match object.get("schema_version") {
        None | Some(Value::Null) => Ok(1),
        Some(Value::Number(number)) => number
            .as_u64()
            .and_then(|version| u32::try_from(version).ok())
            .filter(|version| *version >= 1)
            .ok_or_else(|| {
                ProjectError::MalformedDocument(format!(
                    "schema_version must be a positive integer, found {number}"
                ))
            }),
        Some(other) => Err(ProjectError::MalformedDocument(format!(
            "schema_version must be an integer, found {other}"
        ))),
    }
}

/// v1 → v2: `bit` becomes `accessor`, and the `.V` variable kinds fold into
/// their base kind plus the `"Value"` accessor.
///
/// Every [`softladder_core::VarRef`] in the document is rewritten: element
/// variables, their nested `index_expr` chain and symbol variables. Contradictory
/// input (a `TimerIecValue` that also selects a bit) resolves in favour of the
/// `kind` rewrite, because that is the field stating which sub-value the
/// variable denotes.
fn migrate_v1_to_v2(mut document: Value) -> Result<Value, ProjectError> {
    let object = document.as_object_mut().ok_or_else(|| {
        ProjectError::MalformedDocument("the document must be a JSON object".to_owned())
    })?;

    if let Some(rungs) = object.get_mut("rungs").and_then(Value::as_array_mut) {
        for rung in rungs {
            let Some(elements) = rung.get_mut("elements").and_then(Value::as_array_mut) else {
                continue;
            };
            for element in elements {
                if let Some(var) = element.get_mut("var") {
                    migrate_var_ref(var)?;
                }
            }
        }
    }

    if let Some(symbols) = object.get_mut("symbols").and_then(Value::as_array_mut) {
        for symbol in symbols {
            if let Some(var) = symbol.get_mut("var") {
                migrate_var_ref(var)?;
            }
        }
    }

    object.insert("schema_version".to_owned(), json!(2));
    Ok(document)
}

/// Rewrites one v1 `VarRef` in place, recursively through `index_expr`.
///
/// A `var` that is not a JSON object is left untouched: there is no sensible
/// rewrite, and deserialization reports the shape error with a span-like
/// message. A `bit` that is neither `null` nor an integer in `0..=255` is
/// rejected, because silently dropping it would change what the variable means.
fn migrate_var_ref(var: &mut Value) -> Result<(), ProjectError> {
    let Some(object) = var.as_object_mut() else {
        return Ok(());
    };

    if let Some(index_expr) = object.get_mut("index_expr") {
        migrate_var_ref(index_expr)?;
    }

    if let Some(bit) = object.remove("bit") {
        let accessor = match bit {
            Value::Null => Value::Null,
            Value::Number(number) => match number.as_u64().and_then(|bit| u8::try_from(bit).ok()) {
                Some(bit) => json!({ "Bit": bit }),
                None => {
                    return Err(ProjectError::MalformedDocument(format!(
                        "variable `bit` must be null or an integer in 0..=255, found {number}"
                    )));
                }
            },
            other => {
                return Err(ProjectError::MalformedDocument(format!(
                    "variable `bit` must be null or an integer, found {other}"
                )));
            }
        };
        object.insert("accessor".to_owned(), accessor);
    }

    let folded_kind = match object.get("kind").and_then(Value::as_str) {
        Some("TimerIecValue") => Some("TimerIec"),
        Some("CounterValue") => Some("Counter"),
        _ => None,
    };
    if let Some(base) = folded_kind {
        object.insert("kind".to_owned(), Value::String(base.to_owned()));
        object.insert("accessor".to_owned(), Value::String("Value".to_owned()));
    }

    Ok(())
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
    fn saved_json_carries_the_current_schema_version() {
        let mut project = sample_project();
        project.schema_version = 1;
        let text = to_json(&project).expect("serializes");
        let document: Value = serde_json::from_str(&text).expect("valid json");
        assert_eq!(document["schema_version"], json!(CURRENT_SCHEMA_VERSION));
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

    #[test]
    fn missing_schema_version_is_read_as_version_one() {
        let mut document = json!({ "name": "old" });
        assert_eq!(document_version(&document).expect("version"), 1);
        document["schema_version"] = Value::Null;
        assert_eq!(document_version(&document).expect("version"), 1);
    }

    #[test]
    fn non_object_documents_are_rejected() {
        for document in [json!([1, 2, 3]), json!("text"), json!(42), Value::Null] {
            let error = migrate(document).expect_err("must fail");
            assert!(matches!(error, ProjectError::MalformedDocument(_)));
        }
    }

    #[test]
    fn bogus_schema_versions_are_rejected() {
        for version in [json!("1"), json!(1.5), json!(-1), json!(0), json!(true)] {
            let document = json!({ "schema_version": version });
            let error = migrate(document).expect_err("must fail");
            assert!(
                matches!(error, ProjectError::MalformedDocument(_)),
                "{version} must be malformed, got {error:?}"
            );
        }
    }

    #[test]
    fn future_schema_versions_are_rejected() {
        let document = json!({ "schema_version": 99 });
        let error = migrate(document).expect_err("must fail");
        assert!(matches!(error, ProjectError::UnsupportedSchema(99)));
    }

    #[test]
    fn bit_conversion_survives_hostile_shapes() {
        // `bit` on a non-object `var` is ignored, not a panic.
        let mut var = json!("not an object");
        migrate_var_ref(&mut var).expect("ignored");
        assert_eq!(var, json!("not an object"));

        // A numeric bit outside 0..=255 is rejected.
        let mut var = json!({ "kind": "MemWord", "index": 0, "bit": 300 });
        assert!(matches!(
            migrate_var_ref(&mut var).expect_err("must fail"),
            ProjectError::MalformedDocument(_)
        ));

        // A non-numeric bit is rejected.
        let mut var = json!({ "kind": "MemWord", "index": 0, "bit": "3" });
        assert!(matches!(
            migrate_var_ref(&mut var).expect_err("must fail"),
            ProjectError::MalformedDocument(_)
        ));

        // Null becomes an explicit null accessor and `bit` disappears.
        let mut var = json!({ "kind": "MemWord", "index": 0, "bit": null });
        migrate_var_ref(&mut var).expect("converts");
        assert_eq!(
            var,
            json!({ "kind": "MemWord", "index": 0, "accessor": null })
        );
    }

    #[test]
    fn non_array_collections_are_ignored() {
        let document = json!({
            "schema_version": 1,
            "rungs": "not an array",
            "symbols": { "not": "an array" }
        });
        let migrated = migrate(document).expect("ignored");
        assert_eq!(migrated["schema_version"], json!(2));
    }

    #[test]
    fn nested_index_expr_is_migrated_recursively() {
        let mut document = json!({
            "schema_version": 1,
            "rungs": [{
                "elements": [{
                    "var": {
                        "kind": "MemWord",
                        "index": 0,
                        "index_expr": {
                            "kind": "MemWord",
                            "index": 0,
                            "bit": 4,
                            "index_expr": { "kind": "MemWord", "index": 0, "bit": 1 }
                        },
                        "bit": null
                    }
                }]
            }]
        });
        document = migrate(document).expect("migrates");
        let outer = &document["rungs"][0]["elements"][0]["var"];
        assert_eq!(outer["accessor"], Value::Null);
        assert_eq!(outer["index_expr"]["accessor"], json!({ "Bit": 4 }));
        assert_eq!(
            outer["index_expr"]["index_expr"]["accessor"],
            json!({ "Bit": 1 })
        );
        assert!(outer["index_expr"].get("bit").is_none());
    }

    #[test]
    fn kind_folding_wins_over_a_stale_bit() {
        let mut document = json!({
            "schema_version": 1,
            "rungs": [{
                "elements": [
                    { "var": { "kind": "TimerIecValue", "index": 0, "bit": 3 } },
                    { "var": { "kind": "CounterValue", "index": 1, "bit": null } }
                ]
            }]
        });
        document = migrate(document).expect("migrates");
        let timer = &document["rungs"][0]["elements"][0]["var"];
        let counter = &document["rungs"][0]["elements"][1]["var"];
        assert_eq!(timer["kind"], json!("TimerIec"));
        assert_eq!(timer["accessor"], json!("Value"));
        assert_eq!(counter["kind"], json!("Counter"));
        assert_eq!(counter["accessor"], json!("Value"));
        assert!(timer.get("bit").is_none());
    }

    #[test]
    fn the_chain_covers_every_step_up_to_the_current_version() {
        assert_eq!(MIGRATIONS.len() as u32, CURRENT_SCHEMA_VERSION - 1);
        assert_eq!(MIGRATIONS.len(), 1);
    }
}
