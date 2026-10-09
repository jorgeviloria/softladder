//! The SFC (sequential) command vocabulary, exercised without a window.
//!
//! Every command has to be validated before it mutates anything, has to record
//! exactly one undo step, and has to round-trip through undo and redo. The last
//! group of tests drives deliberately hostile charts — a transition naming steps
//! that do not exist, a step recorded on another page, a section with no page —
//! because those are what a real imported project looks like when it is broken,
//! and the editor must open, draw and repair them rather than panic.

use softladder_core::{Project, Rung, Section, SectionLanguage, SequentialPage, Step, Transition};
use softladder_edit::{Command, EditError, Editor};

/// A page holding two steps and the transition between them.
fn sfc_page() -> SequentialPage {
    let mut page = SequentialPage::new(0, "main sequence");
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
    page.transitions.push(Transition {
        number: 0,
        condition: Some("%I0".parse().expect("a condition parses")),
        from: vec![0],
        to: vec![1],
        page: 0,
        x: 0,
        y: 1,
    });
    page
}

/// A project with a ladder section and an SFC section that owns [`sfc_page`].
fn sfc_project() -> Project {
    let mut project = Project::new("sfc tests");
    let mut main = Section::new(1, "Main");
    main.rungs.push(1);
    project.sections.push(main);
    project.rungs.push(Rung::new(1));
    project
        .sections
        .push(Section::sfc(2, "Sequence", sfc_page()));
    project
}

fn sfc_editor() -> Editor {
    Editor::new(sfc_project())
}

/// The page of the SFC section of a fixture editor.
fn page(editor: &Editor) -> &SequentialPage {
    editor
        .project()
        .section(2)
        .and_then(|section| section.sequential_page.as_ref())
        .expect("the SFC section has a page")
}

/// The transition with `number` of the fixture page.
fn transition(editor: &Editor, number: u32) -> Transition {
    page(editor)
        .transition(number)
        .cloned()
        .expect("the transition exists")
}

/// Applies a command that must be rejected and checks the rejection is a
/// complete no-op: project, history and dirty flag all unchanged.
fn reject(editor: &mut Editor, command: Command) -> EditError {
    let before = editor.project().clone();
    let history = editor.history_len();
    let dirty = editor.is_dirty();
    let error = editor
        .apply(command)
        .expect_err("the command must be rejected");
    assert_eq!(
        editor.project(),
        &before,
        "a rejected command must not change the project"
    );
    assert_eq!(
        editor.history_len(),
        history,
        "a rejected command must not record history"
    );
    assert_eq!(
        editor.is_dirty(),
        dirty,
        "a rejected command must not change the dirty flag"
    );
    error
}

#[test]
fn steps_are_inserted_renumbered_moved_and_removed_reversibly() {
    let mut editor = sfc_editor();
    let original = editor.project().clone();

    let number = editor.insert_step(2, 1, 1, false).expect("inserts");
    assert_eq!(number, 2, "the largest step number plus one");
    let inserted = page(&editor)
        .step(2)
        .cloned()
        .expect("the step is on the page");
    assert_eq!((inserted.x, inserted.y), (1, 1));
    assert_eq!(inserted.page, 0, "a step lands on its section's page");
    assert!(!inserted.is_initial);

    editor
        .set_step_initial(2, 2, true)
        .expect("marks it initial");
    assert!(page(&editor).step(2).is_some_and(|step| step.is_initial));

    editor.set_step_number(2, 2, 7).expect("renumbers");
    assert!(page(&editor).step(2).is_none());
    assert!(page(&editor).step(7).is_some());

    editor.move_step(2, 7, 3, 3).expect("moves");
    let moved = page(&editor).step(7).cloned().expect("still there");
    assert_eq!((moved.x, moved.y), (3, 3));

    editor.remove_step(2, 7).expect("removes");
    assert!(page(&editor).step(7).is_none());

    let mut undone = 0;
    while editor.undo() {
        undone += 1;
    }
    assert_eq!(undone, 5);
    assert_eq!(editor.project(), &original, "undo-all restores the chart");

    let mut redone = 0;
    while editor.redo() {
        redone += 1;
    }
    assert_eq!(redone, 5);
    assert!(page(&editor).step(7).is_none());
}

#[test]
fn inserting_a_step_on_an_occupied_cell_replaces_it_in_one_command() {
    let mut editor = sfc_editor();
    let history = editor.history_len();
    // Cell (0, 2) holds step 1, which the transition names as its target.
    editor.insert_step(2, 0, 2, false).expect("replaces");
    assert_eq!(editor.history_len(), history + 1, "one undo step");
    assert!(page(&editor).step(1).is_none(), "the old step is gone");
    let replacement = page(&editor).step(2).cloned().expect("the new step");
    assert_eq!((replacement.x, replacement.y), (0, 2));
    assert!(
        transition(&editor, 0).to.is_empty(),
        "the displaced step is pruned from the chart"
    );

    assert!(editor.undo());
    assert!(page(&editor).step(1).is_some());
    assert_eq!(transition(&editor, 0).to, vec![1]);
}

#[test]
fn step_numbers_are_unique_across_the_whole_project() {
    let mut project = sfc_project();
    project
        .sections
        .push(Section::sfc(3, "Second", SequentialPage::new(1, "second")));
    let mut editor = Editor::new(project);
    editor
        .insert_step(3, 0, 0, false)
        .expect("the fresh number skips the other page");
    assert!(
        editor
            .project()
            .section(3)
            .and_then(|section| section.sequential_page.as_ref())
            .is_some_and(|page| page.step(2).is_some()),
        "the new step took the first number no page uses"
    );

    // A number another page uses is refused rather than duplicated.
    let error = reject(
        &mut editor,
        Command::SetStepNumber {
            section: 2,
            step: 0,
            number: 2,
        },
    );
    assert!(matches!(error, EditError::DuplicateStep(2)));
}

#[test]
fn step_commands_reject_unknown_targets_and_busy_cells() {
    let mut editor = sfc_editor();
    assert!(matches!(
        reject(
            &mut editor,
            Command::RemoveStep {
                section: 2,
                step: 9
            }
        ),
        EditError::UnknownStep { step: 9, .. }
    ));
    assert!(matches!(
        reject(
            &mut editor,
            Command::SetStepInitial {
                section: 2,
                step: 9,
                initial: true
            }
        ),
        EditError::UnknownStep { step: 9, .. }
    ));
    assert!(matches!(
        reject(
            &mut editor,
            Command::MoveStep {
                section: 2,
                step: 0,
                x: 0,
                y: 2
            }
        ),
        EditError::SfcCellOccupied { x: 0, y: 2, .. }
    ));
    assert!(matches!(
        reject(
            &mut editor,
            Command::MoveStep {
                section: 2,
                step: 0,
                x: 0,
                y: 0
            }
        ),
        EditError::SfcCellOccupied { .. }
    ));
    assert!(matches!(
        reject(
            &mut editor,
            Command::InsertStep {
                section: 2,
                step: Step {
                    number: 0,
                    x: 5,
                    y: 5,
                    ..Step::new(0, 0)
                }
            }
        ),
        EditError::DuplicateStep(0)
    ));
    assert!(matches!(
        reject(
            &mut editor,
            Command::SetStepNumber {
                section: 2,
                step: 0,
                number: 1
            }
        ),
        EditError::DuplicateStep(1)
    ));
    assert!(matches!(
        reject(
            &mut editor,
            Command::RemoveStep {
                section: 9,
                step: 0
            }
        ),
        EditError::UnknownSection(9)
    ));
}

#[test]
fn renumbering_a_step_follows_every_reference_to_it() {
    let mut editor = sfc_editor();
    let before = editor.project().clone();
    editor.set_step_number(2, 1, 5).expect("renumbers");
    let updated = transition(&editor, 0);
    assert_eq!(updated.from, vec![0]);
    assert_eq!(updated.to, vec![5], "the reference follows the number");
    assert!(editor.undo());
    assert_eq!(editor.project(), &before);
    editor.redo();
    assert_eq!(transition(&editor, 0).to, vec![5]);
}

#[test]
fn removing_a_step_prunes_the_transitions_that_named_it() {
    let mut editor = sfc_editor();
    let before = editor.project().clone();
    editor.remove_step(2, 1).expect("removes");
    assert!(transition(&editor, 0).to.is_empty());
    assert_eq!(
        page(&editor).transitions.len(),
        1,
        "the transition itself stays"
    );
    assert!(editor.undo());
    assert_eq!(editor.project(), &before);
}

#[test]
fn transitions_are_inserted_linked_moved_and_removed() {
    let mut editor = sfc_editor();
    let original = editor.project().clone();
    let number = editor
        .insert_transition_linked(2, 1, 3, &[1, 1], &[0])
        .expect("inserts");
    assert_eq!(number, 1);
    let inserted = transition(&editor, 1);
    assert_eq!(inserted.from, vec![1], "the set is deduplicated");
    assert_eq!(inserted.to, vec![0]);
    assert_eq!((inserted.x, inserted.y), (1, 3));

    editor
        .set_transition_condition(2, 1, "%I1 AND %I2")
        .expect("sets a condition");
    assert!(transition(&editor, 1).condition.is_some());
    editor
        .set_transition_condition(2, 1, "")
        .expect("clears the condition");
    assert!(transition(&editor, 1).condition.is_none());

    editor.move_transition(2, 1, 2, 3).expect("moves");
    assert_eq!(
        transition(&editor, 1).x,
        2,
        "the transition moved to another cell"
    );

    editor.remove_transition(2, 1).expect("removes");
    assert!(page(&editor).transition(1).is_none());

    while editor.undo() {}
    assert_eq!(editor.project(), &original);
    while editor.redo() {}
    assert!(page(&editor).transition(1).is_none());
}

#[test]
fn transition_commands_reject_unknown_targets_bad_sets_and_busy_cells() {
    let mut editor = sfc_editor();
    assert!(matches!(
        reject(
            &mut editor,
            Command::RemoveTransition {
                section: 2,
                transition: 9
            }
        ),
        EditError::UnknownTransition { transition: 9, .. }
    ));
    assert!(matches!(
        reject(
            &mut editor,
            Command::MoveTransition {
                section: 2,
                transition: 0,
                x: 0,
                y: 0
            }
        ),
        EditError::SfcCellOccupied { .. }
    ));
    assert!(matches!(
        reject(
            &mut editor,
            Command::SetTransitionFrom {
                section: 2,
                transition: 0,
                from: vec![42]
            }
        ),
        EditError::UnknownStep { step: 42, .. }
    ));
    assert!(matches!(
        reject(
            &mut editor,
            Command::SetTransitionTo {
                section: 2,
                transition: 0,
                to: vec![0, 42]
            }
        ),
        EditError::UnknownStep { step: 42, .. }
    ));
    assert!(matches!(
        reject(
            &mut editor,
            Command::InsertTransition {
                section: 2,
                transition: Transition {
                    number: 9,
                    from: vec![7],
                    ..Transition::new(9, 0)
                }
            }
        ),
        EditError::UnknownStep { step: 7, .. }
    ));
    assert!(matches!(
        reject(
            &mut editor,
            Command::InsertTransition {
                section: 2,
                transition: Transition::new(0, 0)
            }
        ),
        EditError::DuplicateTransition(0)
    ));
}

#[test]
fn a_condition_that_does_not_parse_is_refused_with_its_reason() {
    let mut editor = sfc_editor();
    let before = editor.project().clone();
    let error = reject(
        &mut editor,
        Command::SetTransitionCondition {
            section: 2,
            transition: 0,
            condition: Some("%?".to_owned()),
        },
    );
    match error {
        EditError::BadCondition { text, message } => {
            assert_eq!(text, "%?");
            assert!(!message.is_empty(), "the reason is reported");
        }
        other => panic!("expected BadCondition, got {other}"),
    }
    assert_eq!(editor.project(), &before);
    assert!(matches!(
        reject(
            &mut editor,
            Command::SetTransitionCondition {
                section: 2,
                transition: 9,
                condition: Some("%I0".to_owned())
            }
        ),
        EditError::UnknownTransition { .. }
    ));
}

#[test]
fn transition_sets_can_be_replaced_and_linked_member_by_member() {
    let mut editor = sfc_editor();
    let before = editor.project().clone();
    editor
        .set_transition_from(2, 0, &[1, 0, 1])
        .expect("replaces the set");
    assert_eq!(transition(&editor, 0).from, vec![0, 1], "sorted, unique");
    editor
        .set_transition_from(2, 0, &[])
        .expect("clears the set");
    assert!(transition(&editor, 0).from.is_empty());

    editor
        .link_transition_from(2, 0, 1, true)
        .expect("links the source");
    assert_eq!(transition(&editor, 0).from, vec![1]);
    editor
        .link_transition_to(2, 0, 0, true)
        .expect("links the target");
    assert_eq!(transition(&editor, 0).to, vec![0, 1]);
    editor
        .link_transition_to(2, 0, 0, false)
        .expect("unlinks the target");
    assert_eq!(transition(&editor, 0).to, vec![1]);
    editor
        .link_transition_from(2, 0, 1, false)
        .expect("unlinks the source");
    assert!(transition(&editor, 0).from.is_empty());

    assert!(matches!(
        reject(
            &mut editor,
            Command::SetTransitionTo {
                section: 9,
                transition: 0,
                to: Vec::new()
            }
        ),
        EditError::UnknownSection(9)
    ));
    assert!(matches!(
        reject(
            &mut editor,
            Command::SetTransitionFrom {
                section: 2,
                transition: 9,
                from: Vec::new()
            }
        ),
        EditError::UnknownTransition { .. }
    ));

    while editor.undo() {}
    assert_eq!(editor.project(), &before);
}

#[test]
fn a_page_can_be_added_commented_and_removed() {
    let mut project = Project::new("pages");
    project.sections.push(Section::new(1, "Main"));
    let mut sub = Section::new(2, "Sequence");
    sub.language = SectionLanguage::Sfc;
    project.sections.push(sub);
    let mut editor = Editor::new(project);

    let number = editor.add_page(2).expect("adds a page");
    assert_eq!(number, 0);
    assert!(page(&editor).is_empty());

    editor
        .set_page_comment(2, "start-up and stop")
        .expect("comments");
    assert_eq!(page(&editor).comment, "start-up and stop");
    editor.set_page_comment(2, "").expect("clears the comment");
    assert!(page(&editor).comment.is_empty());

    // A page number the project already uses is never handed out twice.
    let mut other = Project::new("pages");
    other.sections.push(Section::new(1, "Main"));
    let mut sub = Section::new(2, "Sequence");
    sub.language = SectionLanguage::Sfc;
    other.sections.push(sub);
    other
        .sections
        .push(Section::sfc(3, "Third", SequentialPage::new(4, "")));
    let mut editor = Editor::new(other);
    assert_eq!(editor.add_page(2).expect("adds"), 5);

    assert!(matches!(
        reject(
            &mut editor,
            Command::AddPage {
                section: 3,
                page: SequentialPage::new(9, "")
            }
        ),
        EditError::PageExists(3)
    ));
    editor.remove_page(3).expect("removes the page");
    assert!(editor
        .project()
        .section(3)
        .is_some_and(|section| section.sequential_page.is_none()));
    assert!(matches!(
        reject(&mut editor, Command::RemovePage { section: 3 }),
        EditError::NoSequentialPage(3)
    ));
}

#[test]
fn an_sfc_section_without_a_page_refuses_every_page_command() {
    let mut project = Project::new("no page");
    let mut sub = Section::new(1, "Sequence");
    sub.language = SectionLanguage::Sfc;
    project.sections.push(sub);
    let mut editor = Editor::new(project);

    assert!(matches!(
        reject(
            &mut editor,
            Command::InsertStep {
                section: 1,
                step: Step::new(0, 0)
            }
        ),
        EditError::NoSequentialPage(1)
    ));
    assert!(matches!(
        reject(
            &mut editor,
            Command::InsertTransition {
                section: 1,
                transition: Transition::new(0, 0)
            }
        ),
        EditError::NoSequentialPage(1)
    ));
    assert!(matches!(
        reject(
            &mut editor,
            Command::SetPageComment {
                section: 1,
                comment: "x".to_owned()
            }
        ),
        EditError::NoSequentialPage(1)
    ));
    assert!(matches!(
        reject(
            &mut editor,
            Command::SetTransitionFrom {
                section: 1,
                transition: 0,
                from: Vec::new()
            }
        ),
        EditError::NoSequentialPage(1)
    ));
    assert!(matches!(
        reject(
            &mut editor,
            Command::RemoveStep {
                section: 1,
                step: 0
            }
        ),
        EditError::NoSequentialPage(1)
    ));
    assert!(matches!(
        reject(
            &mut editor,
            Command::RemoveTransition {
                section: 1,
                transition: 0
            }
        ),
        EditError::NoSequentialPage(1)
    ));

    // And the SFC section is still a valid, loadable program.
    assert!(!editor.is_dirty());
}

#[test]
fn a_hostile_chart_never_panics_the_editor() {
    // A transition pointing at steps that do not exist, a step and a transition
    // recorded on another page, and a section with no page at all.
    let mut page = SequentialPage::new(3, "hostile");
    page.steps.push(Step {
        number: 4,
        is_initial: false,
        x: 0,
        y: 0,
        page: 9,
    });
    page.transitions.push(Transition {
        number: 2,
        condition: None,
        from: vec![77, 78],
        to: vec![79],
        page: 9,
        x: 1,
        y: 1,
    });
    let mut project = Project::new("hostile");
    project.sections.push(Section::new(1, "Main"));
    project.sections.push(Section::sfc(2, "Broken", page));
    let mut sub = Section::new(3, "Empty");
    sub.language = SectionLanguage::Sfc;
    project.sections.push(sub);

    let mut editor = Editor::new(project);
    assert!(
        editor.problems().iter().any(|d| d.code == "SL-E011"),
        "the dangling references are reported: {:?}",
        editor.problems()
    );

    // Every command is either applied or refused, and none of them panics.
    let commands = [
        Command::InsertStep {
            section: 3,
            step: Step::new(0, 0),
        },
        Command::RemoveStep {
            section: 2,
            step: 4,
        },
        Command::MoveStep {
            section: 2,
            step: 4,
            x: 5,
            y: 5,
        },
        Command::SetStepNumber {
            section: 2,
            step: 4,
            number: 0,
        },
        Command::SetStepInitial {
            section: 2,
            step: 4,
            initial: true,
        },
        Command::InsertTransition {
            section: 2,
            transition: Transition::new(5, 3),
        },
        Command::RemoveTransition {
            section: 2,
            transition: 2,
        },
        Command::MoveTransition {
            section: 2,
            transition: 2,
            x: 2,
            y: 2,
        },
        Command::SetTransitionCondition {
            section: 2,
            transition: 2,
            condition: Some("%I0".to_owned()),
        },
        Command::SetTransitionFrom {
            section: 2,
            transition: 2,
            from: vec![4],
        },
        Command::SetTransitionTo {
            section: 2,
            transition: 2,
            to: vec![4],
        },
        Command::SetPageComment {
            section: 2,
            comment: "x".to_owned(),
        },
        Command::AddPage {
            section: 2,
            page: SequentialPage::new(0, ""),
        },
        Command::RemovePage { section: 3 },
    ];
    for command in &commands {
        let _ = editor.apply(command.clone());
    }
    // Rewinding and replaying a hostile history is defensive too.
    while editor.undo() {}
    while editor.redo() {}
    while editor.undo() {}

    // An empty project, and a section that does not exist.
    let mut empty = Editor::new(Project::new("nothing"));
    let _ = empty.remove_page(1);
    let _ = empty.set_page_comment(1, "x");
    let _ = empty.insert_step(1, 0, 0, true);
    let _ = empty.remove_step(1, 0);
    let _ = empty.move_step(1, 0, 1, 1);
    let _ = empty.link_transition_to(1, 0, 0, true);
    assert!(!empty.undo());
    assert!(empty.problems().is_empty());
}

#[test]
fn an_sfc_session_undoes_and_redoes_exactly() {
    let mut editor = sfc_editor();
    let original = editor.project().clone();
    editor.insert_step(2, 2, 0, true).expect("inserts a step");
    editor.insert_step(2, 1, 4, false).expect("inserts another");
    editor
        .insert_transition_linked(2, 2, 2, &[0, 1], &[3])
        .expect("inserts an AND divergence");
    editor
        .set_transition_condition(2, 1, "%M0 AND NOT %M1")
        .expect("sets a condition");
    editor
        .set_page_comment(2, "two branches")
        .expect("comments the page");
    editor.move_step(2, 3, 3, 6).expect("moves a step");
    editor.remove_step(2, 3).expect("removes it again");
    editor
        .set_transition_from(2, 1, &[0])
        .expect("narrows the source set");
    let edited = editor.project().clone();
    assert_ne!(edited, original);
    let commands = editor.history_len();

    let mut undone = 0;
    while editor.undo() {
        undone += 1;
    }
    assert_eq!(undone, commands);
    assert_eq!(editor.project(), &original, "undo-all is exact");

    let mut redone = 0;
    while editor.redo() {
        redone += 1;
    }
    assert_eq!(redone, commands);
    assert_eq!(editor.project(), &edited, "redo-all is exact");
}
