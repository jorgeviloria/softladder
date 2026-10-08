//! Guards the shipped example project: it must load with the native loader and
//! describe the section/rung shape the README promises.

use std::path::PathBuf;

use softladder_core::{SectionLanguage, Value, VarRef};
use softladder_project::native;

fn example_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/traffic_light.slprj")
        .canonicalize()
        .expect("examples/traffic_light.slprj exists")
}

#[test]
fn traffic_light_example_loads() {
    let project = native::load(&example_path()).expect("example project loads");

    assert_eq!(project.schema_version, native::CURRENT_SCHEMA_VERSION);
    assert_eq!(project.name, "traffic_light");
    assert_eq!(project.sections.len(), 1);
    assert_eq!(project.rungs.len(), 4, "the example has four rungs");

    let section = &project.sections[0];
    assert_eq!(section.name, "Main");
    assert_eq!(section.language, SectionLanguage::Ladder);
    assert_eq!(section.rungs.len(), project.rungs.len());

    for rung_id in &section.rungs {
        assert!(
            project.rung(*rung_id).is_some(),
            "section references missing rung {rung_id}"
        );
    }

    // The example must still be a fixed point of the loader.
    let text = native::to_json(&project).expect("re-serializes");
    assert_eq!(native::from_json(&text).expect("re-parses"), project);
}

#[test]
fn traffic_light_example_is_runnable() {
    let project = native::load(&example_path()).expect("example project loads");
    let mut engine = softladder_core::ScanEngine::new(project);

    let input = |text: &str| text.parse::<VarRef>().expect("variable parses");
    engine
        .store_mut()
        .set(&input("%I0"), Value::Bit(true))
        .expect("start button can be pressed");

    // Press start, then run the equivalent of four seconds of scans.
    let mut last = None;
    for cycle in 0..400u64 {
        last = Some(engine.scan_once(cycle * 10));
    }

    let report = last.expect("at least one scan ran");
    assert!(
        report
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.severity != softladder_core::Severity::Error),
        "the example must scan without errors: {:?}",
        report.diagnostics
    );
    assert_eq!(engine.store().get(&input("%Q0")), Some(Value::Bit(true)));
    assert_eq!(engine.store().get(&input("%Q1")), Some(Value::Bit(true)));
}
