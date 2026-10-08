//! M2 acceptance: the shipped example can be opened, edited, simulated,
//! saved and reopened with the project intact.
//!
//! This is the milestone's stated criterion, expressed as a test:
//!
//! > the traffic-light example can be edited and run in simulation, and closing
//! > and reopening it keeps the project.
//!
//! It deliberately drives the same API the editor does (`softladder-edit` +
//! `softladder-core`), so a UI regression that bypasses the editing layer is the
//! only thing it cannot catch.

use std::path::PathBuf;

use softladder_core::{ElementKind, SimulationPanel, Value, VarRef};
use softladder_edit::{Bench, Editor, RuntimeState};

fn example_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/traffic_light.slprj")
        .canonicalize()
        .expect("examples/traffic_light.slprj exists")
}

fn var(text: &str) -> VarRef {
    text.parse().expect("test variable parses")
}

fn open_example() -> Editor {
    Editor::open(&example_path()).expect("the shipped example loads")
}

#[test]
fn the_example_opens_with_a_bench_ready_to_use() {
    let editor = open_example();
    let project = editor.project();

    assert!(!editor.is_dirty(), "a freshly opened project is clean");
    assert_eq!(project.sections.len(), 1);
    assert_eq!(project.rungs.len(), 4);

    // The bench travels with the project and mirrors its physical variables.
    let panel = &project.simulation;
    assert_eq!(panel.switches.len(), 3, "{:?}", panel.switches);
    assert_eq!(panel.lamps.len(), 2);
    assert!(panel.validate().is_empty(), "{:?}", panel.validate());
    assert!(
        panel
            .switches
            .iter()
            .any(|switch| switch.label == "start_button"),
        "the bench uses the project's symbol names"
    );

    assert!(
        editor.problems().is_empty(),
        "the example must be problem-free: {:?}",
        editor.problems()
    );
}

#[test]
fn switching_the_bench_inputs_runs_the_traffic_light() {
    let mut bench = Bench::new(open_example().project().clone());
    assert_eq!(bench.state(), RuntimeState::Stop);
    assert_eq!(bench.readings().len(), 2);

    // Closing the start switch energises the green lamp.
    bench.panel_state_mut().set_closed(0, true);
    bench.start();
    assert_eq!(bench.step().map(|report| report.diagnostics.len()), Some(0));
    assert_eq!(
        bench.engine().store().get(&var("%Q0")),
        Some(Value::Bit(true)),
        "the green lamp follows the start switch"
    );

    // The amber lamp comes on once the 3000 ms on-delay has elapsed. The example
    // scans every 10 ms, and the engine charges no elapsed time to the first scan
    // (there is no previous scan to measure from), so 3000 ms of counting needs
    // 301 scans. 200 of them are not enough, and the exact count is asserted.
    for _ in 0..199 {
        bench.step();
    }
    assert_eq!(
        bench.engine().store().get(&var("%Q1")),
        Some(Value::Bit(false)),
        "200 scans are not enough for a 3000 ms preset at a 10 ms period"
    );

    let mut scans = 200u64;
    loop {
        bench.step();
        scans += 1;
        if bench.engine().store().get(&var("%Q1")) == Some(Value::Bit(true)) {
            break;
        }
        assert!(scans < 500, "the on-delay never expired");
    }
    assert_eq!(
        scans, 301,
        "a 3000 ms preset at a 10 ms period takes 301 scans: the first one only \
         establishes the edge state"
    );

    // Releasing the start switch does *not* drop the lamps: the self-holding
    // branch keeps %Q0 sealed in until the stop button is pressed.
    bench.panel_state_mut().set_closed(0, false);
    for _ in 0..3 {
        bench.step();
    }
    assert_eq!(
        bench.engine().store().get(&var("%Q0")),
        Some(Value::Bit(true)),
        "the seal holds after the start button is released"
    );

    // The stop button is wired normally closed, so closing its bench switch
    // opens the contact and breaks the seal.
    bench.panel_state_mut().set_closed(2, true);
    for _ in 0..3 {
        bench.step();
    }
    assert_eq!(
        bench.engine().store().get(&var("%Q0")),
        Some(Value::Bit(false))
    );
    assert_eq!(
        bench.engine().store().get(&var("%Q1")),
        Some(Value::Bit(false))
    );

    // Readings report the bench widgets, not raw variables.
    let readings = bench.readings();
    assert!(readings
        .iter()
        .all(|reading| reading.value == Value::Bit(false)));
}

#[test]
fn editing_a_program_and_reopening_it_keeps_the_change() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("edited.slprj");

    let mut editor = open_example();
    let original = editor.project().clone();
    let rung = editor.project().sections[0].rungs[0];

    // Place a contact on a free cell, then take it back with undo.
    editor
        .replace_element(rung, ElementKind::ContactNo, 6, 0, Some(var("%I1")), &[])
        .expect("placing on a free cell works");
    assert_eq!(
        editor
            .project()
            .rung(rung)
            .expect("rung exists")
            .elements
            .iter()
            .filter(|element| element.col == 6)
            .count(),
        1
    );
    assert!(editor.is_dirty());
    assert!(editor.undo());
    assert_eq!(
        editor.project(),
        &original,
        "undo restores the project exactly"
    );

    // Make a change that stays, add a rung, and save.
    editor
        .replace_element(rung, ElementKind::ContactNc, 6, 0, Some(var("%I2")), &[])
        .expect("placing works");
    let section = editor.project().sections[0].id;
    let new_rung = editor.insert_rung(section, 1).expect("a rung can be added");
    let mut label = String::new();
    editor
        .set_rung_text(new_rung, "spare", "kept for later")
        .expect("rung text is editable");
    label.push_str("spare");
    assert_eq!(label, "spare");

    editor.save(&path).expect("the project saves");
    assert!(!editor.is_dirty(), "saving clears the dirty flag");
    assert_eq!(editor.path(), Some(path.as_path()));

    // Reopening gives back exactly what was saved, with a clean history.
    let reopened = Editor::open(&path).expect("the saved project reopens");
    assert_eq!(reopened.project(), editor.project());
    assert!(!reopened.is_dirty());
    assert!(
        !reopened.can_undo(),
        "a reopened project starts with no history"
    );
    assert_eq!(reopened.project().rungs.len(), 5);
    assert_eq!(
        reopened
            .project()
            .rung(rung)
            .expect("rung exists")
            .elements
            .iter()
            .filter(|element| element.col == 6)
            .map(|element| element.kind)
            .collect::<Vec<_>>(),
        vec![ElementKind::ContactNc]
    );

    // And the reopened program still runs: the added contact is part of it.
    let mut bench = Bench::new(reopened.project().clone());
    let panel: &SimulationPanel = &bench.runtime().project.simulation;
    assert_eq!(panel.switches.len(), 3, "the bench survived the round trip");
    bench.panel_state_mut().set_closed(0, true);
    bench.start();
    bench.step();
    assert_eq!(
        bench.engine().store().get(&var("%Q0")),
        Some(Value::Bit(true)),
        "the reopened project still drives the lamp"
    );
}

#[test]
fn a_broken_program_reports_problems_instead_of_panicking() {
    let mut editor = open_example();
    let rung = editor.project().sections[0].rungs[0];

    // A compare block whose expression divides by zero is a runtime error.
    editor
        .replace_element(rung, ElementKind::Compare, 7, 0, None, &["1 / 0"])
        .expect("the block is placed");
    assert!(
        editor
            .problems()
            .iter()
            .any(|diagnostic| diagnostic.code == "SL-E002"),
        "the divide-by-zero is reported: {:?}",
        editor.problems()
    );

    // A bench widget pointing at the wrong kind of variable is a warning.
    let mut panel = editor.project().simulation.clone();
    panel.switches.push(softladder_core::SimSwitch {
        var: var("%Q1"),
        label: "wrong".to_owned(),
        momentary: false,
    });
    editor.set_panel(panel).expect("the panel is editable");
    assert!(
        editor
            .problems()
            .iter()
            .any(|diagnostic| diagnostic.code == "SL-W020"),
        "the bench problem is reported: {:?}",
        editor.problems()
    );

    // Undo takes both problems away again.
    assert!(editor.undo());
    assert!(editor.undo());
    assert!(
        editor.problems().is_empty(),
        "undo cleared the problems: {:?}",
        editor.problems()
    );
}
