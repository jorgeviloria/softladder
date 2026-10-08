//! Property tests for the undo contract.
//!
//! Two things must hold for *any* sequence of commands, valid or not:
//!
//! 1. applying them all and then undoing all restores the original [`Project`]
//!    exactly, and redoing all restores the edited one;
//! 2. no sequence makes the editor panic, and the structural invariant (every
//!    rung a section references exists, and rung ids stay unique) is preserved.
//!
//! The strategy is deliberately small — two sections, three rungs, five columns,
//! four rows — so a case is cheap and proptest can try many of them.

use proptest::prelude::*;
use softladder_core::{
    CounterKind, ElementKind, PlacedElement, Project, Rung, ScanConfig, Section, SectionLanguage,
    SimSwitch, SimulationPanel, Symbol, TimerMode, VarKind, VarRef,
};
use softladder_edit::Editor;

/// Element kinds the generator may place or switch to.
///
/// Jump and call coils are deliberately absent: they can re-enter the scan
/// engine, and this test is about the edit history, not the evaluator.
const ELEMENT_KINDS: [ElementKind; 13] = [
    ElementKind::ContactNo,
    ElementKind::ContactNc,
    ElementKind::ContactRising,
    ElementKind::ContactFalling,
    ElementKind::CoilOut,
    ElementKind::CoilOutNeg,
    ElementKind::CoilSet,
    ElementKind::CoilReset,
    ElementKind::Connection,
    ElementKind::Compare,
    ElementKind::Operate,
    ElementKind::Timer {
        mode: TimerMode::On,
    },
    ElementKind::Counter {
        kind: CounterKind::Up,
    },
];

/// Variable kinds the generator may bind.
const VAR_KINDS: [VarKind; 6] = [
    VarKind::MemBit,
    VarKind::MemWord,
    VarKind::PhysIn,
    VarKind::PhysOut,
    VarKind::PhysInWord,
    VarKind::PhysOutWord,
];

/// Parameter lists the generator may write.
const PARAM_SETS: [&[&str]; 3] = [&["1"], &["%MW0", "=", "1"], &[]];

/// Builds a small, valid variable reference from a generator choice.
fn make_var(choice: usize) -> VarRef {
    let kind = VAR_KINDS
        .get(choice % VAR_KINDS.len())
        .copied()
        .unwrap_or(VarKind::MemBit);
    let index = u32::try_from(choice % 3).unwrap_or(0);
    VarRef::new(kind, index)
}

/// One editing step, addressed by *position* so that later steps still find the
/// object after earlier steps inserted or deleted rungs.
#[derive(Debug, Clone)]
enum Op {
    Place {
        rung: usize,
        col: u8,
        row: u8,
        kind: usize,
        var: usize,
        bind: bool,
    },
    Remove {
        rung: usize,
        col: u8,
        row: u8,
    },
    Move {
        rung: usize,
        from: (u8, u8),
        to: (u8, u8),
    },
    SetVar {
        rung: usize,
        col: u8,
        row: u8,
        var: usize,
    },
    SetParams {
        rung: usize,
        col: u8,
        row: u8,
        params: usize,
    },
    SetKind {
        rung: usize,
        col: u8,
        row: u8,
        kind: usize,
    },
    SetLink {
        rung: usize,
        col: u8,
        row: u8,
        linked: bool,
    },
    Text {
        rung: usize,
        label: u8,
    },
    InsertRung {
        section: usize,
        index: usize,
    },
    DeleteRung {
        section: usize,
        rung: usize,
    },
    MoveRung {
        section: usize,
        from: usize,
        to: usize,
    },
    AddSection {
        name: u8,
    },
    RemoveSection {
        section: usize,
    },
    RenameSection {
        section: usize,
        name: u8,
    },
    SetSymbols {
        count: usize,
    },
    SetPanel {
        count: usize,
    },
    SetScan {
        period_ms: u32,
    },
}

fn op_strategy() -> impl Strategy<Value = Op> {
    prop_oneof![
        (
            0usize..4,
            0u8..5,
            0u8..4,
            0usize..ELEMENT_KINDS.len(),
            0usize..VAR_KINDS.len(),
            any::<bool>()
        )
            .prop_map(|(rung, col, row, kind, var, bind)| Op::Place {
                rung,
                col,
                row,
                kind,
                var,
                bind,
            }),
        (0usize..4, 0u8..5, 0u8..4).prop_map(|(rung, col, row)| Op::Remove { rung, col, row }),
        (0usize..4, 0u8..5, 0u8..4, 0u8..5, 0u8..4).prop_map(|(rung, fc, fr, tc, tr)| Op::Move {
            rung,
            from: (fc, fr),
            to: (tc, tr),
        }),
        (0usize..4, 0u8..5, 0u8..4, 0usize..VAR_KINDS.len()).prop_map(|(rung, col, row, var)| {
            Op::SetVar {
                rung,
                col,
                row,
                var,
            }
        }),
        (0usize..4, 0u8..5, 0u8..4, 0usize..PARAM_SETS.len()).prop_map(
            |(rung, col, row, params)| Op::SetParams {
                rung,
                col,
                row,
                params,
            }
        ),
        (0usize..4, 0u8..5, 0u8..4, 0usize..ELEMENT_KINDS.len()).prop_map(
            |(rung, col, row, kind)| Op::SetKind {
                rung,
                col,
                row,
                kind,
            }
        ),
        (0usize..4, 0u8..5, 0u8..4, any::<bool>()).prop_map(|(rung, col, row, linked)| {
            Op::SetLink {
                rung,
                col,
                row,
                linked,
            }
        }),
        (0usize..4, 0u8..8).prop_map(|(rung, label)| Op::Text { rung, label }),
        (0usize..2, 0usize..4).prop_map(|(section, index)| Op::InsertRung { section, index }),
        (0usize..2, 0usize..4).prop_map(|(section, rung)| Op::DeleteRung { section, rung }),
        (0usize..2, 0usize..4, 0usize..4).prop_map(|(section, from, to)| Op::MoveRung {
            section,
            from,
            to,
        }),
        (0u8..4).prop_map(|name| Op::AddSection { name }),
        (0usize..3).prop_map(|section| Op::RemoveSection { section }),
        (0usize..3, 0u8..4).prop_map(|(section, name)| Op::RenameSection { section, name }),
        (0usize..3).prop_map(|count| Op::SetSymbols { count }),
        (0usize..3).prop_map(|count| Op::SetPanel { count }),
        (1u32..40).prop_map(|period_ms| Op::SetScan { period_ms }),
    ]
}

/// The sample project every property starts from.
fn sample_project() -> Project {
    let mut project = Project::new("proptest");
    let mut main = Section::new(0, "Main");
    main.rungs.push(0);
    main.rungs.push(1);
    project.sections.push(main);
    let mut sub = Section::new(1, "Sub");
    sub.rungs.push(2);
    project.sections.push(sub);
    project.rungs.push(Rung {
        elements: vec![
            PlacedElement::with_var(ElementKind::ContactNo, make_var(0), 0, 0),
            PlacedElement::with_var(ElementKind::CoilOut, make_var(1), 1, 0),
        ],
        ..Rung::new(0)
    });
    project.rungs.push(Rung {
        elements: vec![PlacedElement::with_var(
            ElementKind::ContactNc,
            make_var(2),
            0,
            0,
        )],
        ..Rung::new(1)
    });
    project.rungs.push(Rung::new(2));
    project.simulation = SimulationPanel {
        switches: vec![SimSwitch {
            var: make_var(0),
            label: "in".to_owned(),
            momentary: false,
        }],
        ..SimulationPanel::default()
    };
    project
}

/// Resolves a section position to an id, falling back to an id that cannot exist.
fn section_id(editor: &Editor, index: usize) -> u32 {
    editor
        .project()
        .sections
        .get(index)
        .map_or(8_000 + u32::try_from(index).unwrap_or(0), |section| {
            section.id
        })
}

/// Resolves a rung position to an id, falling back to an id that cannot exist.
fn rung_id(editor: &Editor, index: usize) -> u32 {
    editor
        .project()
        .rungs
        .get(index)
        .map_or(9_000 + u32::try_from(index).unwrap_or(0), |rung| rung.id)
}

/// Applies one generated operation, ignoring rejections (they are part of the
/// contract under test).
fn run(editor: &mut Editor, op: &Op) {
    match op {
        Op::Place {
            rung,
            col,
            row,
            kind,
            var,
            bind,
        } => {
            let id = rung_id(editor, *rung);
            let kind = ELEMENT_KINDS
                .get(*kind)
                .copied()
                .unwrap_or(ElementKind::ContactNo);
            let variable = if *bind { Some(make_var(*var)) } else { None };
            let _ = editor.place_element(id, kind, *col, *row, variable, &[]);
        }
        Op::Remove { rung, col, row } => {
            let id = rung_id(editor, *rung);
            let _ = editor.delete_element(id, *col, *row);
        }
        Op::Move { rung, from, to } => {
            let id = rung_id(editor, *rung);
            let _ = editor.move_element(id, *from, *to);
        }
        Op::SetVar {
            rung,
            col,
            row,
            var,
        } => {
            let id = rung_id(editor, *rung);
            let _ = editor.set_element_var(id, *col, *row, Some(make_var(*var)));
        }
        Op::SetParams {
            rung,
            col,
            row,
            params,
        } => {
            let id = rung_id(editor, *rung);
            let list = PARAM_SETS.get(*params).copied().unwrap_or(&[]);
            let _ = editor.set_element_params(id, *col, *row, list);
        }
        Op::SetKind {
            rung,
            col,
            row,
            kind,
        } => {
            let id = rung_id(editor, *rung);
            let kind = ELEMENT_KINDS
                .get(*kind)
                .copied()
                .unwrap_or(ElementKind::ContactNo);
            let _ = editor.apply(softladder_edit::Command::SetElementKind {
                rung: id,
                col: *col,
                row: *row,
                kind,
            });
        }
        Op::SetLink {
            rung,
            col,
            row,
            linked,
        } => {
            let id = rung_id(editor, *rung);
            let _ = editor.set_vertical_link(id, *col, *row, *linked);
        }
        Op::Text { rung, label } => {
            let id = rung_id(editor, *rung);
            let text = format!("L{label}");
            let _ = editor.set_rung_text(id, &text, &text);
        }
        Op::InsertRung { section, index } => {
            let id = section_id(editor, *section);
            let _ = editor.insert_rung(id, *index);
        }
        Op::DeleteRung { section, rung } => {
            let section = section_id(editor, *section);
            let rung = rung_id(editor, *rung);
            let _ = editor.delete_rung(section, rung);
        }
        Op::MoveRung { section, from, to } => {
            let section = section_id(editor, *section);
            let _ = editor.apply(softladder_edit::Command::MoveRung {
                section,
                from: *from,
                to: *to,
            });
        }
        Op::AddSection { name } => {
            let _ = editor.add_section(&format!("s{name}"), SectionLanguage::Ladder);
        }
        Op::RemoveSection { section } => {
            let id = section_id(editor, *section);
            let _ = editor.remove_section(id);
        }
        Op::RenameSection { section, name } => {
            let id = section_id(editor, *section);
            let _ = editor.rename_section(id, &format!("r{name}"));
        }
        Op::SetSymbols { count } => {
            let symbols = (0..*count)
                .map(|index| Symbol {
                    name: format!("s{index}"),
                    var: Some(make_var(index)),
                    comment: String::new(),
                    unit: None,
                })
                .collect();
            let _ = editor.set_symbols(symbols);
        }
        Op::SetPanel { count } => {
            let switches = (0..*count)
                .map(|index| SimSwitch {
                    var: VarRef::new(VarKind::PhysIn, u32::try_from(index).unwrap_or(0)),
                    label: format!("sw{index}"),
                    momentary: index % 2 == 0,
                })
                .collect();
            let _ = editor.set_panel(SimulationPanel {
                switches,
                ..SimulationPanel::default()
            });
        }
        Op::SetScan { period_ms } => {
            let _ = editor.set_scan_config(ScanConfig {
                period_ms: *period_ms,
                input_period_ms: (*period_ms / 2).max(1),
            });
        }
    }
}

/// Every rung a section references must exist, and rung ids must be unique.
fn assert_consistent(project: &Project) {
    let mut ids: Vec<u32> = project.rungs.iter().map(|rung| rung.id).collect();
    let unique = ids.len();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), unique, "rung ids stay unique");
    for section in &project.sections {
        for id in &section.rungs {
            assert!(
                project.rung(*id).is_some(),
                "section `{}` references missing rung {id}",
                section.name
            );
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]

    #[test]
    fn undoing_everything_restores_the_original_project(ops in prop::collection::vec(op_strategy(), 0..14)) {
        let original = sample_project();
        let mut editor = Editor::new(original.clone());
        for op in &ops {
            run(&mut editor, op);
        }
        let edited = editor.project().clone();

        let mut undone = 0usize;
        while editor.undo() {
            undone += 1;
        }
        prop_assert_eq!(editor.project(), &original, "undo-all must be exact");
        prop_assert!(undone <= softladder_edit::MAX_HISTORY);

        let mut redone = 0usize;
        while editor.redo() {
            redone += 1;
        }
        prop_assert_eq!(redone, undone);
        prop_assert_eq!(editor.project(), &edited, "redo-all must be exact");
    }

    #[test]
    fn no_sequence_breaks_the_project_invariants(ops in prop::collection::vec(op_strategy(), 0..24)) {
        let mut editor = Editor::new(sample_project());
        for op in &ops {
            run(&mut editor, op);
        }
        assert_consistent(editor.project());

        // The diagnostics path scans a throwaway clone and must never panic,
        // whatever the commands left behind.
        editor.refresh_diagnostics();

        while editor.undo() {
            assert_consistent(editor.project());
        }
        while editor.redo() {
            assert_consistent(editor.project());
        }
    }
}
