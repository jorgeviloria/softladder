//! The command vocabulary the editor understands.
//!
//! Every mutation is a [`Command`]; this is what makes the editor scriptable,
//! undoable and testable without a UI. [`Command::label`] feeds the UI's
//! "Undo <label>" / "Redo <label>" menu entries.

use softladder_core::{
    ElementKind, PlacedElement, Rung, ScanConfig, Section, SequentialPage, SimulationPanel, Step,
    Symbol, Transition, VarRef,
};

/// A single reversible edit.
///
/// The variants carry plain data only: a command never holds a mutable borrow
/// of the project, so it can be built by the UI, logged, or replayed later.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// Places `element` on `rung`; rejected when the cell is occupied.
    PlaceElement {
        /// Id of the rung to edit.
        rung: u32,
        /// Element to place (its `col`/`row` say where).
        element: PlacedElement,
    },
    /// Puts `element` on `rung`, replacing whatever sat on that cell.
    ///
    /// This is what the editor uses when a palette element is dropped on a cell
    /// that already holds one: it is a single undo step, unlike deleting and
    /// placing again. Placing on an empty cell behaves like
    /// [`Command::PlaceElement`].
    ReplaceElement {
        /// Id of the rung to edit.
        rung: u32,
        /// Element to put in place (its `col`/`row` say where).
        element: PlacedElement,
    },
    /// Deletes the element at `(col, row)`.
    RemoveElement {
        /// Id of the rung to edit.
        rung: u32,
        /// Column of the cell to clear.
        col: u8,
        /// Row of the cell to clear.
        row: u8,
    },
    /// Moves the element at `(from_col, from_row)` to `(to_col, to_row)`.
    MoveElement {
        /// Id of the rung to edit.
        rung: u32,
        /// Column the element currently sits in.
        from_col: u8,
        /// Row the element currently sits in.
        from_row: u8,
        /// Column to move it to.
        to_col: u8,
        /// Row to move it to.
        to_row: u8,
    },
    /// Binds (or unbinds) the element's variable.
    SetElementVar {
        /// Id of the rung to edit.
        rung: u32,
        /// Column of the element.
        col: u8,
        /// Row of the element.
        row: u8,
        /// New variable, or `None` to unbind.
        var: Option<VarRef>,
    },
    /// Replaces the element's free-form parameter list.
    SetElementParams {
        /// Id of the rung to edit.
        rung: u32,
        /// Column of the element.
        col: u8,
        /// Row of the element.
        row: u8,
        /// New parameters, in order.
        params: Vec<String>,
    },
    /// Changes what the element is, keeping its variable and parameters.
    SetElementKind {
        /// Id of the rung to edit.
        rung: u32,
        /// Column of the element.
        col: u8,
        /// Row of the element.
        row: u8,
        /// New element kind.
        kind: ElementKind,
    },
    /// Sets the cell's vertical link to the cell above it.
    SetVerticalLink {
        /// Id of the rung to edit.
        rung: u32,
        /// Column of the element.
        col: u8,
        /// Row of the element.
        row: u8,
        /// New state of `connected_with_top`.
        linked: bool,
    },
    /// Replaces the rung's label and comment.
    SetRungText {
        /// Id of the rung to edit.
        rung: u32,
        /// New label (a jump target).
        label: String,
        /// New comment.
        comment: String,
    },
    /// Inserts a caller-provided rung at `index` in a section.
    InsertRung {
        /// Id of the section to edit.
        section: u32,
        /// Position in `Section::rungs` (may equal the current length).
        index: usize,
        /// The rung to insert; its id must be unused.
        rung: Rung,
    },
    /// Removes a rung from a section (and from the project pool when unshared).
    DeleteRung {
        /// Id of the section to edit.
        section: u32,
        /// Id of the rung to remove.
        rung: u32,
    },
    /// Re-orders a section's rungs.
    MoveRung {
        /// Id of the section to edit.
        section: u32,
        /// Current position of the rung.
        from: usize,
        /// Position it should end up at.
        to: usize,
    },
    /// Appends a caller-provided section.
    AddSection {
        /// The section to add; its id must be unused.
        section: Section,
    },
    /// Removes a section and the pool rungs it owned exclusively.
    RemoveSection {
        /// Id of the section to remove.
        section: u32,
    },
    /// Renames a section.
    SetSectionName {
        /// Id of the section to rename.
        section: u32,
        /// New display name.
        name: String,
    },
    /// Replaces the whole symbol table.
    SetSymbols {
        /// New symbol table.
        symbols: Vec<Symbol>,
    },
    /// Replaces the simulation panel (the bench layout).
    SetPanel {
        /// New panel.
        panel: SimulationPanel,
    },
    /// Replaces the scan timing configuration.
    SetScanConfig {
        /// New configuration.
        scan: ScanConfig,
    },
    /// Inserts a caller-provided step into a section's sequential page.
    ///
    /// The step carries its own page, number and `(x, y)` cell; whatever already
    /// occupies that cell is replaced, so palette placement is a single undo
    /// step, exactly like [`Command::ReplaceElement`] on the ladder.
    InsertStep {
        /// Id of the section to edit.
        section: u32,
        /// The step to insert; its number must be unused.
        step: Step,
    },
    /// Removes a step from a section's sequential page.
    ///
    /// Every transition that named the step in its `from` or `to` set is pruned
    /// with it, so removing a step never leaves a dangling reference behind.
    RemoveStep {
        /// Id of the section to edit.
        section: u32,
        /// Number of the step to remove.
        step: u32,
    },
    /// Moves a step to another cell of its page.
    MoveStep {
        /// Id of the section to edit.
        section: u32,
        /// Number of the step to move.
        step: u32,
        /// New X coordinate.
        x: i32,
        /// New Y coordinate.
        y: i32,
    },
    /// Renumbers a step, and every reference the transitions make to it.
    SetStepNumber {
        /// Id of the section to edit.
        section: u32,
        /// Current number of the step.
        step: u32,
        /// Number it should have.
        number: u32,
    },
    /// Sets whether a step is active at start-up.
    SetStepInitial {
        /// Id of the section to edit.
        section: u32,
        /// Number of the step.
        step: u32,
        /// New value of the step's initial flag.
        initial: bool,
    },
    /// Inserts a caller-provided transition into a section's sequential page.
    ///
    /// The caller fills in `from` and `to`, which is how a placement that also
    /// links the surrounding steps stays a single undo step.
    InsertTransition {
        /// Id of the section to edit.
        section: u32,
        /// The transition to insert; its number must be unused.
        transition: Transition,
    },
    /// Removes a transition from a section's sequential page.
    RemoveTransition {
        /// Id of the section to edit.
        section: u32,
        /// Number of the transition to remove.
        transition: u32,
    },
    /// Moves a transition to another cell of its page.
    MoveTransition {
        /// Id of the section to edit.
        section: u32,
        /// Number of the transition to move.
        transition: u32,
        /// New X coordinate.
        x: i32,
        /// New Y coordinate.
        y: i32,
    },
    /// Sets or clears a transition's firing condition.
    ///
    /// The text is parsed with `softladder_core::parse`; `None` and a blank
    /// string both clear the condition, which makes the transition fire whenever
    /// all of its source steps are active. Text that does not parse is refused
    /// with [`EditError::BadCondition`](crate::EditError::BadCondition) before
    /// anything is changed.
    SetTransitionCondition {
        /// Id of the section to edit.
        section: u32,
        /// Number of the transition.
        transition: u32,
        /// Expression source, or `None` to clear it.
        condition: Option<String>,
    },
    /// Replaces the steps a transition requires to be active before it fires.
    SetTransitionFrom {
        /// Id of the section to edit.
        section: u32,
        /// Number of the transition.
        transition: u32,
        /// Step numbers; every one of them must be active. Sorted and deduplicated.
        from: Vec<u32>,
    },
    /// Replaces the steps a transition activates when it fires.
    SetTransitionTo {
        /// Id of the section to edit.
        section: u32,
        /// Number of the transition.
        transition: u32,
        /// Step numbers; all of them are activated together. Sorted and deduplicated.
        to: Vec<u32>,
    },
    /// Replaces an SFC page's comment.
    SetPageComment {
        /// Id of the section to edit.
        section: u32,
        /// New comment.
        comment: String,
    },
    /// Gives an SFC section its page.
    AddPage {
        /// Id of the section to edit.
        section: u32,
        /// The page to install; its steps and transitions are taken as they are.
        page: SequentialPage,
    },
    /// Removes an SFC section's page, and every step and transition it held.
    RemovePage {
        /// Id of the section to edit.
        section: u32,
    },
}

impl Command {
    /// Short human-readable label, used by the undo/redo menu entries.
    pub fn label(&self) -> &'static str {
        match self {
            Command::PlaceElement { .. } => "place element",
            Command::ReplaceElement { .. } => "replace element",
            Command::RemoveElement { .. } => "delete element",
            Command::MoveElement { .. } => "move element",
            Command::SetElementVar { .. } => "set element variable",
            Command::SetElementParams { .. } => "set element parameters",
            Command::SetElementKind { .. } => "change element kind",
            Command::SetVerticalLink { .. } => "set vertical link",
            Command::SetRungText { .. } => "edit rung text",
            Command::InsertRung { .. } => "insert rung",
            Command::DeleteRung { .. } => "delete rung",
            Command::MoveRung { .. } => "move rung",
            Command::AddSection { .. } => "add section",
            Command::RemoveSection { .. } => "remove section",
            Command::SetSectionName { .. } => "rename section",
            Command::SetSymbols { .. } => "set symbols",
            Command::SetPanel { .. } => "set simulation panel",
            Command::SetScanConfig { .. } => "set scan configuration",
            Command::InsertStep { .. } => "insert step",
            Command::RemoveStep { .. } => "delete step",
            Command::MoveStep { .. } => "move step",
            Command::SetStepNumber { .. } => "renumber step",
            Command::SetStepInitial { .. } => "set the initial step",
            Command::InsertTransition { .. } => "insert transition",
            Command::RemoveTransition { .. } => "delete transition",
            Command::MoveTransition { .. } => "move transition",
            Command::SetTransitionCondition { .. } => "set the transition condition",
            Command::SetTransitionFrom { .. } => "set the transition sources",
            Command::SetTransitionTo { .. } => "set the transition targets",
            Command::SetPageComment { .. } => "edit the page comment",
            Command::AddPage { .. } => "add a page",
            Command::RemovePage { .. } => "remove the page",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use softladder_core::{SimulationPanel, TimerMode};

    fn label(command: &Command) -> &'static str {
        command.label()
    }

    #[test]
    fn every_command_has_a_stable_label() {
        let commands = [
            (
                Command::PlaceElement {
                    rung: 1,
                    element: PlacedElement::new(ElementKind::ContactNo, 0, 0),
                },
                "place element",
            ),
            (
                Command::RemoveElement {
                    rung: 1,
                    col: 0,
                    row: 0,
                },
                "delete element",
            ),
            (
                Command::MoveElement {
                    rung: 1,
                    from_col: 0,
                    from_row: 0,
                    to_col: 1,
                    to_row: 0,
                },
                "move element",
            ),
            (
                Command::SetElementVar {
                    rung: 1,
                    col: 0,
                    row: 0,
                    var: None,
                },
                "set element variable",
            ),
            (
                Command::SetElementParams {
                    rung: 1,
                    col: 0,
                    row: 0,
                    params: vec!["3000".to_owned()],
                },
                "set element parameters",
            ),
            (
                Command::SetElementKind {
                    rung: 1,
                    col: 0,
                    row: 0,
                    kind: ElementKind::Timer {
                        mode: TimerMode::On,
                    },
                },
                "change element kind",
            ),
            (
                Command::SetVerticalLink {
                    rung: 1,
                    col: 0,
                    row: 1,
                    linked: true,
                },
                "set vertical link",
            ),
            (
                Command::SetRungText {
                    rung: 1,
                    label: String::new(),
                    comment: String::new(),
                },
                "edit rung text",
            ),
            (
                Command::InsertRung {
                    section: 1,
                    index: 0,
                    rung: Rung::new(9),
                },
                "insert rung",
            ),
            (
                Command::DeleteRung {
                    section: 1,
                    rung: 1,
                },
                "delete rung",
            ),
            (
                Command::MoveRung {
                    section: 1,
                    from: 0,
                    to: 1,
                },
                "move rung",
            ),
            (
                Command::AddSection {
                    section: Section::new(5, "new"),
                },
                "add section",
            ),
            (Command::RemoveSection { section: 1 }, "remove section"),
            (
                Command::SetSectionName {
                    section: 1,
                    name: "renamed".to_owned(),
                },
                "rename section",
            ),
            (
                Command::SetSymbols {
                    symbols: Vec::new(),
                },
                "set symbols",
            ),
            (
                Command::SetPanel {
                    panel: SimulationPanel::default(),
                },
                "set simulation panel",
            ),
            (
                Command::SetScanConfig {
                    scan: ScanConfig::default(),
                },
                "set scan configuration",
            ),
            (
                Command::InsertStep {
                    section: 1,
                    step: Step::new(1, 0),
                },
                "insert step",
            ),
            (
                Command::RemoveStep {
                    section: 1,
                    step: 1,
                },
                "delete step",
            ),
            (
                Command::MoveStep {
                    section: 1,
                    step: 1,
                    x: 2,
                    y: 3,
                },
                "move step",
            ),
            (
                Command::SetStepNumber {
                    section: 1,
                    step: 1,
                    number: 4,
                },
                "renumber step",
            ),
            (
                Command::SetStepInitial {
                    section: 1,
                    step: 1,
                    initial: true,
                },
                "set the initial step",
            ),
            (
                Command::InsertTransition {
                    section: 1,
                    transition: Transition::new(1, 0),
                },
                "insert transition",
            ),
            (
                Command::RemoveTransition {
                    section: 1,
                    transition: 1,
                },
                "delete transition",
            ),
            (
                Command::MoveTransition {
                    section: 1,
                    transition: 1,
                    x: 0,
                    y: 1,
                },
                "move transition",
            ),
            (
                Command::SetTransitionCondition {
                    section: 1,
                    transition: 1,
                    condition: None,
                },
                "set the transition condition",
            ),
            (
                Command::SetTransitionFrom {
                    section: 1,
                    transition: 1,
                    from: vec![1],
                },
                "set the transition sources",
            ),
            (
                Command::SetTransitionTo {
                    section: 1,
                    transition: 1,
                    to: vec![2],
                },
                "set the transition targets",
            ),
            (
                Command::SetPageComment {
                    section: 1,
                    comment: "loop".to_owned(),
                },
                "edit the page comment",
            ),
            (
                Command::AddPage {
                    section: 1,
                    page: SequentialPage::new(0, "main"),
                },
                "add a page",
            ),
            (Command::RemovePage { section: 1 }, "remove the page"),
        ];

        for (command, expected) in &commands {
            assert_eq!(label(command), *expected);
            assert!(!expected.is_empty());
        }
        assert_eq!(commands.len(), 31, "one label per command variant");
    }
}
