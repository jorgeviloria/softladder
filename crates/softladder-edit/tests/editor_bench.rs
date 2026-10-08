//! Integration: the editor and the bench are independent values.
//!
//! `docs/ARCHITECTURE.md` §9 says the UI renders `Project` + `Runtime` and emits
//! `Command`s. These tests exercise the intended split: an [`Editor`] can be
//! edited (which rescans a throwaway clone for its Problems list) while a
//! [`Bench`] keeps running, and the edited project can then be hot-reloaded into
//! the bench without losing the operator's positions.

use softladder_core::{
    ElementKind, PlacedElement, Project, Rung, Section, SimLamp, SimSwitch, SimulationPanel, Value,
    VarRef,
};
use softladder_edit::{Bench, Editor, RuntimeState};

fn var(text: &str) -> VarRef {
    text.parse().expect("test variable parses")
}

/// `%I0` drives `%Q0`, with one toggle and one lamp on the bench.
fn project() -> Project {
    let mut project = Project::new("editor and bench");
    let mut section = Section::new(1, "Main");
    section.rungs.push(1);
    project.sections.push(section);
    project.rungs.push(Rung {
        elements: vec![
            PlacedElement::with_var(ElementKind::ContactNo, var("%I0"), 0, 0),
            PlacedElement::with_var(ElementKind::CoilOut, var("%Q0"), 1, 0),
        ],
        ..Rung::new(1)
    });
    project.simulation = SimulationPanel {
        switches: vec![SimSwitch {
            var: var("%I0"),
            label: "start".to_owned(),
            momentary: false,
        }],
        lamps: vec![SimLamp {
            var: var("%Q0"),
            label: "green".to_owned(),
        }],
        ..SimulationPanel::default()
    };
    project
}

#[test]
fn editing_the_project_never_disturbs_a_running_bench() {
    let project = project();
    let mut editor = Editor::new(project.clone());
    let mut bench = Bench::new(project);
    bench.start();
    bench.panel_state_mut().toggle(0);
    bench.step().expect("the running bench scans");
    assert_eq!(bench.readings()[0].value, Value::Bit(true));

    // Every one of these edits rescans a throwaway clone for the Problems list.
    editor
        .place_element(1, ElementKind::ContactNc, 2, 0, Some(var("%I1")), &[])
        .expect("places");
    editor
        .set_rung_text(1, "LAMP", "edited while running")
        .expect("sets text");
    editor.delete_element(1, 2, 0).expect("deletes");
    assert!(editor.is_dirty());
    assert!(editor.undo());

    assert_eq!(bench.state(), RuntimeState::Run);
    assert_eq!(bench.cycles(), 1);
    assert_eq!(bench.readings()[0].value, Value::Bit(true));
    assert_eq!(
        bench.engine().store().get(&var("%Q0")),
        Some(Value::Bit(true))
    );
}

#[test]
fn an_edited_project_can_be_hot_reloaded_into_the_bench() {
    let project = project();
    let mut editor = Editor::new(project.clone());
    let mut bench = Bench::new(project);
    bench.start();
    bench.panel_state_mut().toggle(0);
    bench.step().expect("scans the original program");

    editor
        .delete_element(1, 1, 0)
        .expect("removes the old coil");
    editor
        .place_element(1, ElementKind::CoilOut, 1, 0, Some(var("%Q1")), &[])
        .expect("places a new coil");
    bench.reload(editor.project().clone());

    assert!(
        bench.panel_state().is_closed(0),
        "the operator position survives the reload"
    );
    assert_eq!(bench.engine().project(), editor.project());
    bench.step().expect("scans the reloaded program");
    assert_eq!(
        bench.engine().store().get(&var("%Q1")),
        Some(Value::Bit(true)),
        "the reloaded program drives the new output"
    );
    assert_eq!(bench.cycles(), 2);
}

#[test]
fn dropping_an_element_on_an_occupied_cell_is_one_undo_step() {
    let mut editor = Editor::new(project());
    let rung = editor.project().sections[0].rungs[0];

    editor
        .place_element(rung, ElementKind::ContactNo, 2, 0, Some(var("%I1")), &[])
        .expect("the cell starts free");
    assert_eq!(
        editor.project().rung(rung).map(|r| r.elements.len()),
        Some(3)
    );

    // Placing on top is refused, replacing is not.
    assert!(editor
        .place_element(rung, ElementKind::CoilOut, 2, 0, Some(var("%Q1")), &[])
        .is_err());
    editor
        .replace_element(rung, ElementKind::CoilOut, 2, 0, Some(var("%Q1")), &[])
        .expect("replacing an occupied cell is allowed");

    let cell = |editor: &Editor| {
        editor
            .project()
            .rung(rung)
            .expect("rung exists")
            .elements
            .iter()
            .find(|element| element.col == 2)
            .cloned()
    };

    let replaced = cell(&editor).expect("the cell is still occupied");
    assert_eq!(replaced.kind, ElementKind::CoilOut);
    assert_eq!(replaced.var, Some(var("%Q1")));
    assert_eq!(
        editor
            .project()
            .rung(rung)
            .expect("rung exists")
            .elements
            .len(),
        3,
        "the replaced element is gone, not shadowed"
    );

    // One undo restores the original element, not an empty cell.
    assert!(editor.undo());
    let restored = cell(&editor).expect("the old element is back");
    assert_eq!(restored.kind, ElementKind::ContactNo);
    assert_eq!(restored.var, Some(var("%I1")));

    assert!(editor.redo());
    assert_eq!(
        cell(&editor).map(|element| element.kind),
        Some(ElementKind::CoilOut)
    );
}
