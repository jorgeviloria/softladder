//! Property tests for the undo contract on the sequential (SFC) chart.
//!
//! Two things must hold for *any* sequence of SFC commands, valid or not:
//!
//! 1. applying them all and then undoing all restores the original [`Project`]
//!    exactly, and redoing all restores the edited one;
//! 2. no sequence makes the editor panic, and the structural invariants of a
//!    chart hold throughout: step numbers are unique across the project (they
//!    index the engine's single `%X` array), no two steps or transitions share a
//!    cell of a page, and every step a transition names exists on that page.
//!
//! The strategy is deliberately small — two sections, three steps, two
//! transitions, coordinates in `0..4` — so a case is cheap and proptest can try
//! many of them.

use proptest::prelude::*;
use softladder_core::{Project, Rung, Section, SectionLanguage, SequentialPage, Step, Transition};
use softladder_edit::{Command, Editor};

/// Condition sources the generator may write; one of them does not parse, so the
/// rejection path is exercised too.
const CONDITIONS: [&str; 4] = ["%I0", "%M1 AND NOT %M2", "not an expression", ""];

/// The page the sample project starts from.
fn sample_page() -> SequentialPage {
    let mut page = SequentialPage::new(0, "sequence");
    page.steps.push(Step {
        number: 0,
        is_initial: true,
        x: 0,
        y: 0,
        page: 0,
    });
    page.steps.push(Step {
        number: 1,
        is_initial: false,
        x: 0,
        y: 2,
        page: 0,
    });
    page.steps.push(Step {
        number: 2,
        is_initial: false,
        x: 0,
        y: 4,
        page: 0,
    });
    page.transitions.push(Transition {
        number: 0,
        condition: Some("%I0".parse().expect("a condition parses")),
        from: vec![0],
        to: vec![1],
        page: 0,
        x: 0,
        y: 1,
    });
    page.transitions.push(Transition {
        number: 1,
        condition: None,
        from: vec![1],
        to: vec![2],
        page: 0,
        x: 0,
        y: 3,
    });
    page
}

/// The sample project every property starts from.
fn sample_project() -> Project {
    let mut project = Project::new("sfc proptest");
    project.sections.push(Section::new(0, "Main"));
    project.rungs.push(Rung::new(0));
    project.sections[0].rungs.push(0);
    project
        .sections
        .push(Section::sfc(1, "Sequence", sample_page()));
    // A section with no page at all, so the "no page" rejections are reached.
    let mut empty = Section::new(2, "Empty");
    empty.language = SectionLanguage::Sfc;
    project.sections.push(empty);
    project
}

/// One editing step, addressed by *position* so later steps still find the
/// object after earlier steps inserted or deleted elements.
#[derive(Debug, Clone)]
enum Op {
    InsertStep {
        section: usize,
        x: i32,
        y: i32,
        number: u32,
        initial: bool,
    },
    RemoveStep {
        section: usize,
        step: usize,
    },
    MoveStep {
        section: usize,
        step: usize,
        x: i32,
        y: i32,
    },
    SetNumber {
        section: usize,
        step: usize,
        number: u32,
    },
    SetInitial {
        section: usize,
        step: usize,
        initial: bool,
    },
    InsertTransition {
        section: usize,
        x: i32,
        y: i32,
        number: u32,
        from: usize,
        to: usize,
    },
    RemoveTransition {
        section: usize,
        transition: usize,
    },
    MoveTransition {
        section: usize,
        transition: usize,
        x: i32,
        y: i32,
    },
    SetCondition {
        section: usize,
        transition: usize,
        text: usize,
    },
    SetFrom {
        section: usize,
        transition: usize,
        members: u8,
    },
    SetTo {
        section: usize,
        transition: usize,
        members: u8,
    },
    LinkFrom {
        section: usize,
        transition: usize,
        step: usize,
        linked: bool,
    },
    LinkTo {
        section: usize,
        transition: usize,
        step: usize,
        linked: bool,
    },
    Comment {
        section: usize,
        text: u8,
    },
    AddPage {
        section: usize,
    },
    RemovePage {
        section: usize,
    },
}

fn op_strategy() -> impl Strategy<Value = Op> {
    prop_oneof![
        (0usize..3, -1i32..4, -1i32..5, 0u32..4, any::<bool>()).prop_map(
            |(section, x, y, number, initial)| Op::InsertStep {
                section,
                x,
                y,
                number,
                initial,
            }
        ),
        (0usize..3, 0usize..4).prop_map(|(section, step)| Op::RemoveStep { section, step }),
        (0usize..3, 0usize..4, -1i32..4, -1i32..5).prop_map(|(section, step, x, y)| Op::MoveStep {
            section,
            step,
            x,
            y,
        }),
        (0usize..3, 0usize..4, 0u32..4).prop_map(|(section, step, number)| Op::SetNumber {
            section,
            step,
            number,
        }),
        (0usize..3, 0usize..4, any::<bool>()).prop_map(|(section, step, initial)| Op::SetInitial {
            section,
            step,
            initial,
        }),
        (0usize..3, -1i32..4, -1i32..5, 0u32..4, 0usize..4, 0usize..4).prop_map(
            |(section, x, y, number, from, to)| Op::InsertTransition {
                section,
                x,
                y,
                number,
                from,
                to,
            }
        ),
        (0usize..3, 0usize..4).prop_map(|(section, transition)| Op::RemoveTransition {
            section,
            transition,
        }),
        (0usize..3, 0usize..4, -1i32..4, -1i32..5).prop_map(|(section, transition, x, y)| {
            Op::MoveTransition {
                section,
                transition,
                x,
                y,
            }
        }),
        (0usize..3, 0usize..4, 0usize..CONDITIONS.len()).prop_map(|(section, transition, text)| {
            Op::SetCondition {
                section,
                transition,
                text,
            }
        }),
        (0usize..3, 0usize..4, 0u8..8).prop_map(|(section, transition, members)| Op::SetFrom {
            section,
            transition,
            members,
        }),
        (0usize..3, 0usize..4, 0u8..8).prop_map(|(section, transition, members)| Op::SetTo {
            section,
            transition,
            members,
        }),
        (0usize..3, 0usize..4, 0usize..4, any::<bool>()).prop_map(
            |(section, transition, step, linked)| Op::LinkFrom {
                section,
                transition,
                step,
                linked,
            }
        ),
        (0usize..3, 0usize..4, 0usize..4, any::<bool>()).prop_map(
            |(section, transition, step, linked)| Op::LinkTo {
                section,
                transition,
                step,
                linked,
            }
        ),
        (0usize..3, 0u8..3).prop_map(|(section, text)| Op::Comment { section, text }),
        (0usize..3).prop_map(|section| Op::AddPage { section }),
        (0usize..3).prop_map(|section| Op::RemovePage { section }),
    ]
}

/// Resolves a section position to an id that cannot exist when it is out of range.
fn section_id(editor: &Editor, index: usize) -> u32 {
    editor
        .project()
        .sections
        .get(index)
        .map_or(9_000 + u32::try_from(index).unwrap_or(0), |section| {
            section.id
        })
}

/// Resolves a step position of `section` to its number.
fn step_number(editor: &Editor, section: usize, index: usize) -> Option<u32> {
    editor
        .project()
        .sections
        .get(section)
        .and_then(|entry| entry.sequential_page.as_ref())
        .and_then(|page| page.steps.get(index))
        .map(|step| step.number)
}

/// Resolves a transition position of `section` to its number.
fn transition_number(editor: &Editor, section: usize, index: usize) -> Option<u32> {
    editor
        .project()
        .sections
        .get(section)
        .and_then(|entry| entry.sequential_page.as_ref())
        .and_then(|page| page.transitions.get(index))
        .map(|transition| transition.number)
}

/// The set a `members` bit field describes: the step numbers `0..3` it selects.
fn members(editor: &Editor, section: usize, mask: u8) -> Vec<u32> {
    (0..4u32)
        .filter(|number| mask & (1 << number) != 0)
        .filter(|number| {
            editor
                .project()
                .sections
                .get(section)
                .and_then(|entry| entry.sequential_page.as_ref())
                .is_some_and(|page| page.step(*number).is_some())
        })
        .collect()
}

/// Applies one generated operation, ignoring rejections (they are part of the
/// contract under test).
fn run(editor: &mut Editor, op: &Op) {
    match op {
        Op::InsertStep {
            section,
            x,
            y,
            number,
            initial,
        } => {
            let id = section_id(editor, *section);
            let page = editor
                .project()
                .sections
                .get(*section)
                .and_then(|entry| entry.sequential_page.as_ref())
                .map_or(0, |page| page.number);
            let _ = editor.apply(Command::InsertStep {
                section: id,
                step: Step {
                    number: *number,
                    is_initial: *initial,
                    x: *x,
                    y: *y,
                    page,
                },
            });
        }
        Op::RemoveStep { section, step } => {
            let id = section_id(editor, *section);
            if let Some(number) = step_number(editor, *section, *step) {
                let _ = editor.remove_step(id, number);
            }
        }
        Op::MoveStep {
            section,
            step,
            x,
            y,
        } => {
            let id = section_id(editor, *section);
            if let Some(number) = step_number(editor, *section, *step) {
                let _ = editor.move_step(id, number, *x, *y);
            }
        }
        Op::SetNumber {
            section,
            step,
            number,
        } => {
            let id = section_id(editor, *section);
            if let Some(current) = step_number(editor, *section, *step) {
                let _ = editor.set_step_number(id, current, *number);
            }
        }
        Op::SetInitial {
            section,
            step,
            initial,
        } => {
            let id = section_id(editor, *section);
            if let Some(number) = step_number(editor, *section, *step) {
                let _ = editor.set_step_initial(id, number, *initial);
            }
        }
        Op::InsertTransition {
            section,
            x,
            y,
            number,
            from,
            to,
        } => {
            let id = section_id(editor, *section);
            let page = editor
                .project()
                .sections
                .get(*section)
                .and_then(|entry| entry.sequential_page.as_ref())
                .map_or(0, |page| page.number);
            let sources: Vec<u32> = step_number(editor, *section, *from).into_iter().collect();
            let targets: Vec<u32> = step_number(editor, *section, *to).into_iter().collect();
            let _ = editor.apply(Command::InsertTransition {
                section: id,
                transition: Transition {
                    number: *number,
                    condition: None,
                    from: sources,
                    to: targets,
                    page,
                    x: *x,
                    y: *y,
                },
            });
        }
        Op::RemoveTransition {
            section,
            transition,
        } => {
            let id = section_id(editor, *section);
            if let Some(number) = transition_number(editor, *section, *transition) {
                let _ = editor.remove_transition(id, number);
            }
        }
        Op::MoveTransition {
            section,
            transition,
            x,
            y,
        } => {
            let id = section_id(editor, *section);
            if let Some(number) = transition_number(editor, *section, *transition) {
                let _ = editor.move_transition(id, number, *x, *y);
            }
        }
        Op::SetCondition {
            section,
            transition,
            text,
        } => {
            let id = section_id(editor, *section);
            let text = CONDITIONS.get(*text).copied().unwrap_or("%I0");
            if let Some(number) = transition_number(editor, *section, *transition) {
                let _ = editor.set_transition_condition(id, number, text);
            }
        }
        Op::SetFrom {
            section,
            transition,
            members: mask,
        } => {
            let id = section_id(editor, *section);
            let set = members(editor, *section, *mask);
            if let Some(number) = transition_number(editor, *section, *transition) {
                let _ = editor.set_transition_from(id, number, &set);
            }
        }
        Op::SetTo {
            section,
            transition,
            members: mask,
        } => {
            let id = section_id(editor, *section);
            let set = members(editor, *section, *mask);
            if let Some(number) = transition_number(editor, *section, *transition) {
                let _ = editor.set_transition_to(id, number, &set);
            }
        }
        Op::LinkFrom {
            section,
            transition,
            step,
            linked,
        } => {
            let id = section_id(editor, *section);
            if let (Some(transition), Some(step)) = (
                transition_number(editor, *section, *transition),
                step_number(editor, *section, *step),
            ) {
                let _ = editor.link_transition_from(id, transition, step, *linked);
            }
        }
        Op::LinkTo {
            section,
            transition,
            step,
            linked,
        } => {
            let id = section_id(editor, *section);
            if let (Some(transition), Some(step)) = (
                transition_number(editor, *section, *transition),
                step_number(editor, *section, *step),
            ) {
                let _ = editor.link_transition_to(id, transition, step, *linked);
            }
        }
        Op::Comment { section, text } => {
            let id = section_id(editor, *section);
            let _ = editor.set_page_comment(id, &format!("note {text}"));
        }
        Op::AddPage { section } => {
            let id = section_id(editor, *section);
            let _ = editor.add_page(id);
        }
        Op::RemovePage { section } => {
            let id = section_id(editor, *section);
            let _ = editor.remove_page(id);
        }
    }
}

/// Asserts the structural invariants of every sequential page of the project.
fn check(editor: &Editor) {
    let project = editor.project();
    let mut numbers: Vec<u32> = Vec::new();
    for section in &project.sections {
        let Some(page) = section.sequential_page.as_ref() else {
            continue;
        };
        let mut cells: Vec<(i32, i32)> = Vec::new();
        for step in &page.steps {
            assert!(
                !numbers.contains(&step.number),
                "step {} is used by two pages",
                step.number
            );
            numbers.push(step.number);
            assert!(
                !cells.contains(&(step.x, step.y)),
                "two elements share cell ({}, {})",
                step.x,
                step.y
            );
            cells.push((step.x, step.y));
        }
        for transition in &page.transitions {
            assert!(
                !cells.contains(&(transition.x, transition.y)),
                "two elements share cell ({}, {})",
                transition.x,
                transition.y
            );
            cells.push((transition.x, transition.y));
            for number in transition.from.iter().chain(transition.to.iter()) {
                assert!(
                    page.step(*number).is_some(),
                    "transition {} names step {number}, which the page does not define",
                    transition.number
                );
            }
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Any sequence of SFC commands undoes and redoes exactly, and keeps the
    /// chart structurally sound while it runs.
    #[test]
    fn any_sequence_of_sfc_commands_undoes_exactly(ops in prop::collection::vec(op_strategy(), 0..24)) {
        let mut editor = Editor::new(sample_project());
        let original = editor.project().clone();

        for op in &ops {
            run(&mut editor, op);
            check(&editor);
        }
        let edited = editor.project().clone();

        let mut undone = 0;
        while editor.undo() {
            undone += 1;
            check(&editor);
        }
        prop_assert_eq!(editor.project(), &original, "undo-all restores the original");
        prop_assert!(undone <= ops.len());

        let mut redone = 0;
        while editor.redo() {
            redone += 1;
            check(&editor);
        }
        prop_assert_eq!(editor.project(), &edited, "redo-all restores the edit");
        prop_assert_eq!(redone, undone);
    }
}
