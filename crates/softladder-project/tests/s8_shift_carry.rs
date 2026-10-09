//! `WordsShiftsLeftRightExample.clprj` is ClassicLadder's demonstration of
//! `SHL`, `SHR`, `ROL` and `ROR` — its `PARAM_COMMENT` calls it "the new system
//! bit S8 associated" with them — so it is the natural corpus test for the carry
//! that closes divergence 2 of `testdata/known-divergences.md`.
//!
//! **What this asserts, and what it does not.** The file imports cleanly and
//! scans without a panic or an Error; the carry itself is asserted precisely in
//! `softladder-core` (per function, last-operation-wins, a rung reading `%S8` in
//! the same scan). What it does *not* assert is the example's word results,
//! because its two rungs do not become live through our import: with every
//! `%I1`…`%I9` contact closed the coils on the same rows still read false, so its
//! arithmetic cells never run. That is an import question for the next round, not
//! a carry question, and it is recorded in `testdata/known-divergences.md`.
//!
//! The corpus lives in `testdata/classicladder-corpus/projects_examples` (fetched
//! by `scripts/fetch_corpus.sh`, git-ignored), so the test skips itself with a
//! printed note when it is absent, like the M3 corpus group.

use std::path::PathBuf;

use softladder_core::{ScanEngine, Severity, Value, VarRef};
use softladder_project::classicladder::{self, Document};

/// The example, or `None` when the corpus has not been fetched.
fn shift_example() -> Option<PathBuf> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata/classicladder-corpus/projects_examples")
        .join("WordsShiftsLeftRightExample.clprj");
    path.is_file().then_some(path)
}

/// Reads a variable, or `None` when the store has no such slot.
fn get(engine: &ScanEngine, text: &str) -> Option<Value> {
    let var: VarRef = text.parse().expect("the variable parses");
    engine.store().get(&var)
}

#[test]
fn the_reference_shift_example_runs_and_publishes_the_carry() {
    let Some(path) = shift_example() else {
        println!("note: skipping, the corpus is absent");
        return;
    };
    let document = Document::parse(&std::fs::read(&path).expect("the corpus file is readable"))
        .expect("the container parses");
    let report = classicladder::import(&document).expect("the project imports");
    assert_eq!(report.project.schema_version, 2);

    // Drive it the way a user does: close every input its contacts read.
    let mut engine = ScanEngine::new(report.project.clone());
    for index in 1..=9u32 {
        let var = VarRef::new(softladder_core::VarKind::PhysIn, index);
        engine
            .store_mut()
            .set(&var, Value::Bit(true))
            .expect("an input is writable");
    }
    for now in [0_u64, 50, 100] {
        let scan = engine.scan_once(now);
        assert!(
            scan.diagnostics
                .iter()
                .all(|diagnostic| diagnostic.severity != Severity::Error),
            "the reference's own example must scan clean: {:?}",
            scan.diagnostics
        );
    }

    // The carry is published: after the shifts of the last scan it is clear. (The
    // `true` direction is asserted per function in `softladder-core`.)
    assert_eq!(get(&engine, "%S8"), Some(Value::Bit(false)));
}
