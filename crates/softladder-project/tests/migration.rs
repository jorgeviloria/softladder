//! Schema migration and round-trip tests for the native project format.
//!
//! The v1 fixture is hand-written (it must stay valid v1 forever), so these
//! tests pin the whole v1 → v2 chain: the migration itself, the snapshot of the
//! resulting project, idempotence and the rejection of documents from the
//! future or of documents that do not look like projects at all.

use std::io::Write;
use std::path::PathBuf;

use serde_json::Value;
use softladder_core::{Project, Symbol, VarRef};
use softladder_project::{native, ProjectError};

/// Path of a fixture inside this crate.
fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// Path of the shipped example project at the workspace root.
fn example() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/traffic_light.slprj")
        .canonicalize()
        .expect("examples/traffic_light.slprj exists")
}

/// The raw v1 fixture, used to drive the loader from text.
fn v1_text() -> String {
    std::fs::read_to_string(fixture("v1_before.slprj")).expect("fixture is readable")
}

/// Builds a minimal v1 document around a raw `rungs` and `symbols` value.
fn v1_document(rungs: &str, symbols: &str) -> String {
    format!(
        "{{ \"schema_version\": 1, \"name\": \"hostile\", \"author\": \"\", \"comment\": \"\", \
         \"sections\": [], \"rungs\": {rungs}, \"symbols\": {symbols}, \
         \"scan\": {{ \"period_ms\": 10, \"input_period_ms\": 10 }} }}"
    )
}

/// Builds a v1 document with one rung holding the given raw `elements` value.
fn v1_with_elements(elements: &str) -> String {
    v1_document(
        &format!(
            "[ {{ \"id\": 1, \"label\": \"\", \"comment\": \"\", \"elements\": {elements} }} ]"
        ),
        "[]",
    )
}

/// `true` when `key` appears anywhere in `value`, at any depth.
fn contains_key(value: &Value, key: &str) -> bool {
    match value {
        Value::Object(object) => {
            object.contains_key(key) || object.values().any(|child| contains_key(child, key))
        }
        Value::Array(items) => items.iter().any(|child| contains_key(child, key)),
        _ => false,
    }
}

#[test]
fn v1_fixture_migrates_to_v2() {
    let project = native::load(&fixture("v1_before.slprj")).expect("the v1 fixture loads");
    assert_eq!(project.schema_version, native::CURRENT_SCHEMA_VERSION);

    let value = serde_json::to_value(&project).expect("the project serializes");
    let elements = &value["rungs"][0]["elements"];

    // `bit: 3` becomes the `{"Bit": 3}` accessor.
    assert_eq!(elements[0]["var"]["kind"], "MemWord");
    assert_eq!(
        elements[0]["var"]["accessor"],
        serde_json::json!({ "Bit": 3 })
    );
    assert!(elements[0]["var"].get("bit").is_none());

    // `TimerIecValue` folds into `TimerIec` plus the `Value` accessor.
    assert_eq!(elements[1]["var"]["kind"], "TimerIec");
    assert_eq!(elements[1]["var"]["accessor"], "Value");

    // `CounterValue` folds into `Counter` plus the `Value` accessor.
    assert_eq!(elements[2]["var"]["kind"], "Counter");
    assert_eq!(elements[2]["var"]["accessor"], "Value");

    // Nested `index_expr` chains are walked recursively.
    let nested = &elements[3]["var"];
    assert_eq!(nested["accessor"], Value::Null);
    assert_eq!(nested["index_expr"]["accessor"], Value::Null);
    assert_eq!(
        nested["index_expr"]["index_expr"]["accessor"],
        serde_json::json!({ "Bit": 4 })
    );

    // Symbols are walked too, and a symbol without `var` stays that way.
    assert_eq!(
        value["symbols"][0]["var"],
        Value::Null,
        "an absent symbol variable must not be invented"
    );
    assert_eq!(
        value["symbols"][1]["var"]["accessor"],
        serde_json::json!({ "Bit": 7 })
    );
    assert_eq!(project.symbols[0].var, None);

    // No v1 key survives anywhere in the document.
    assert!(!contains_key(&value, "bit"));

    insta::assert_json_snapshot!("v1_fixture_migrated_to_v2", value);
}

#[test]
fn migrating_the_v1_fixture_is_idempotent() {
    let loaded = native::load(&fixture("v1_before.slprj")).expect("the v1 fixture loads");
    let first = native::to_json(&loaded).expect("serializes");
    let again = native::from_json(&first).expect("the migrated text reloads");
    let second = native::to_json(&again).expect("serializes");

    assert_eq!(first, second, "a migrated document must be a fixed point");
    assert_eq!(loaded, again);
    assert!(first.contains("\"schema_version\": 2"));
}

#[test]
fn save_load_save_is_byte_identical() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let first_path = directory.path().join("first.slprj");
    let second_path = directory.path().join("second.slprj");

    let project = native::load(&fixture("v1_before.slprj")).expect("the v1 fixture loads");
    native::save(&project, &first_path).expect("saves");
    let reloaded = native::load(&first_path).expect("reloads");
    native::save(&reloaded, &second_path).expect("saves again");

    let first = std::fs::read(&first_path).expect("reads");
    let second = std::fs::read(&second_path).expect("reads");
    assert_eq!(first, second, "saving must not drift the schema");
    assert_eq!(reloaded, project);
}

#[test]
fn gzip_documents_are_migrated_on_load() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("v1.slprjz");

    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(v1_text().as_bytes()).expect("compresses");
    let bytes = encoder.finish().expect("finishes the stream");
    assert!(native::is_gzip(&bytes));
    std::fs::write(&path, bytes).expect("writes the gz document");

    let project = native::load(&path).expect("a gzipped v1 document loads");
    assert_eq!(project.schema_version, native::CURRENT_SCHEMA_VERSION);
    assert_eq!(native::load_gz(&path).expect("loads gz"), project);
}

#[test]
fn the_shipped_example_uses_the_current_schema_and_loads() {
    let raw = std::fs::read_to_string(example()).expect("the example is readable");
    let document: Value = serde_json::from_str(&raw).expect("the example is valid json");
    assert_eq!(
        document["schema_version"],
        native::CURRENT_SCHEMA_VERSION,
        "examples/traffic_light.slprj is kept at the current schema version"
    );
    assert!(
        !contains_key(&document, "bit"),
        "the v2 example spells sub-values with `accessor`"
    );
    // The v1 input path is covered by `tests/fixtures/v1_before.slprj`.

    let project = native::load(&example()).expect("the example loads");
    assert_eq!(project.schema_version, native::CURRENT_SCHEMA_VERSION);
    assert_eq!(project.sections.len(), 1);
    assert_eq!(project.rungs.len(), 4);
    assert_eq!(project.symbols.len(), 6);
    assert!(
        project.symbols.iter().all(|symbol| symbol.var.is_some()),
        "every example symbol is bound to a variable"
    );
    assert!(
        project.rungs.iter().any(|rung| rung
            .elements
            .iter()
            .any(|element| element.connected_with_top)),
        "the self-holding rung uses a vertical link to merge its branches"
    );
}

#[test]
fn future_schema_versions_are_rejected() {
    let text = "{ \"schema_version\": 99, \"name\": \"from the future\" }";
    let error = native::from_json(text).expect_err("must fail");
    assert!(matches!(error, ProjectError::UnsupportedSchema(99)));

    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("future.slprj");
    std::fs::write(&path, text).expect("writes");
    let error = native::load(&path).expect_err("must fail");
    assert!(matches!(error, ProjectError::UnsupportedSchema(99)));
    assert!(error.to_string().contains("unsupported schema version 99"));
}

#[test]
fn documents_that_are_not_objects_are_rejected() {
    for text in ["[1, 2, 3]", "\"a string\"", "42", "null"] {
        let error = native::from_json(text).expect_err("must fail");
        assert!(
            matches!(error, ProjectError::MalformedDocument(_)),
            "{text} must be malformed, got {error:?}"
        );
    }
}

#[test]
fn hostile_v1_bits_are_rejected_without_panicking() {
    for bit in ["\"three\"", "-1", "1.5", "[]", "{}"] {
        let text = v1_with_elements(&format!(
            "[ {{ \"kind\": \"ContactNo\", \"var\": {{ \"kind\": \"MemWord\", \"index\": 0, \"bit\": {bit} }}, \
             \"col\": 0, \"row\": 0, \"params\": [] }} ]"
        ));
        let error = native::from_json(&text).expect_err("must fail");
        assert!(
            matches!(error, ProjectError::MalformedDocument(_)),
            "bit {bit} must be malformed, got {error:?}"
        );
    }
}

#[test]
fn hostile_v1_shapes_are_ignored_and_left_to_serde() {
    // A `var` that is not an object has no sensible rewrite: the migration
    // leaves it alone and deserialization reports the shape error.
    let text = v1_with_elements(
        "[ { \"kind\": \"ContactNo\", \"var\": \"%MW0.3\", \"col\": 0, \"row\": 0, \"params\": [] } ]",
    );
    let error = native::from_json(&text).expect_err("must fail");
    assert!(matches!(error, ProjectError::Json(_)), "got {error:?}");

    // `elements` that is not an array is skipped by the migration, then
    // rejected by serde.
    let text = v1_document(
        "[ { \"id\": 1, \"label\": \"\", \"comment\": \"\", \"elements\": \"nope\" } ]",
        "[]",
    );
    let error = native::from_json(&text).expect_err("must fail");
    assert!(matches!(error, ProjectError::Json(_)), "got {error:?}");

    // `rungs` that is not an array is skipped too.
    let text = v1_document("\"nope\"", "[]");
    let error = native::from_json(&text).expect_err("must fail");
    assert!(matches!(error, ProjectError::Json(_)), "got {error:?}");

    // A symbol whose `var` is a string is skipped as well.
    let text = v1_document(
        "[]",
        "[ { \"name\": \"bad\", \"var\": 7, \"comment\": \"\", \"unit\": null } ]",
    );
    let error = native::from_json(&text).expect_err("must fail");
    assert!(matches!(error, ProjectError::Json(_)), "got {error:?}");
}

#[test]
fn symbols_round_trip_with_and_without_a_variable() {
    let mut project = Project::new("symbols");
    project.symbols.push(Symbol {
        name: "bound".to_owned(),
        var: Some("%MW3.7".parse::<VarRef>().expect("the variable parses")),
        ..Symbol::default()
    });
    project.symbols.push(Symbol {
        name: "unbound".to_owned(),
        var: None,
        ..Symbol::default()
    });

    let text = native::to_json(&project).expect("serializes");
    let reloaded = native::from_json(&text).expect("reloads");
    assert_eq!(reloaded.symbols, project.symbols);
    assert_eq!(reloaded, project);

    // Both spellings are accepted; only the absent one relies on the serde
    // default for `var`.
    let value: Value = serde_json::from_str(&text).expect("valid json");
    assert!(value["symbols"][0]["var"].is_object());
    assert!(value["symbols"][1]["var"].is_null());
}
