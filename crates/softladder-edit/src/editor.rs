//! The authoritative editing state: project, file, history and diagnostics.
//!
//! [`Editor`] owns the one true [`Project`]. The UI never mutates it directly:
//! it emits a [`Command`], [`Editor::apply`] validates and executes it, and the
//! command is recorded as a partial snapshot for undo. A rejected command
//! leaves the project byte-identical and records nothing.

use std::path::{Path, PathBuf};

use softladder_core::{
    lint, Diagnostic, ElementKind, Expr, PlacedElement, Project, Rung, ScanConfig, ScanEngine,
    Section, SectionLanguage, SequentialPage, Severity, SimulationPanel, Step, Symbol, Transition,
    VarRef,
};
use softladder_project::native;

use crate::command::Command;
use crate::error::EditError;
use crate::history::{Direction, Edit};

/// Maximum number of undo entries kept.
///
/// The oldest entry is dropped when the bound is reached, so a long editing
/// session has a bounded memory cost. `history_len` reports the number of
/// commands that can still be undone.
pub const MAX_HISTORY: usize = 1000;

/// One entry of the undo stack: the command label and its partial snapshot.
#[derive(Debug)]
struct Entry {
    label: &'static str,
    edit: Edit,
}

/// The editor's authoritative state.
///
/// It is a plain value with no interior mutability, no I/O of its own and no
/// dependency on a window, so an entire editing session can be driven from a
/// test or a script.
#[derive(Debug)]
pub struct Editor {
    project: Project,
    path: Option<PathBuf>,
    dirty: bool,
    undo_stack: Vec<Entry>,
    redo_stack: Vec<Entry>,
    diagnostics: Vec<Diagnostic>,
    problems: Vec<Diagnostic>,
}

impl Editor {
    /// Creates an editor around `project`, with no path and a clean dirty flag.
    pub fn new(project: Project) -> Self {
        let mut editor = Self {
            project,
            path: None,
            dirty: false,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            diagnostics: Vec::new(),
            problems: Vec::new(),
        };
        editor.refresh_diagnostics();
        editor
    }

    /// Opens a project file through `softladder_project::native`.
    ///
    /// Nothing is returned unless the file loaded completely, so a failed open
    /// never leaves a half-built editor behind.
    pub fn open(path: &Path) -> Result<Self, EditError> {
        let project = native::load(path)?;
        let mut editor = Self::new(project);
        editor.path = Some(path.to_path_buf());
        editor.dirty = false;
        Ok(editor)
    }

    /// Saves the project to `path` and makes `path` the current file.
    ///
    /// The dirty flag is cleared only when the write succeeded.
    pub fn save(&mut self, path: &Path) -> Result<(), EditError> {
        native::save(&self.project, path)?;
        self.path = Some(path.to_path_buf());
        self.dirty = false;
        Ok(())
    }

    /// Saves the project to its current file.
    ///
    /// Returns [`EditError::NoPath`] when the project has never been saved.
    pub fn save_current(&mut self) -> Result<(), EditError> {
        match self.path.clone() {
            Some(path) => self.save(&path),
            None => Err(EditError::NoPath),
        }
    }

    /// The file the project was last opened from or saved to.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Points the editor at a file (or at none), without touching the project.
    pub fn set_path(&mut self, path: Option<PathBuf>) {
        self.path = path;
    }

    /// The project being edited.
    pub fn project(&self) -> &Project {
        &self.project
    }

    /// `true` when there are edits that have not been saved.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Marks the project as saved, without writing anything.
    pub fn mark_clean(&mut self) {
        self.dirty = false;
    }

    /// Applies `command`, recording it for undo.
    ///
    /// On error the project is left untouched and no history entry is
    /// recorded, so callers may treat a rejection as a no-op.
    pub fn apply(&mut self, command: Command) -> Result<(), EditError> {
        let label = command.label();
        let edit = self.execute(&command)?;
        self.record(Entry { label, edit });
        Ok(())
    }

    /// Undoes the most recent command; returns `false` when there is none.
    pub fn undo(&mut self) -> bool {
        let Some(entry) = self.undo_stack.pop() else {
            return false;
        };
        entry.edit.replay(&mut self.project, Direction::Undo);
        self.redo_stack.push(entry);
        self.dirty = true;
        self.refresh_diagnostics();
        true
    }

    /// Redoes the most recently undone command; returns `false` when there is none.
    pub fn redo(&mut self) -> bool {
        let Some(entry) = self.redo_stack.pop() else {
            return false;
        };
        entry.edit.replay(&mut self.project, Direction::Redo);
        self.undo_stack.push(entry);
        self.dirty = true;
        self.refresh_diagnostics();
        true
    }

    /// `true` when there is a command to undo.
    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    /// `true` when there is an undone command to redo.
    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// Label of the command that [`Editor::undo`] would reverse.
    pub fn undo_label(&self) -> Option<&'static str> {
        self.undo_stack.last().map(|entry| entry.label)
    }

    /// Label of the command that [`Editor::redo`] would re-apply.
    pub fn redo_label(&self) -> Option<&'static str> {
        self.redo_stack.last().map(|entry| entry.label)
    }

    /// Number of commands that can still be undone.
    pub fn history_len(&self) -> usize {
        self.undo_stack.len()
    }

    /// Drops the whole undo and redo history, without touching the project.
    pub fn clear_history(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
    }

    /// Errors produced by the last [`Editor::refresh_diagnostics`] scan.
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Recomputes [`Editor::diagnostics`] and [`Editor::problems`].
    ///
    /// The scan runs on a **throwaway clone** of the project
    /// ([`ScanEngine::scan_once`] at `now_ms = 0`), so refreshing the editor's
    /// Problems list can never disturb a running [`crate::Bench`].
    pub fn refresh_diagnostics(&mut self) {
        let mut engine = ScanEngine::new(self.project.clone());
        let report = engine.scan_once(0);
        self.diagnostics = report
            .diagnostics
            .into_iter()
            .filter(|diagnostic| diagnostic.severity == Severity::Error)
            .collect();

        let mut problems = lint(&self.project);
        for message in self.project.simulation.validate() {
            push_unique(
                &mut problems,
                Diagnostic::new(Severity::Warning, "SL-W020", message),
            );
        }
        for diagnostic in &self.diagnostics {
            push_unique(&mut problems, diagnostic.clone());
        }
        self.problems = problems;
    }

    /// Diagnostics for the whole project, recomputed after every edit:
    /// structural problems from `softladder_core::lint`, one `SL-W020` warning
    /// per message from [`SimulationPanel::validate`], and the errors of the
    /// last scan performed by [`Editor::refresh_diagnostics`].
    pub fn problems(&self) -> &[Diagnostic] {
        &self.problems
    }

    // -- convenience wrappers used by the UI --------------------------------

    /// Puts an element on a cell, replacing whatever was there.
    ///
    /// Unlike [`Editor::place_element`] this never fails because the cell is
    /// occupied: dropping a palette element on top of another is a normal edit.
    pub fn replace_element(
        &mut self,
        rung: u32,
        kind: ElementKind,
        col: u8,
        row: u8,
        var: Option<VarRef>,
        params: &[&str],
    ) -> Result<(), EditError> {
        let mut element = PlacedElement::new(kind, col, row);
        element.var = var;
        element.params = params.iter().map(|param| (*param).to_owned()).collect();
        self.apply(Command::ReplaceElement { rung, element })
    }

    /// Places an element built from its parts; see [`Command::PlaceElement`].
    pub fn place_element(
        &mut self,
        rung: u32,
        kind: ElementKind,
        col: u8,
        row: u8,
        var: Option<VarRef>,
        params: &[&str],
    ) -> Result<(), EditError> {
        let element = PlacedElement {
            kind,
            var,
            col,
            row,
            connected_with_top: false,
            params: params.iter().map(|param| (*param).to_owned()).collect(),
        };
        self.apply(Command::PlaceElement { rung, element })
    }

    /// Deletes the element at `(col, row)`.
    pub fn delete_element(&mut self, rung: u32, col: u8, row: u8) -> Result<(), EditError> {
        self.apply(Command::RemoveElement { rung, col, row })
    }

    /// Binds (or unbinds) an element's variable.
    pub fn set_element_var(
        &mut self,
        rung: u32,
        col: u8,
        row: u8,
        var: Option<VarRef>,
    ) -> Result<(), EditError> {
        self.apply(Command::SetElementVar {
            rung,
            col,
            row,
            var,
        })
    }

    /// Replaces an element's parameter list.
    pub fn set_element_params(
        &mut self,
        rung: u32,
        col: u8,
        row: u8,
        params: &[&str],
    ) -> Result<(), EditError> {
        self.apply(Command::SetElementParams {
            rung,
            col,
            row,
            params: params.iter().map(|param| (*param).to_owned()).collect(),
        })
    }

    /// Sets or clears an element's vertical link to the cell above it.
    pub fn set_vertical_link(
        &mut self,
        rung: u32,
        col: u8,
        row: u8,
        linked: bool,
    ) -> Result<(), EditError> {
        self.apply(Command::SetVerticalLink {
            rung,
            col,
            row,
            linked,
        })
    }

    /// Moves an element from one cell to another.
    pub fn move_element(
        &mut self,
        rung: u32,
        from: (u8, u8),
        to: (u8, u8),
    ) -> Result<(), EditError> {
        self.apply(Command::MoveElement {
            rung,
            from_col: from.0,
            from_row: from.1,
            to_col: to.0,
            to_row: to.1,
        })
    }

    /// Inserts a fresh, empty rung at `index` in `section` and returns its id.
    ///
    /// The id is the largest existing rung id plus one (zero when the project
    /// has no rungs), so ids are never recycled by this call.
    pub fn insert_rung(&mut self, section: u32, index: usize) -> Result<u32, EditError> {
        let id = self.fresh_rung_id();
        self.apply(Command::InsertRung {
            section,
            index,
            rung: Rung::new(id),
        })?;
        Ok(id)
    }

    /// Deletes a rung from `section`.
    ///
    /// The rung is also dropped from `Project::rungs` unless another section
    /// still references it. Deleting the last rung of a section is allowed: the
    /// model and the file format both accept an empty section, so an editor can
    /// clear a program back to nothing (see the crate report for the reasoning).
    pub fn delete_rung(&mut self, section: u32, rung: u32) -> Result<(), EditError> {
        self.apply(Command::DeleteRung { section, rung })
    }

    /// Replaces a rung's label and comment.
    pub fn set_rung_text(
        &mut self,
        rung: u32,
        label: &str,
        comment: &str,
    ) -> Result<(), EditError> {
        self.apply(Command::SetRungText {
            rung,
            label: label.to_owned(),
            comment: comment.to_owned(),
        })
    }

    /// Appends a fresh, empty section and returns its id.
    ///
    /// The id is the largest existing section id plus one. The section starts
    /// with no rungs; add one with [`Editor::insert_rung`].
    pub fn add_section(&mut self, name: &str, language: SectionLanguage) -> Result<u32, EditError> {
        let id = self.fresh_section_id();
        let section = Section {
            id,
            name: name.to_owned(),
            language,
            subroutine: None,
            rungs: Vec::new(),
            sequential_page: None,
        };
        self.apply(Command::AddSection { section })?;
        Ok(id)
    }

    /// Removes a section and the rungs it owned exclusively.
    pub fn remove_section(&mut self, section: u32) -> Result<(), EditError> {
        self.apply(Command::RemoveSection { section })
    }

    /// Renames a section.
    pub fn rename_section(&mut self, section: u32, name: &str) -> Result<(), EditError> {
        self.apply(Command::SetSectionName {
            section,
            name: name.to_owned(),
        })
    }

    /// Replaces the whole symbol table.
    pub fn set_symbols(&mut self, symbols: Vec<Symbol>) -> Result<(), EditError> {
        self.apply(Command::SetSymbols { symbols })
    }

    /// Replaces the simulation panel.
    pub fn set_panel(&mut self, panel: SimulationPanel) -> Result<(), EditError> {
        self.apply(Command::SetPanel { panel })
    }

    /// Replaces the panel with [`SimulationPanel::auto_fill`] of the project.
    pub fn auto_fill_panel(&mut self) -> Result<(), EditError> {
        let panel = SimulationPanel::auto_fill(&self.project);
        self.set_panel(panel)
    }

    /// Replaces the scan timing configuration.
    pub fn set_scan_config(&mut self, scan: ScanConfig) -> Result<(), EditError> {
        self.apply(Command::SetScanConfig { scan })
    }

    // -- sequential (SFC) commands ------------------------------------------

    /// Gives `section` a fresh, empty page and returns its number.
    ///
    /// The number is the largest page number of the project plus one, so a new
    /// page never collides with one an imported chart already uses. The comment
    /// is empty; set it with [`Editor::set_page_comment`].
    pub fn add_page(&mut self, section: u32) -> Result<u32, EditError> {
        let number = self.fresh_page_number();
        self.apply(Command::AddPage {
            section,
            page: SequentialPage::new(number, String::new()),
        })?;
        Ok(number)
    }

    /// Removes `section`'s page, with every step and transition on it.
    pub fn remove_page(&mut self, section: u32) -> Result<(), EditError> {
        self.apply(Command::RemovePage { section })
    }

    /// Replaces the comment of `section`'s page.
    pub fn set_page_comment(&mut self, section: u32, comment: &str) -> Result<(), EditError> {
        self.apply(Command::SetPageComment {
            section,
            comment: comment.to_owned(),
        })
    }

    /// Inserts a fresh step at `(x, y)` of `section`'s page and returns its number.
    ///
    /// The number is the largest step number of the project plus one, because
    /// `%X<number>` is one variable shared by every page.
    pub fn insert_step(
        &mut self,
        section: u32,
        x: i32,
        y: i32,
        initial: bool,
    ) -> Result<u32, EditError> {
        let number = self.fresh_step_number();
        let page = self.page_number(section);
        let step = Step {
            number,
            is_initial: initial,
            x,
            y,
            page,
        };
        self.apply(Command::InsertStep { section, step })?;
        Ok(number)
    }

    /// Removes a step, and every reference the transitions made to it.
    pub fn remove_step(&mut self, section: u32, step: u32) -> Result<(), EditError> {
        self.apply(Command::RemoveStep { section, step })
    }

    /// Moves a step to another cell of its page.
    pub fn move_step(&mut self, section: u32, step: u32, x: i32, y: i32) -> Result<(), EditError> {
        self.apply(Command::MoveStep {
            section,
            step,
            x,
            y,
        })
    }

    /// Renumbers a step, following every transition reference with it.
    pub fn set_step_number(
        &mut self,
        section: u32,
        step: u32,
        number: u32,
    ) -> Result<(), EditError> {
        self.apply(Command::SetStepNumber {
            section,
            step,
            number,
        })
    }

    /// Sets whether a step is active at start-up.
    pub fn set_step_initial(
        &mut self,
        section: u32,
        step: u32,
        initial: bool,
    ) -> Result<(), EditError> {
        self.apply(Command::SetStepInitial {
            section,
            step,
            initial,
        })
    }

    /// Inserts a fresh, unconditional transition at `(x, y)` and returns its number.
    pub fn insert_transition(&mut self, section: u32, x: i32, y: i32) -> Result<u32, EditError> {
        self.insert_transition_linked(section, x, y, &[], &[])
    }

    /// Inserts a fresh transition at `(x, y)` with its source and target steps.
    ///
    /// This is what the AND and OR divergence tools place: the wiring is decided
    /// by the caller from the chart it can see and applied as one command, so a
    /// divergence is a single undo step.
    pub fn insert_transition_linked(
        &mut self,
        section: u32,
        x: i32,
        y: i32,
        from: &[u32],
        to: &[u32],
    ) -> Result<u32, EditError> {
        let number = self.fresh_transition_number();
        let page = self.page_number(section);
        let transition = Transition {
            number,
            condition: None,
            from: sorted_set(from),
            to: sorted_set(to),
            page,
            x,
            y,
        };
        self.apply(Command::InsertTransition {
            section,
            transition,
        })?;
        Ok(number)
    }

    /// Removes a transition from its page.
    pub fn remove_transition(&mut self, section: u32, transition: u32) -> Result<(), EditError> {
        self.apply(Command::RemoveTransition {
            section,
            transition,
        })
    }

    /// Moves a transition to another cell of its page.
    pub fn move_transition(
        &mut self,
        section: u32,
        transition: u32,
        x: i32,
        y: i32,
    ) -> Result<(), EditError> {
        self.apply(Command::MoveTransition {
            section,
            transition,
            x,
            y,
        })
    }

    /// Sets or clears a transition's condition from its source text.
    ///
    /// Blank text clears it; text that does not parse as an expression is refused
    /// with [`EditError::BadCondition`] and changes nothing.
    pub fn set_transition_condition(
        &mut self,
        section: u32,
        transition: u32,
        text: &str,
    ) -> Result<(), EditError> {
        let condition = if text.trim().is_empty() {
            None
        } else {
            Some(text.to_owned())
        };
        self.apply(Command::SetTransitionCondition {
            section,
            transition,
            condition,
        })
    }

    /// Replaces the steps a transition requires to be active.
    pub fn set_transition_from(
        &mut self,
        section: u32,
        transition: u32,
        from: &[u32],
    ) -> Result<(), EditError> {
        self.apply(Command::SetTransitionFrom {
            section,
            transition,
            from: from.to_vec(),
        })
    }

    /// Replaces the steps a transition activates.
    pub fn set_transition_to(
        &mut self,
        section: u32,
        transition: u32,
        to: &[u32],
    ) -> Result<(), EditError> {
        self.apply(Command::SetTransitionTo {
            section,
            transition,
            to: to.to_vec(),
        })
    }

    /// Adds `step` to (`linked`) or removes it from a transition's `from` set.
    ///
    /// This is the Link tool's edit: links are derived from the model's sets, so
    /// drawing a wire is exactly this membership change, and undoing it undraws
    /// the wire.
    pub fn link_transition_from(
        &mut self,
        section: u32,
        transition: u32,
        step: u32,
        linked: bool,
    ) -> Result<(), EditError> {
        let mut set = self.transition_of(section, transition)?.from;
        set.retain(|entry| *entry != step);
        if linked {
            set.push(step);
        }
        self.set_transition_from(section, transition, &set)
    }

    /// Adds `step` to (`linked`) or removes it from a transition's `to` set.
    pub fn link_transition_to(
        &mut self,
        section: u32,
        transition: u32,
        step: u32,
        linked: bool,
    ) -> Result<(), EditError> {
        let mut set = self.transition_of(section, transition)?.to;
        set.retain(|entry| *entry != step);
        if linked {
            set.push(step);
        }
        self.set_transition_to(section, transition, &set)
    }

    /// A copy of one transition of a section's page.
    fn transition_of(&self, section: u32, transition: u32) -> Result<Transition, EditError> {
        let position = self.require_section(section)?;
        self.project
            .sections
            .get(position)
            .and_then(|entry| entry.sequential_page.as_ref())
            .ok_or(EditError::NoSequentialPage(section))?
            .transition(transition)
            .cloned()
            .ok_or(EditError::UnknownTransition {
                section,
                transition,
            })
    }

    /// The number of a section's page, or zero when it has none.
    fn page_number(&self, section: u32) -> u32 {
        self.project
            .section(section)
            .and_then(|entry| entry.sequential_page.as_ref())
            .map_or(0, |page| page.number)
    }

    // -- internals ----------------------------------------------------------

    /// Pushes an entry, enforces the history bound and invalidates the redo
    /// branch.
    fn record(&mut self, entry: Entry) {
        self.undo_stack.push(entry);
        if self.undo_stack.len() > MAX_HISTORY {
            self.undo_stack.remove(0);
        }
        self.redo_stack.clear();
        self.dirty = true;
        self.refresh_diagnostics();
    }

    /// The largest existing rung id plus one, or zero for an empty project.
    fn fresh_rung_id(&self) -> u32 {
        self.project
            .rungs
            .iter()
            .map(|rung| rung.id)
            .max()
            .map_or(0, |id| id.saturating_add(1))
    }

    /// The largest existing section id plus one, or zero for an empty project.
    fn fresh_section_id(&self) -> u32 {
        self.project
            .sections
            .iter()
            .map(|section| section.id)
            .max()
            .map_or(0, |id| id.saturating_add(1))
    }

    /// The largest step number any page uses, plus one.
    ///
    /// Step numbers index the engine's one `%X` array, so they are unique across
    /// the project rather than inside a page.
    fn fresh_step_number(&self) -> u32 {
        self.pages()
            .flat_map(|page| page.steps.iter())
            .map(|step| step.number)
            .max()
            .map_or(0, |number| number.saturating_add(1))
    }

    /// The largest transition number any page uses, plus one.
    fn fresh_transition_number(&self) -> u32 {
        self.pages()
            .flat_map(|page| page.transitions.iter())
            .map(|transition| transition.number)
            .max()
            .map_or(0, |number| number.saturating_add(1))
    }

    /// The largest page number the project uses, plus one.
    fn fresh_page_number(&self) -> u32 {
        self.pages()
            .map(|page| page.number)
            .max()
            .map_or(0, |number| number.saturating_add(1))
    }

    /// Every sequential page of the project, in section order.
    fn pages(&self) -> impl Iterator<Item = &SequentialPage> {
        self.project
            .sections
            .iter()
            .filter_map(|section| section.sequential_page.as_ref())
    }

    /// Position of a rung in `Project::rungs`.
    fn rung_index(&self, id: u32) -> Option<usize> {
        self.project.rungs.iter().position(|rung| rung.id == id)
    }

    /// Position of a section in `Project::sections`.
    fn section_index(&self, id: u32) -> Option<usize> {
        self.project
            .sections
            .iter()
            .position(|section| section.id == id)
    }

    /// Validates a rung id and returns its pool position.
    fn require_rung(&self, id: u32) -> Result<usize, EditError> {
        self.rung_index(id).ok_or(EditError::UnknownRung(id))
    }

    /// Validates a section id and returns its position.
    fn require_section(&self, id: u32) -> Result<usize, EditError> {
        self.section_index(id).ok_or(EditError::UnknownSection(id))
    }

    /// Validates that a cell holds an element.
    fn require_element(&self, rung: u32, col: u8, row: u8) -> Result<(), EditError> {
        let index = self.require_rung(rung)?;
        let exists = self.project.rungs.get(index).is_some_and(|target| {
            target
                .elements
                .iter()
                .any(|element| element.col == col && element.row == row)
        });
        if exists {
            Ok(())
        } else {
            Err(EditError::NoSuchCell { rung, col, row })
        }
    }

    /// Validates that a cell is free.
    fn require_free(&self, rung: u32, col: u8, row: u8) -> Result<(), EditError> {
        let index = self.require_rung(rung)?;
        let occupied = self.project.rungs.get(index).is_some_and(|target| {
            target
                .elements
                .iter()
                .any(|element| element.col == col && element.row == row)
        });
        if occupied {
            Err(EditError::Occupied { rung, col, row })
        } else {
            Ok(())
        }
    }

    /// Applies `mutate` to the element at `(col, row)` and snapshots its rung.
    fn edit_element(
        &mut self,
        rung: u32,
        col: u8,
        row: u8,
        mutate: impl FnOnce(&mut PlacedElement),
    ) -> Result<Edit, EditError> {
        let index = self.require_rung(rung)?;
        self.require_element(rung, col, row)?;
        let target = self
            .project
            .rungs
            .get_mut(index)
            .ok_or(EditError::UnknownRung(rung))?;
        let before = target.clone();
        if let Some(element) = target
            .elements
            .iter_mut()
            .find(|element| element.col == col && element.row == row)
        {
            mutate(element);
        }
        Ok(Edit::Rung {
            id: rung,
            before,
            after: target.clone(),
        })
    }

    /// Validates and executes a command, returning its partial snapshot.
    ///
    /// Validation always happens before the first mutation, so a rejected
    /// command cannot leave a half-applied project behind.
    fn execute(&mut self, command: &Command) -> Result<Edit, EditError> {
        match command {
            Command::PlaceElement { rung, element } => {
                self.require_free(*rung, element.col, element.row)?;
                let index = self.require_rung(*rung)?;
                let target = self
                    .project
                    .rungs
                    .get_mut(index)
                    .ok_or(EditError::UnknownRung(*rung))?;
                let before = target.clone();
                target.elements.push(element.clone());
                sort_elements(target);
                Ok(Edit::Rung {
                    id: *rung,
                    before,
                    after: target.clone(),
                })
            }
            Command::ReplaceElement { rung, element } => {
                let index = self.require_rung(*rung)?;
                let target = self
                    .project
                    .rungs
                    .get_mut(index)
                    .ok_or(EditError::UnknownRung(*rung))?;
                let before = target.clone();
                target.elements.retain(|existing| {
                    !(existing.col == element.col && existing.row == element.row)
                });
                target.elements.push(element.clone());
                sort_elements(target);
                Ok(Edit::Rung {
                    id: *rung,
                    before,
                    after: target.clone(),
                })
            }
            Command::RemoveElement { rung, col, row } => {
                let index = self.require_rung(*rung)?;
                self.require_element(*rung, *col, *row)?;
                let target = self
                    .project
                    .rungs
                    .get_mut(index)
                    .ok_or(EditError::UnknownRung(*rung))?;
                let before = target.clone();
                target
                    .elements
                    .retain(|element| !(element.col == *col && element.row == *row));
                Ok(Edit::Rung {
                    id: *rung,
                    before,
                    after: target.clone(),
                })
            }
            Command::MoveElement {
                rung,
                from_col,
                from_row,
                to_col,
                to_row,
            } => {
                let index = self.require_rung(*rung)?;
                self.require_element(*rung, *from_col, *from_row)?;
                if from_col == to_col && from_row == to_row {
                    // Dropping a cell onto itself is not a move; the target is
                    // occupied by the element being moved.
                    return Err(EditError::Occupied {
                        rung: *rung,
                        col: *to_col,
                        row: *to_row,
                    });
                }
                self.require_free(*rung, *to_col, *to_row)?;
                let target = self
                    .project
                    .rungs
                    .get_mut(index)
                    .ok_or(EditError::UnknownRung(*rung))?;
                let before = target.clone();
                for element in &mut target.elements {
                    if element.col == *from_col && element.row == *from_row {
                        element.col = *to_col;
                        element.row = *to_row;
                    }
                }
                sort_elements(target);
                Ok(Edit::Rung {
                    id: *rung,
                    before,
                    after: target.clone(),
                })
            }
            Command::SetElementVar {
                rung,
                col,
                row,
                var,
            } => self.edit_element(*rung, *col, *row, |element| element.var = var.clone()),
            Command::SetElementParams {
                rung,
                col,
                row,
                params,
            } => self.edit_element(*rung, *col, *row, |element| {
                element.params = params.clone();
            }),
            Command::SetElementKind {
                rung,
                col,
                row,
                kind,
            } => self.edit_element(*rung, *col, *row, |element| element.kind = *kind),
            Command::SetVerticalLink {
                rung,
                col,
                row,
                linked,
            } => self.edit_element(*rung, *col, *row, |element| {
                element.connected_with_top = *linked;
            }),
            Command::SetRungText {
                rung,
                label,
                comment,
            } => {
                let index = self.require_rung(*rung)?;
                let target = self
                    .project
                    .rungs
                    .get_mut(index)
                    .ok_or(EditError::UnknownRung(*rung))?;
                let before = target.clone();
                target.label = label.clone();
                target.comment = comment.clone();
                Ok(Edit::Rung {
                    id: *rung,
                    before,
                    after: target.clone(),
                })
            }
            Command::InsertRung {
                section,
                index,
                rung,
            } => {
                let position = self.require_section(*section)?;
                if self
                    .project
                    .rungs
                    .iter()
                    .any(|existing| existing.id == rung.id)
                {
                    return Err(EditError::DuplicateRung(rung.id));
                }
                let len = self
                    .project
                    .sections
                    .get(position)
                    .map_or(0, |target| target.rungs.len());
                if *index > len {
                    return Err(EditError::IndexOutOfRange { index: *index, len });
                }
                let pool_index = self.project.rungs.len();
                if let Some(target) = self.project.sections.get_mut(position) {
                    target.rungs.insert(*index, rung.id);
                }
                self.project.rungs.push(rung.clone());
                Ok(Edit::InsertRung {
                    section: *section,
                    index: *index,
                    pool_index,
                    rung: rung.clone(),
                })
            }
            Command::DeleteRung { section, rung } => {
                let position = self.require_section(*section)?;
                let index = self
                    .project
                    .sections
                    .get(position)
                    .and_then(|target| target.rungs.iter().position(|id| id == rung))
                    .ok_or(EditError::UnknownRung(*rung))?;
                let pool_index = self.rung_index(*rung);
                let Some(removed) = pool_index
                    .and_then(|at| self.project.rungs.get(at))
                    .cloned()
                else {
                    return Err(EditError::UnknownRung(*rung));
                };
                if let Some(target) = self.project.sections.get_mut(position) {
                    target.rungs.remove(index);
                }
                // A rung shared by another section stays in the flat pool.
                let shared = self
                    .project
                    .sections
                    .iter()
                    .any(|target| target.rungs.contains(rung));
                let pool_index = match (pool_index, shared) {
                    (Some(at), false) => {
                        self.project.rungs.remove(at);
                        Some(at)
                    }
                    _ => None,
                };
                Ok(Edit::RemoveRung {
                    section: *section,
                    index,
                    rung: removed,
                    pool_index,
                })
            }
            Command::MoveRung { section, from, to } => {
                let position = self.require_section(*section)?;
                let len = self
                    .project
                    .sections
                    .get(position)
                    .map_or(0, |target| target.rungs.len());
                if *from >= len {
                    return Err(EditError::IndexOutOfRange { index: *from, len });
                }
                if *to >= len {
                    return Err(EditError::IndexOutOfRange { index: *to, len });
                }
                let target = self
                    .project
                    .sections
                    .get_mut(position)
                    .ok_or(EditError::UnknownSection(*section))?;
                let before = target.rungs.clone();
                let id = target.rungs.remove(*from);
                target.rungs.insert(*to, id);
                let after = target.rungs.clone();
                Ok(Edit::MoveRung {
                    section: *section,
                    before,
                    after,
                })
            }
            Command::AddSection { section } => {
                if self
                    .project
                    .sections
                    .iter()
                    .any(|existing| existing.id == section.id)
                {
                    return Err(EditError::DuplicateSection(section.id));
                }
                let index = self.project.sections.len();
                self.project.sections.push(section.clone());
                Ok(Edit::InsertSection {
                    index,
                    section: section.clone(),
                })
            }
            Command::RemoveSection { section } => {
                let position = self.require_section(*section)?;
                let Some(removed) = self.project.sections.get(position).cloned() else {
                    return Err(EditError::UnknownSection(*section));
                };
                self.project.sections.remove(position);

                // Collect the rungs this section owned exclusively, recording
                // their original pool positions, then drop them from the end so
                // the earlier positions stay valid.
                let mut rungs: Vec<(usize, Rung)> = Vec::new();
                for id in &removed.rungs {
                    if self
                        .project
                        .sections
                        .iter()
                        .any(|target| target.rungs.contains(id))
                    {
                        continue;
                    }
                    if let Some(at) = self.rung_index(*id) {
                        if let Some(rung) = self.project.rungs.get(at).cloned() {
                            rungs.push((at, rung));
                        }
                    }
                }
                rungs.sort_by_key(|(at, _)| *at);
                for (at, _) in rungs.iter().rev() {
                    self.project.rungs.remove(*at);
                }
                Ok(Edit::RemoveSection {
                    index: position,
                    section: removed,
                    rungs,
                })
            }
            Command::SetSectionName { section, name } => {
                let position = self.require_section(*section)?;
                let target = self
                    .project
                    .sections
                    .get_mut(position)
                    .ok_or(EditError::UnknownSection(*section))?;
                let before = target.name.clone();
                target.name = name.clone();
                Ok(Edit::SectionName {
                    id: *section,
                    before,
                    after: target.name.clone(),
                })
            }
            Command::SetSymbols { symbols } => {
                let before = std::mem::replace(&mut self.project.symbols, symbols.clone());
                Ok(Edit::Symbols {
                    before,
                    after: symbols.clone(),
                })
            }
            Command::SetPanel { panel } => {
                let before = std::mem::replace(&mut self.project.simulation, panel.clone());
                Ok(Edit::Panel {
                    before,
                    after: panel.clone(),
                })
            }
            Command::SetScanConfig { scan } => {
                let before = self.project.scan;
                self.project.scan = *scan;
                Ok(Edit::Scan {
                    before,
                    after: *scan,
                })
            }
            Command::InsertStep { section, step } => {
                let position = self.require_section(*section)?;
                if self.step_number_taken(*section, step.number) {
                    return Err(EditError::DuplicateStep(step.number));
                }
                let page = self.page_at(position, *section)?;
                let replacing = page
                    .step(step.number)
                    .is_some_and(|existing| existing.x == step.x && existing.y == step.y);
                if page.step(step.number).is_some() && !replacing {
                    return Err(EditError::DuplicateStep(step.number));
                }
                let before = page.clone();
                // Placement clears the cell it lands on, as one undo step; the
                // numbers of the steps it displaced are pruned from the chart.
                let displaced = clear_cell(page, step.x, step.y, Some(step.number));
                for number in displaced {
                    prune_step(page, number);
                }
                page.steps.push(step.clone());
                sort_page(page);
                let after = page.clone();
                Ok(Edit::SfcPage {
                    section: *section,
                    before: Some(before),
                    after: Some(after),
                })
            }
            Command::RemoveStep { section, step } => {
                let position = self.require_section(*section)?;
                let page = self.page_at(position, *section)?;
                let at = page
                    .steps
                    .iter()
                    .position(|entry| entry.number == *step)
                    .ok_or(EditError::UnknownStep {
                        section: *section,
                        step: *step,
                    })?;
                let before = page.clone();
                page.steps.remove(at);
                prune_step(page, *step);
                let after = page.clone();
                Ok(Edit::SfcPage {
                    section: *section,
                    before: Some(before),
                    after: Some(after),
                })
            }
            Command::MoveStep {
                section,
                step,
                x,
                y,
            } => {
                let position = self.require_section(*section)?;
                let page = self.page_at(position, *section)?;
                let at = page
                    .steps
                    .iter()
                    .position(|entry| entry.number == *step)
                    .ok_or(EditError::UnknownStep {
                        section: *section,
                        step: *step,
                    })?;
                let Some(before) = page.steps.get(at).cloned() else {
                    return Err(EditError::UnknownStep {
                        section: *section,
                        step: *step,
                    });
                };
                if (before.x == *x && before.y == *y) || cell_occupied(page, *x, *y) {
                    return Err(EditError::SfcCellOccupied {
                        section: *section,
                        x: *x,
                        y: *y,
                    });
                }
                if let Some(slot) = page.steps.get_mut(at) {
                    slot.x = *x;
                    slot.y = *y;
                }
                let Some(after) = page.steps.get(at).cloned() else {
                    return Err(EditError::UnknownStep {
                        section: *section,
                        step: *step,
                    });
                };
                Ok(Edit::SfcStep {
                    section: *section,
                    index: at,
                    before,
                    after,
                })
            }
            Command::SetStepNumber {
                section,
                step,
                number,
            } => {
                let position = self.require_section(*section)?;
                if self.step_number_taken(*section, *number) {
                    return Err(EditError::DuplicateStep(*number));
                }
                let page = self.page_at(position, *section)?;
                let at = page
                    .steps
                    .iter()
                    .position(|entry| entry.number == *step)
                    .ok_or(EditError::UnknownStep {
                        section: *section,
                        step: *step,
                    })?;
                if *number != *step && page.step(*number).is_some() {
                    return Err(EditError::DuplicateStep(*number));
                }
                let before = page.clone();
                if let Some(slot) = page.steps.get_mut(at) {
                    slot.number = *number;
                }
                // The number is the index of `%X<n>`, so every transition that
                // named the step follows it rather than dangling.
                for transition in &mut page.transitions {
                    for entry in transition.from.iter_mut().chain(transition.to.iter_mut()) {
                        if *entry == *step {
                            *entry = *number;
                        }
                    }
                }
                let after = page.clone();
                Ok(Edit::SfcPage {
                    section: *section,
                    before: Some(before),
                    after: Some(after),
                })
            }
            Command::SetStepInitial {
                section,
                step,
                initial,
            } => self.edit_step(*section, *step, |entry| entry.is_initial = *initial),
            Command::InsertTransition {
                section,
                transition,
            } => {
                let position = self.require_section(*section)?;
                if self.transition_number_taken(*section, transition.number) {
                    return Err(EditError::DuplicateTransition(transition.number));
                }
                let page = self.page_at(position, *section)?;
                let replacing = page.transition(transition.number).is_some_and(|existing| {
                    existing.x == transition.x && existing.y == transition.y
                });
                if page.transition(transition.number).is_some() && !replacing {
                    return Err(EditError::DuplicateTransition(transition.number));
                }
                let displaced = steps_at_cell(page, transition.x, transition.y);
                for number in transition.from.iter().chain(transition.to.iter()) {
                    // A step the placement itself displaces cannot be named: it
                    // would be pruned again by the clear below.
                    if page.step(*number).is_none() || displaced.contains(number) {
                        return Err(EditError::UnknownStep {
                            section: *section,
                            step: *number,
                        });
                    }
                }
                let before = page.clone();
                let removed = clear_cell(page, transition.x, transition.y, None);
                for number in removed {
                    prune_step(page, number);
                }
                let mut inserted = transition.clone();
                inserted.from = sorted_set(&inserted.from);
                inserted.to = sorted_set(&inserted.to);
                page.transitions.push(inserted);
                sort_page(page);
                let after = page.clone();
                Ok(Edit::SfcPage {
                    section: *section,
                    before: Some(before),
                    after: Some(after),
                })
            }
            Command::RemoveTransition {
                section,
                transition,
            } => {
                let position = self.require_section(*section)?;
                let page = self.page_at(position, *section)?;
                let at = page
                    .transitions
                    .iter()
                    .position(|entry| entry.number == *transition)
                    .ok_or(EditError::UnknownTransition {
                        section: *section,
                        transition: *transition,
                    })?;
                let before = page.clone();
                page.transitions.remove(at);
                let after = page.clone();
                Ok(Edit::SfcPage {
                    section: *section,
                    before: Some(before),
                    after: Some(after),
                })
            }
            Command::MoveTransition {
                section,
                transition,
                x,
                y,
            } => {
                let position = self.require_section(*section)?;
                let page = self.page_at(position, *section)?;
                let at = page
                    .transitions
                    .iter()
                    .position(|entry| entry.number == *transition)
                    .ok_or(EditError::UnknownTransition {
                        section: *section,
                        transition: *transition,
                    })?;
                let Some(before) = page.transitions.get(at).cloned() else {
                    return Err(EditError::UnknownTransition {
                        section: *section,
                        transition: *transition,
                    });
                };
                if (before.x == *x && before.y == *y) || cell_occupied(page, *x, *y) {
                    return Err(EditError::SfcCellOccupied {
                        section: *section,
                        x: *x,
                        y: *y,
                    });
                }
                if let Some(slot) = page.transitions.get_mut(at) {
                    slot.x = *x;
                    slot.y = *y;
                }
                let Some(after) = page.transitions.get(at).cloned() else {
                    return Err(EditError::UnknownTransition {
                        section: *section,
                        transition: *transition,
                    });
                };
                Ok(Edit::SfcTransition {
                    section: *section,
                    index: at,
                    before,
                    after,
                })
            }
            Command::SetTransitionCondition {
                section,
                transition,
                condition,
            } => {
                let parsed = parse_condition(condition.as_deref())?;
                self.edit_transition(*section, *transition, |entry| entry.condition = parsed)
            }
            Command::SetTransitionFrom {
                section,
                transition,
                from,
            } => {
                let set = sorted_set(from);
                self.require_steps(*section, &set)?;
                self.edit_transition(*section, *transition, |entry| entry.from = set)
            }
            Command::SetTransitionTo {
                section,
                transition,
                to,
            } => {
                let set = sorted_set(to);
                self.require_steps(*section, &set)?;
                self.edit_transition(*section, *transition, |entry| entry.to = set)
            }
            Command::SetPageComment { section, comment } => {
                let position = self.require_section(*section)?;
                let page = self.page_at(position, *section)?;
                let before = page.clone();
                page.comment = comment.clone();
                let after = page.clone();
                Ok(Edit::SfcPage {
                    section: *section,
                    before: Some(before),
                    after: Some(after),
                })
            }
            Command::AddPage { section, page } => {
                let position = self.require_section(*section)?;
                let target = self
                    .project
                    .sections
                    .get_mut(position)
                    .ok_or(EditError::UnknownSection(*section))?;
                if target.sequential_page.is_some() {
                    return Err(EditError::PageExists(*section));
                }
                target.sequential_page = Some(page.clone());
                Ok(Edit::SfcPage {
                    section: *section,
                    before: None,
                    after: Some(page.clone()),
                })
            }
            Command::RemovePage { section } => {
                let position = self.require_section(*section)?;
                let target = self
                    .project
                    .sections
                    .get_mut(position)
                    .ok_or(EditError::UnknownSection(*section))?;
                let before = target
                    .sequential_page
                    .take()
                    .ok_or(EditError::NoSequentialPage(*section))?;
                Ok(Edit::SfcPage {
                    section: *section,
                    before: Some(before),
                    after: None,
                })
            }
        }
    }

    /// The sequential page of the section at `position`.
    fn page_at(&mut self, position: usize, section: u32) -> Result<&mut SequentialPage, EditError> {
        self.project
            .sections
            .get_mut(position)
            .and_then(|target| target.sequential_page.as_mut())
            .ok_or(EditError::NoSequentialPage(section))
    }

    /// `true` when a step of another section already uses `number`.
    ///
    /// `%X<number>` is one variable in the scan engine, so a step number is
    /// unique across the whole project and not only inside one page.
    fn step_number_taken(&self, section: u32, number: u32) -> bool {
        self.project.sections.iter().any(|entry| {
            entry.id != section
                && entry
                    .sequential_page
                    .as_ref()
                    .is_some_and(|page| page.step(number).is_some())
        })
    }

    /// `true` when a transition of another section already uses `number`.
    fn transition_number_taken(&self, section: u32, number: u32) -> bool {
        self.project.sections.iter().any(|entry| {
            entry.id != section
                && entry
                    .sequential_page
                    .as_ref()
                    .is_some_and(|page| page.transition(number).is_some())
        })
    }

    /// Validates that every number in `steps` names a step of the page.
    fn require_steps(&self, section: u32, steps: &[u32]) -> Result<(), EditError> {
        let position = self.require_section(section)?;
        let page = self
            .project
            .sections
            .get(position)
            .and_then(|entry| entry.sequential_page.as_ref())
            .ok_or(EditError::NoSequentialPage(section))?;
        match steps.iter().find(|number| page.step(**number).is_none()) {
            Some(number) => Err(EditError::UnknownStep {
                section,
                step: *number,
            }),
            None => Ok(()),
        }
    }

    /// Applies `mutate` to one step of a section's page and snapshots it.
    fn edit_step(
        &mut self,
        section: u32,
        step: u32,
        mutate: impl FnOnce(&mut Step),
    ) -> Result<Edit, EditError> {
        let position = self.require_section(section)?;
        let page = self.page_at(position, section)?;
        let at = page
            .steps
            .iter()
            .position(|entry| entry.number == step)
            .ok_or(EditError::UnknownStep { section, step })?;
        let Some(before) = page.steps.get(at).cloned() else {
            return Err(EditError::UnknownStep { section, step });
        };
        if let Some(slot) = page.steps.get_mut(at) {
            mutate(slot);
        }
        let Some(after) = page.steps.get(at).cloned() else {
            return Err(EditError::UnknownStep { section, step });
        };
        Ok(Edit::SfcStep {
            section,
            index: at,
            before,
            after,
        })
    }

    /// Applies `mutate` to one transition of a section's page and snapshots it.
    fn edit_transition(
        &mut self,
        section: u32,
        transition: u32,
        mutate: impl FnOnce(&mut Transition),
    ) -> Result<Edit, EditError> {
        let position = self.require_section(section)?;
        let page = self.page_at(position, section)?;
        let at = page
            .transitions
            .iter()
            .position(|entry| entry.number == transition)
            .ok_or(EditError::UnknownTransition {
                section,
                transition,
            })?;
        let Some(before) = page.transitions.get(at).cloned() else {
            return Err(EditError::UnknownTransition {
                section,
                transition,
            });
        };
        if let Some(slot) = page.transitions.get_mut(at) {
            mutate(slot);
        }
        let Some(after) = page.transitions.get(at).cloned() else {
            return Err(EditError::UnknownTransition {
                section,
                transition,
            });
        };
        Ok(Edit::SfcTransition {
            section,
            index: at,
            before,
            after,
        })
    }
}

/// Keeps the element list in a canonical `(row, col)` order.
///
/// The format promises stable serialization (`docs/FORMAT.md`), so two projects
/// with the same elements must not differ only by insertion order.
fn sort_elements(rung: &mut Rung) {
    rung.elements
        .sort_by_key(|element| (element.row, element.col));
}

/// Keeps a page's steps and transitions in a canonical `(y, x)` order.
///
/// Same reasoning as [`sort_elements`]: the drawing reads positions, not
/// sequence, so the stored order is free and a stable one keeps a saved project
/// from depending on the order the user happened to click in.
fn sort_page(page: &mut SequentialPage) {
    page.steps.sort_by_key(|step| (step.y, step.x));
    page.transitions
        .sort_by_key(|transition| (transition.y, transition.x));
}

/// `true` when `(x, y)` of `page` already holds a step or a transition.
fn cell_occupied(page: &SequentialPage, x: i32, y: i32) -> bool {
    page.steps.iter().any(|step| step.x == x && step.y == y)
        || page
            .transitions
            .iter()
            .any(|transition| transition.x == x && transition.y == y)
}

/// Removes `number` from every `from` and `to` set of `page`.
fn prune_step(page: &mut SequentialPage, number: u32) {
    for transition in &mut page.transitions {
        transition.from.retain(|entry| *entry != number);
        transition.to.retain(|entry| *entry != number);
    }
}

/// The numbers of the steps occupying `(x, y)` of `page`.
fn steps_at_cell(page: &SequentialPage, x: i32, y: i32) -> Vec<u32> {
    page.steps
        .iter()
        .filter(|step| step.x == x && step.y == y)
        .map(|step| step.number)
        .collect()
}

/// Clears `(x, y)` of `page` and returns the numbers of the steps it removed.
///
/// `keep` names a step number that is about to be written back into the cell, so
/// its references survive the clear; every other displaced step is pruned by the
/// caller.
fn clear_cell(page: &mut SequentialPage, x: i32, y: i32, keep: Option<u32>) -> Vec<u32> {
    let removed: Vec<u32> = page
        .steps
        .iter()
        .filter(|step| step.x == x && step.y == y && Some(step.number) != keep)
        .map(|step| step.number)
        .collect();
    page.steps.retain(|step| step.x != x || step.y != y);
    page.transitions
        .retain(|transition| transition.x != x || transition.y != y);
    removed
}

/// A step set in the canonical order [`SequentialPage`] stores.
///
/// A set is a set: the order it was written in carries no meaning, so it is
/// sorted and deduplicated before it is stored.
fn sorted_set(numbers: &[u32]) -> Vec<u32> {
    let mut set = numbers.to_vec();
    set.sort_unstable();
    set.dedup();
    set
}

/// Parses the text of a transition condition.
///
/// A `None` or blank text clears the condition (the transition then fires
/// whenever its sources are active); anything else must parse as an expression
/// the scan engine can evaluate.
fn parse_condition(text: Option<&str>) -> Result<Option<Expr>, EditError> {
    match text.map(str::trim) {
        None | Some("") => Ok(None),
        Some(text) => text
            .parse::<Expr>()
            .map(Some)
            .map_err(|error| EditError::BadCondition {
                text: text.to_owned(),
                message: error.to_string(),
            }),
    }
}

/// Pushes `diagnostic` unless an identical one is already present.
fn push_unique(problems: &mut Vec<Diagnostic>, diagnostic: Diagnostic) {
    if !problems.contains(&diagnostic) {
        problems.push(diagnostic);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use softladder_core::{SimLamp, SimSwitch, Symbol};
    use softladder_project::ProjectError;

    fn var(text: &str) -> VarRef {
        text.parse().expect("test variable parses")
    }

    fn rung(id: u32) -> Rung {
        Rung {
            elements: vec![
                PlacedElement::with_var(ElementKind::ContactNo, var("%I0"), 0, 0),
                PlacedElement::with_var(ElementKind::CoilOut, var("%Q0"), 1, 0),
            ],
            ..Rung::new(id)
        }
    }

    fn project() -> Project {
        let mut project = Project::new("editor tests");
        let mut main = Section::new(1, "Main");
        main.rungs.push(1);
        main.rungs.push(2);
        project.sections.push(main);
        project.rungs.push(rung(1));
        project.rungs.push(rung(2));
        project.symbols.push(Symbol {
            name: "start".to_owned(),
            var: Some(var("%I0")),
            comment: String::new(),
            unit: None,
        });
        project
    }

    fn editor() -> Editor {
        Editor::new(project())
    }

    fn element(editor: &Editor, rung: u32, col: u8, row: u8) -> PlacedElement {
        editor
            .project()
            .rung(rung)
            .and_then(|target| {
                target
                    .elements
                    .iter()
                    .find(|element| element.col == col && element.row == row)
            })
            .cloned()
            .expect("element exists")
    }

    /// Applies a command that must be rejected and checks the rejection is a
    /// complete no-op: project, history and dirty flag all unchanged.
    fn reject(editor: &mut Editor, command: Command) -> EditError {
        let before = editor.project().clone();
        let history = editor.history_len();
        let dirty = editor.is_dirty();
        let error = editor.apply(command).expect_err("command must be rejected");
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

    // -- construction and file handling -------------------------------------

    #[test]
    fn a_new_editor_is_clean_and_has_no_history() {
        let project = project();
        let editor = Editor::new(project.clone());
        assert_eq!(editor.project(), &project);
        assert_eq!(editor.path(), None);
        assert!(!editor.is_dirty());
        assert!(!editor.can_undo());
        assert!(!editor.can_redo());
        assert_eq!(editor.history_len(), 0);
        assert_eq!(editor.undo_label(), None);
        assert_eq!(editor.redo_label(), None);
    }

    #[test]
    fn save_then_open_round_trips_and_is_clean() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("project.slprj");
        let mut editor = editor();
        editor
            .place_element(1, ElementKind::ContactNc, 2, 0, Some(var("%I1")), &[])
            .expect("places");
        let saved = editor.project().clone();

        editor.save(&path).expect("saves");
        assert_eq!(editor.path(), Some(path.as_path()));
        assert!(!editor.is_dirty());

        let reopened = Editor::open(&path).expect("opens");
        assert_eq!(reopened.project(), &saved);
        assert_eq!(reopened.path(), Some(path.as_path()));
        assert!(!reopened.is_dirty());
        assert!(!reopened.can_undo());
        assert_eq!(reopened.history_len(), 0);

        let text = std::fs::read_to_string(&path).expect("reads the file back");
        let document: serde_json::Value = serde_json::from_str(&text).expect("valid json");
        assert_eq!(document["schema_version"], serde_json::json!(2));
    }

    #[test]
    fn opening_a_missing_file_is_an_error_not_a_panic() {
        let error = Editor::open(Path::new("/definitely/not/here.slprj")).expect_err("must fail");
        assert!(matches!(error, EditError::Project(ProjectError::Io(_))));
    }

    #[test]
    fn opening_broken_documents_is_an_error() {
        let directory = tempfile::tempdir().expect("temporary directory");

        let broken = directory.path().join("broken.slprj");
        std::fs::write(&broken, "{ this is not a project }").expect("writes");
        let error = Editor::open(&broken).expect_err("must fail");
        assert!(matches!(error, EditError::Project(ProjectError::Json(_))));

        let future = directory.path().join("future.slprj");
        std::fs::write(&future, r#"{"schema_version": 99}"#).expect("writes");
        let error = Editor::open(&future).expect_err("must fail");
        assert!(matches!(
            error,
            EditError::Project(ProjectError::UnsupportedSchema(99))
        ));
    }

    #[test]
    fn save_current_needs_a_path_and_then_uses_it() {
        let mut editor = editor();
        assert!(matches!(
            editor.save_current().expect_err("no path"),
            EditError::NoPath
        ));

        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("current.slprj");
        editor.set_path(Some(path.clone()));
        assert_eq!(editor.path(), Some(path.as_path()));
        editor
            .place_element(1, ElementKind::ContactNc, 2, 0, Some(var("%I1")), &[])
            .expect("places");
        editor.save_current().expect("saves to the known path");
        assert!(!editor.is_dirty());
        assert!(path.exists());

        editor.set_path(None);
        assert_eq!(editor.path(), None);
        assert!(matches!(
            editor.save_current().expect_err("no path again"),
            EditError::NoPath
        ));
    }

    #[test]
    fn the_dirty_flag_follows_apply_undo_redo_save_and_mark_clean() {
        let mut editor = editor();
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("dirty.slprj");

        assert!(!editor.is_dirty());
        editor
            .place_element(1, ElementKind::ContactNc, 2, 0, Some(var("%I1")), &[])
            .expect("places");
        assert!(editor.is_dirty());

        editor.mark_clean();
        assert!(!editor.is_dirty());
        assert!(editor.undo());
        assert!(editor.is_dirty(), "undo is an unsaved change");

        editor.mark_clean();
        assert!(editor.redo());
        assert!(editor.is_dirty(), "redo is an unsaved change");

        editor.mark_clean();
        editor.save(&path).expect("saves");
        assert!(!editor.is_dirty());
    }

    // -- element commands ----------------------------------------------------

    #[test]
    fn place_element_stores_every_part() {
        let mut editor = editor();
        editor
            .place_element(1, ElementKind::ContactNc, 2, 0, Some(var("%I1")), &[])
            .expect("places");
        let placed = element(&editor, 1, 2, 0);
        assert_eq!(placed.kind, ElementKind::ContactNc);
        assert_eq!(placed.var, Some(var("%I1")));
        assert!(placed.params.is_empty());
        assert!(!placed.connected_with_top);
        assert!(editor.is_dirty());
        assert_eq!(editor.history_len(), 1);
        assert!(editor.can_undo());
        assert!(!editor.can_redo());
    }

    #[test]
    fn place_element_rejects_an_occupied_cell() {
        let mut editor = editor();
        let error = reject(
            &mut editor,
            Command::PlaceElement {
                rung: 1,
                element: PlacedElement::with_var(ElementKind::ContactNc, var("%I2"), 0, 0),
            },
        );
        assert!(matches!(
            error,
            EditError::Occupied {
                rung: 1,
                col: 0,
                row: 0
            }
        ));
    }

    #[test]
    fn place_element_rejects_an_unknown_rung() {
        let mut editor = editor();
        let error = reject(
            &mut editor,
            Command::PlaceElement {
                rung: 99,
                element: PlacedElement::new(ElementKind::ContactNo, 0, 0),
            },
        );
        assert!(matches!(error, EditError::UnknownRung(99)));
    }

    #[test]
    fn delete_element_removes_the_cell() {
        let mut editor = editor();
        let before = editor.project().clone();
        editor.delete_element(1, 0, 0).expect("deletes");
        assert_eq!(editor.project().rungs[0].elements.len(), 1);
        assert!(editor.undo());
        assert_eq!(editor.project(), &before);
        editor.redo();
        assert_eq!(editor.project().rungs[0].elements.len(), 1);
    }

    #[test]
    fn delete_element_rejects_a_cell_without_an_element() {
        let mut editor = editor();
        let error = reject(
            &mut editor,
            Command::RemoveElement {
                rung: 1,
                col: 5,
                row: 2,
            },
        );
        assert!(matches!(
            error,
            EditError::NoSuchCell {
                rung: 1,
                col: 5,
                row: 2
            }
        ));
    }

    #[test]
    fn move_element_moves_and_keeps_the_grid_sorted() {
        let mut editor = editor();
        let before = editor.project().clone();
        editor.move_element(1, (1, 0), (0, 1)).expect("moves");
        assert_eq!(element(&editor, 1, 0, 1).kind, ElementKind::CoilOut);
        assert_eq!(element(&editor, 1, 0, 1).var, Some(var("%Q0")));
        let cells: Vec<(u8, u8)> = editor.project().rungs[0]
            .elements
            .iter()
            .map(|element| (element.col, element.row))
            .collect();
        assert_eq!(cells, vec![(0, 0), (0, 1)], "elements stay sorted");
        assert!(editor.undo());
        assert_eq!(editor.project(), &before);
    }

    #[test]
    fn move_element_rejects_missing_sources_and_busy_targets() {
        let mut editor = editor();
        let missing = reject(
            &mut editor,
            Command::MoveElement {
                rung: 1,
                from_col: 4,
                from_row: 0,
                to_col: 5,
                to_row: 0,
            },
        );
        assert!(matches!(
            missing,
            EditError::NoSuchCell {
                rung: 1,
                col: 4,
                row: 0
            }
        ));

        let busy = reject(
            &mut editor,
            Command::MoveElement {
                rung: 1,
                from_col: 1,
                from_row: 0,
                to_col: 0,
                to_row: 0,
            },
        );
        assert!(matches!(
            busy,
            EditError::Occupied {
                rung: 1,
                col: 0,
                row: 0
            }
        ));

        let itself = reject(
            &mut editor,
            Command::MoveElement {
                rung: 1,
                from_col: 1,
                from_row: 0,
                to_col: 1,
                to_row: 0,
            },
        );
        assert!(matches!(itself, EditError::Occupied { .. }));

        let unknown = reject(
            &mut editor,
            Command::MoveElement {
                rung: 42,
                from_col: 0,
                from_row: 0,
                to_col: 1,
                to_row: 0,
            },
        );
        assert!(matches!(unknown, EditError::UnknownRung(42)));
    }

    #[test]
    fn set_element_var_binds_and_unbinds() {
        let mut editor = editor();
        editor
            .set_element_var(1, 0, 0, Some(var("%M7")))
            .expect("binds");
        assert_eq!(element(&editor, 1, 0, 0).var, Some(var("%M7")));
        assert_eq!(element(&editor, 1, 0, 0).kind, ElementKind::ContactNo);

        editor.set_element_var(1, 0, 0, None).expect("unbinds");
        assert_eq!(element(&editor, 1, 0, 0).var, None);

        let error = reject(
            &mut editor,
            Command::SetElementVar {
                rung: 1,
                col: 9,
                row: 9,
                var: None,
            },
        );
        assert!(matches!(error, EditError::NoSuchCell { .. }));
    }

    #[test]
    fn set_element_params_replaces_the_list() {
        let mut editor = editor();
        editor
            .set_element_params(1, 1, 0, &["%MW0", "=", "3"])
            .expect("sets");
        assert_eq!(
            element(&editor, 1, 1, 0).params,
            vec!["%MW0".to_owned(), "=".to_owned(), "3".to_owned()]
        );
        editor.set_element_params(1, 1, 0, &[]).expect("clears");
        assert!(element(&editor, 1, 1, 0).params.is_empty());

        let error = reject(
            &mut editor,
            Command::SetElementParams {
                rung: 1,
                col: 4,
                row: 0,
                params: Vec::new(),
            },
        );
        assert!(matches!(error, EditError::NoSuchCell { .. }));
    }

    #[test]
    fn set_element_kind_keeps_the_variable() {
        let mut editor = editor();
        let before = editor.project().clone();
        editor
            .apply(Command::SetElementKind {
                rung: 1,
                col: 0,
                row: 0,
                kind: ElementKind::ContactRising,
            })
            .expect("changes");
        let changed = element(&editor, 1, 0, 0);
        assert_eq!(changed.kind, ElementKind::ContactRising);
        assert_eq!(changed.var, Some(var("%I0")));
        assert!(editor.undo());
        assert_eq!(editor.project(), &before);

        let error = reject(
            &mut editor,
            Command::SetElementKind {
                rung: 1,
                col: 6,
                row: 0,
                kind: ElementKind::CoilSet,
            },
        );
        assert!(matches!(error, EditError::NoSuchCell { .. }));
    }

    #[test]
    fn set_vertical_link_toggles_the_cell_flag() {
        let mut editor = editor();
        editor
            .set_vertical_link(1, 1, 0, true)
            .expect("links upwards");
        assert!(element(&editor, 1, 1, 0).connected_with_top);
        let error = reject(
            &mut editor,
            Command::SetVerticalLink {
                rung: 1,
                col: 3,
                row: 3,
                linked: true,
            },
        );
        assert!(matches!(error, EditError::NoSuchCell { .. }));
    }

    #[test]
    fn set_rung_text_sets_label_and_comment() {
        let mut editor = editor();
        let before = editor.project().clone();
        editor
            .set_rung_text(1, "LAMP", "self holding lamp")
            .expect("sets");
        let target = editor.project().rung(1).expect("rung exists");
        assert_eq!(target.label, "LAMP");
        assert_eq!(target.comment, "self holding lamp");
        assert!(editor.undo());
        assert_eq!(editor.project(), &before);

        let error = reject(
            &mut editor,
            Command::SetRungText {
                rung: 99,
                label: String::new(),
                comment: String::new(),
            },
        );
        assert!(matches!(error, EditError::UnknownRung(99)));
        assert!(matches!(
            reject(
                &mut editor,
                Command::RemoveElement {
                    rung: 99,
                    col: 0,
                    row: 0
                }
            ),
            EditError::UnknownRung(99)
        ));
    }

    // -- rung commands -------------------------------------------------------

    #[test]
    fn insert_rung_uses_a_fresh_id_and_both_lists() {
        let mut editor = editor();
        let id = editor.insert_rung(1, 1).expect("inserts");
        assert_eq!(id, 3, "max existing id plus one");
        assert_eq!(editor.project().sections[0].rungs, vec![1, 3, 2]);
        assert!(editor.project().rung(3).is_some());
        assert_eq!(editor.project().rungs.last().map(|rung| rung.id), Some(3));

        assert!(editor.undo());
        assert!(editor.project().rung(3).is_none());
        assert_eq!(editor.project().sections[0].rungs, vec![1, 2]);
        assert_eq!(editor.project().rungs.len(), 2);

        assert!(editor.redo());
        assert!(editor.project().rung(3).is_some());
        assert_eq!(editor.project().sections[0].rungs, vec![1, 3, 2]);
        assert_eq!(editor.project().rungs.len(), 3);
    }

    #[test]
    fn insert_rung_appends_into_an_empty_section() {
        let mut editor = editor();
        let section = editor
            .add_section("Sub", SectionLanguage::Ladder)
            .expect("adds");
        assert_eq!(section, 2);
        assert!(editor
            .project()
            .section(2)
            .is_some_and(|s| s.rungs.is_empty()));

        let id = editor.insert_rung(section, 0).expect("inserts");
        assert_eq!(id, 3, "the id counts rungs of every section");
        assert_eq!(
            editor.project().section(2).map(|s| s.rungs.clone()),
            Some(vec![3])
        );
    }

    #[test]
    fn insert_rung_rejects_bad_positions_and_duplicate_ids() {
        let mut editor = editor();
        let error = reject(
            &mut editor,
            Command::InsertRung {
                section: 1,
                index: 9,
                rung: Rung::new(50),
            },
        );
        assert!(matches!(
            error,
            EditError::IndexOutOfRange { index: 9, len: 2 }
        ));

        let error = reject(
            &mut editor,
            Command::InsertRung {
                section: 7,
                index: 0,
                rung: Rung::new(50),
            },
        );
        assert!(matches!(error, EditError::UnknownSection(7)));

        let error = reject(
            &mut editor,
            Command::InsertRung {
                section: 1,
                index: 0,
                rung: Rung::new(1),
            },
        );
        assert!(matches!(error, EditError::DuplicateRung(1)));
    }

    #[test]
    fn delete_rung_removes_it_from_both_lists() {
        let mut editor = editor();
        let before = editor.project().clone();
        editor.delete_rung(1, 1).expect("deletes");
        assert!(editor.project().rung(1).is_none());
        assert_eq!(editor.project().sections[0].rungs, vec![2]);
        assert_eq!(editor.project().rungs.len(), 1);

        assert!(editor.undo());
        assert_eq!(editor.project(), &before);
        assert_eq!(
            editor.project().rungs[0].id,
            1,
            "the pool order is restored"
        );
    }

    #[test]
    fn deleting_the_last_rung_of_a_section_is_allowed_and_clean() {
        let mut editor = editor();
        let before = editor.project().clone();
        editor.delete_rung(1, 1).expect("deletes");
        editor.delete_rung(1, 2).expect("deletes the last rung too");
        assert!(editor.project().sections[0].rungs.is_empty());
        assert!(editor.project().rungs.is_empty());
        assert!(
            editor.problems().is_empty(),
            "an empty section is valid: {:?}",
            editor.problems()
        );

        assert!(editor.undo());
        assert!(editor.undo());
        assert_eq!(editor.project(), &before);
    }

    #[test]
    fn delete_rung_rejects_unknown_ids_and_rungs_outside_the_section() {
        let mut editor = editor();
        let error = reject(
            &mut editor,
            Command::DeleteRung {
                section: 7,
                rung: 1,
            },
        );
        assert!(matches!(error, EditError::UnknownSection(7)));

        let mut project = project();
        project.sections[0].rungs = vec![1];
        let mut editor = Editor::new(project);
        let error = reject(
            &mut editor,
            Command::DeleteRung {
                section: 1,
                rung: 2,
            },
        );
        assert!(matches!(error, EditError::UnknownRung(2)));
        assert!(
            editor.project().rung(2).is_some(),
            "the rejected delete kept the rung"
        );
    }

    #[test]
    fn delete_rung_keeps_a_rung_shared_with_another_section() {
        let mut project = project();
        let mut second = Section::new(2, "Second");
        second.rungs.push(1);
        project.sections.push(second);
        let mut editor = Editor::new(project);

        editor
            .delete_rung(1, 1)
            .expect("deletes from the first section");
        assert_eq!(editor.project().sections[0].rungs, vec![2]);
        assert_eq!(editor.project().sections[1].rungs, vec![1]);
        assert!(
            editor.project().rung(1).is_some(),
            "the shared rung stays in the flat pool"
        );

        assert!(editor.undo());
        assert_eq!(editor.project().sections[0].rungs, vec![1, 2]);
    }

    #[test]
    fn move_rung_reorders_and_validates_positions() {
        let mut editor = editor();
        let before = editor.project().clone();
        editor
            .apply(Command::MoveRung {
                section: 1,
                from: 0,
                to: 1,
            })
            .expect("moves");
        assert_eq!(editor.project().sections[0].rungs, vec![2, 1]);
        assert!(editor.undo());
        assert_eq!(editor.project(), &before);

        let error = reject(
            &mut editor,
            Command::MoveRung {
                section: 1,
                from: 0,
                to: 9,
            },
        );
        assert!(matches!(
            error,
            EditError::IndexOutOfRange { index: 9, len: 2 }
        ));
        let error = reject(
            &mut editor,
            Command::MoveRung {
                section: 1,
                from: 9,
                to: 0,
            },
        );
        assert!(matches!(
            error,
            EditError::IndexOutOfRange { index: 9, len: 2 }
        ));
        let error = reject(
            &mut editor,
            Command::MoveRung {
                section: 8,
                from: 0,
                to: 1,
            },
        );
        assert!(matches!(error, EditError::UnknownSection(8)));
    }

    // -- section commands ----------------------------------------------------

    #[test]
    fn add_section_uses_a_fresh_id_and_appends() {
        let mut editor = editor();
        let id = editor
            .add_section("Sub", SectionLanguage::Sfc)
            .expect("adds");
        assert_eq!(id, 2, "max existing id plus one");
        let added = editor.project().section(2).expect("added");
        assert_eq!(added.name, "Sub");
        assert_eq!(added.language, SectionLanguage::Sfc);
        assert!(added.rungs.is_empty());
        assert!(editor.undo());
        assert!(editor.project().section(2).is_none());
        assert_eq!(editor.project().sections.len(), 1);
    }

    #[test]
    fn add_section_rejects_a_duplicate_id() {
        let mut editor = editor();
        let error = reject(
            &mut editor,
            Command::AddSection {
                section: Section::new(1, "duplicate"),
            },
        );
        assert!(matches!(error, EditError::DuplicateSection(1)));
    }

    #[test]
    fn remove_section_drops_the_rungs_it_owned() {
        let mut editor = editor();
        let before = editor.project().clone();
        editor.remove_section(1).expect("removes");
        assert!(editor.project().sections.is_empty());
        assert!(editor.project().rungs.is_empty());

        assert!(editor.undo());
        assert_eq!(editor.project(), &before);
        assert_eq!(editor.project().rungs.len(), 2);
        assert_eq!(
            editor.project().rungs[0].id,
            1,
            "the pool order is restored"
        );
        assert_eq!(editor.project().rungs[1].id, 2);
    }

    #[test]
    fn remove_section_keeps_a_rung_shared_with_another_section() {
        let mut project = project();
        let mut second = Section::new(2, "Second");
        second.rungs.push(2);
        project.sections.push(second);
        let mut editor = Editor::new(project);

        editor.remove_section(1).expect("removes");
        assert_eq!(editor.project().sections.len(), 1);
        assert_eq!(editor.project().rungs.len(), 1);
        assert_eq!(editor.project().rungs[0].id, 2);
        assert!(editor.project().rung(1).is_none());

        assert!(editor.undo());
        assert_eq!(editor.project().rungs.len(), 2);
        assert_eq!(editor.project().rungs[0].id, 1);
        assert_eq!(editor.project().rungs[1].id, 2);
    }

    #[test]
    fn remove_section_rejects_an_unknown_id() {
        let mut editor = editor();
        let error = reject(&mut editor, Command::RemoveSection { section: 4 });
        assert!(matches!(error, EditError::UnknownSection(4)));
    }

    #[test]
    fn rename_section_sets_the_name() {
        let mut editor = editor();
        editor.rename_section(1, "Renamed").expect("renames");
        assert_eq!(editor.project().sections[0].name, "Renamed");
        assert!(editor.undo());
        assert_eq!(editor.project().sections[0].name, "Main");
        let error = reject(
            &mut editor,
            Command::SetSectionName {
                section: 4,
                name: "nope".to_owned(),
            },
        );
        assert!(matches!(error, EditError::UnknownSection(4)));
    }

    // -- project-wide commands -----------------------------------------------

    #[test]
    fn set_symbols_replaces_the_table() {
        let mut editor = editor();
        let before = editor.project().clone();
        editor
            .set_symbols(vec![Symbol {
                name: "stop".to_owned(),
                var: Some(var("%I1")),
                comment: "stop button".to_owned(),
                unit: None,
            }])
            .expect("sets");
        assert_eq!(editor.project().symbols.len(), 1);
        assert_eq!(editor.project().symbols[0].name, "stop");
        assert!(editor.undo());
        assert_eq!(editor.project(), &before);
    }

    #[test]
    fn set_panel_replaces_the_bench_layout() {
        let mut editor = editor();
        let before = editor.project().clone();
        editor
            .set_panel(SimulationPanel {
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
            })
            .expect("sets");
        assert_eq!(editor.project().simulation.len(), 2);
        assert!(editor.undo());
        assert_eq!(editor.project(), &before);
    }

    #[test]
    fn auto_fill_panel_mirrors_the_physical_variables() {
        let mut editor = editor();
        editor.auto_fill_panel().expect("fills");
        let panel = &editor.project().simulation;
        assert_eq!(panel.switches.len(), 1);
        assert_eq!(panel.switches[0].var, var("%I0"));
        assert_eq!(
            panel.switches[0].label, "start",
            "the symbol provides the label"
        );
        assert_eq!(panel.lamps.len(), 1);
        assert_eq!(panel.lamps[0].var, var("%Q0"));
        assert!(panel.validate().is_empty());
        assert!(editor.undo());
        assert!(editor.project().simulation.is_empty());
    }

    #[test]
    fn set_scan_config_replaces_the_timing() {
        let mut editor = editor();
        editor
            .set_scan_config(ScanConfig {
                period_ms: 25,
                input_period_ms: 5,
            })
            .expect("sets");
        assert_eq!(editor.project().scan.period_ms, 25);
        assert_eq!(editor.project().scan.input_period_ms, 5);
        assert!(editor.undo());
        assert_eq!(editor.project().scan, ScanConfig::default());
    }

    // -- undo, redo and history ----------------------------------------------

    #[test]
    fn undo_and_redo_restore_the_project_exactly() {
        let mut editor = editor();
        let original = editor.project().clone();
        editor
            .place_element(1, ElementKind::ContactNc, 3, 0, Some(var("%I2")), &[])
            .expect("places");
        let edited = editor.project().clone();
        assert_ne!(edited, original);

        assert!(editor.can_undo());
        assert!(editor.undo());
        assert_eq!(editor.project(), &original, "undo is byte-identical");
        assert!(!editor.can_undo());
        assert!(editor.can_redo());
        assert_eq!(editor.history_len(), 0);

        assert!(editor.redo());
        assert_eq!(editor.project(), &edited, "redo is byte-identical");
        assert_eq!(editor.history_len(), 1);
    }

    #[test]
    fn undo_all_then_redo_all_round_trips_a_long_session() {
        let mut editor = editor();
        let original = editor.project().clone();

        editor
            .place_element(1, ElementKind::ContactNc, 2, 0, Some(var("%I1")), &[])
            .expect("places");
        editor
            .set_rung_text(1, "LAMP", "self holding")
            .expect("sets text");
        editor.set_vertical_link(1, 1, 0, true).expect("links");
        editor
            .set_element_var(1, 0, 0, Some(var("%M0")))
            .expect("rebinds");
        editor
            .set_element_params(2, 0, 0, &["%MW0", "=", "1"])
            .expect("sets params");
        editor
            .apply(Command::SetElementKind {
                rung: 1,
                col: 2,
                row: 0,
                kind: ElementKind::CoilSet,
            })
            .expect("changes kind");
        let inserted = editor.insert_rung(1, 1).expect("inserts a rung");
        editor
            .place_element(inserted, ElementKind::CoilOut, 1, 0, Some(var("%Q1")), &[])
            .expect("places on the new rung");
        editor
            .move_element(inserted, (1, 0), (2, 0))
            .expect("moves");
        editor.delete_element(2, 0, 0).expect("deletes");
        editor
            .set_symbols(vec![Symbol {
                name: "lamp".to_owned(),
                var: Some(var("%Q0")),
                comment: String::new(),
                unit: None,
            }])
            .expect("sets symbols");
        editor
            .set_panel(SimulationPanel {
                switches: vec![SimSwitch {
                    var: var("%I0"),
                    label: "start".to_owned(),
                    momentary: false,
                }],
                ..SimulationPanel::default()
            })
            .expect("sets the panel");
        editor
            .set_scan_config(ScanConfig {
                period_ms: 20,
                input_period_ms: 5,
            })
            .expect("sets the scan config");
        let sub = editor
            .add_section("Sub", SectionLanguage::Ladder)
            .expect("adds a section");
        let sub_rung = editor.insert_rung(sub, 0).expect("inserts into it");
        editor.rename_section(sub, "Subroutine").expect("renames");
        editor
            .apply(Command::MoveRung {
                section: sub,
                from: 0,
                to: 0,
            })
            .expect("moves the rung onto itself");
        editor.remove_section(sub).expect("removes the section");
        editor.delete_rung(1, 2).expect("deletes a rung");
        editor
            .set_vertical_link(inserted, 2, 0, false)
            .expect("clears a link");

        let commands = 20;
        let edited = editor.project().clone();
        assert_ne!(edited, original);
        assert_eq!(editor.history_len(), commands);

        let mut undone = 0;
        while editor.undo() {
            undone += 1;
        }
        assert_eq!(undone, commands);
        assert_eq!(
            editor.project(),
            &original,
            "undo-all restores the original"
        );
        assert!(!editor.can_undo());

        let mut redone = 0;
        while editor.redo() {
            redone += 1;
        }
        assert_eq!(redone, commands);
        assert_eq!(editor.project(), &edited, "redo-all restores the edit");
        assert_eq!(editor.history_len(), commands);
        assert!(
            editor.project().rung(sub_rung).is_none(),
            "the removed section took its rung with it"
        );
    }

    #[test]
    fn a_new_command_clears_the_redo_stack() {
        let mut editor = editor();
        editor
            .place_element(1, ElementKind::ContactNc, 2, 0, Some(var("%I1")), &[])
            .expect("places");
        assert!(editor.undo());
        assert!(editor.can_redo());
        assert_eq!(editor.redo_label(), Some("place element"));

        editor
            .place_element(1, ElementKind::ContactNc, 3, 0, Some(var("%I2")), &[])
            .expect("places again");
        assert!(!editor.can_redo());
        assert_eq!(editor.redo_label(), None);
        assert_eq!(editor.undo_label(), Some("place element"));
    }

    #[test]
    fn undo_and_redo_report_false_on_an_empty_history() {
        let mut editor = editor();
        assert!(!editor.undo());
        assert!(!editor.redo());
        assert!(!editor.can_undo());
        assert!(!editor.can_redo());
        assert_eq!(editor.undo_label(), None);
        assert_eq!(editor.redo_label(), None);
        assert_eq!(editor.history_len(), 0);
    }

    #[test]
    fn labels_survive_undo_and_redo() {
        let mut editor = editor();
        editor
            .apply(Command::RemoveElement {
                rung: 1,
                col: 0,
                row: 0,
            })
            .expect("deletes");
        assert_eq!(editor.undo_label(), Some("delete element"));
        assert_eq!(editor.redo_label(), None);

        editor.undo();
        assert_eq!(editor.undo_label(), None);
        assert_eq!(editor.redo_label(), Some("delete element"));
        assert!(editor.redo());
        assert_eq!(editor.undo_label(), Some("delete element"));
    }

    #[test]
    fn clear_history_drops_both_stacks_but_keeps_the_project() {
        let mut editor = editor();
        editor
            .place_element(1, ElementKind::ContactNc, 2, 0, Some(var("%I1")), &[])
            .expect("places");
        editor.undo();
        assert!(editor.can_redo());
        let project = editor.project().clone();

        editor.clear_history();
        assert!(!editor.can_undo());
        assert!(!editor.can_redo());
        assert_eq!(editor.history_len(), 0);
        assert_eq!(editor.undo_label(), None);
        assert_eq!(editor.redo_label(), None);
        assert_eq!(editor.project(), &project);
    }

    #[test]
    fn the_history_is_bounded_and_drops_the_oldest_entry() {
        let mut editor = editor();
        for _ in 0..((MAX_HISTORY + 5) / 2) {
            let id = editor.insert_rung(1, 0).expect("inserts");
            editor.delete_rung(1, id).expect("deletes");
        }
        assert_eq!(editor.history_len(), MAX_HISTORY);
        assert!(editor.can_undo());

        let mut undone = 0;
        while editor.undo() {
            undone += 1;
        }
        assert_eq!(undone, MAX_HISTORY);
        assert!(!editor.can_undo());
        assert!(editor.can_redo());
    }

    // -- diagnostics ---------------------------------------------------------

    #[test]
    fn a_well_formed_project_has_no_problems() {
        let editor = editor();
        assert!(editor.problems().is_empty(), "{:?}", editor.problems());
        assert!(editor.diagnostics().is_empty());
    }

    #[test]
    fn problems_report_duplicate_cells_as_sl_e009() {
        let mut project = project();
        project.rungs[0].elements.push(PlacedElement::with_var(
            ElementKind::ContactNc,
            var("%I1"),
            0,
            0,
        ));
        let editor = Editor::new(project);
        assert!(
            editor.problems().iter().any(|d| d.code == "SL-E009"),
            "{:?}",
            editor.problems()
        );
    }

    #[test]
    fn problems_report_panel_validation_as_sl_w020() {
        let mut editor = editor();
        editor
            .set_panel(SimulationPanel {
                switches: vec![SimSwitch {
                    var: var("%Q0"),
                    label: "wrong".to_owned(),
                    momentary: false,
                }],
                ..SimulationPanel::default()
            })
            .expect("sets");
        let warnings: Vec<&Diagnostic> = editor
            .problems()
            .iter()
            .filter(|diagnostic| diagnostic.code == "SL-W020")
            .collect();
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].severity, Severity::Warning);
        assert!(warnings[0].message.contains("wrong"));
    }

    #[test]
    fn problems_report_the_errors_of_the_last_scan() {
        let mut project = Project::new("divide by zero");
        let mut section = Section::new(1, "Main");
        section.rungs.push(1);
        project.sections.push(section);
        project.rungs.push(Rung {
            elements: vec![PlacedElement::with_params(
                ElementKind::Compare,
                0,
                0,
                &["1 / 0"],
            )],
            ..Rung::new(1)
        });

        let mut editor = Editor::new(project);
        editor.refresh_diagnostics();
        assert!(
            editor.diagnostics().iter().any(|d| d.code == "SL-E002"),
            "{:?}",
            editor.diagnostics()
        );
        assert!(editor.problems().iter().any(|d| d.code == "SL-E002"));
        assert!(editor
            .diagnostics()
            .iter()
            .all(|diagnostic| diagnostic.severity == Severity::Error));
    }

    #[test]
    fn problems_are_refreshed_by_edits_undo_and_redo() {
        let mut editor = editor();
        assert!(!editor.problems().iter().any(|d| d.code == "SL-E004"));

        editor
            .place_element(1, ElementKind::ContactNo, 2, 0, None, &[])
            .expect("places");
        assert!(
            editor.problems().iter().any(|d| d.code == "SL-E004"),
            "the missing variable is reported without an explicit refresh"
        );

        editor.undo();
        assert!(!editor.problems().iter().any(|d| d.code == "SL-E004"));
        editor.redo();
        assert!(editor.problems().iter().any(|d| d.code == "SL-E004"));
    }

    #[test]
    fn a_bad_panel_does_not_leak_into_a_clean_one() {
        let mut editor = editor();
        editor
            .set_panel(SimulationPanel {
                switches: vec![SimSwitch {
                    var: var("%M0"),
                    label: "not physical".to_owned(),
                    momentary: false,
                }],
                ..SimulationPanel::default()
            })
            .expect("sets");
        assert!(editor.problems().iter().any(|d| d.code == "SL-W020"));
        editor
            .set_panel(SimulationPanel::default())
            .expect("clears");
        assert!(!editor.problems().iter().any(|d| d.code == "SL-W020"));
    }
}
