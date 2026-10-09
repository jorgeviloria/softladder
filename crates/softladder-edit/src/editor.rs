//! The authoritative editing state: project, file, history and diagnostics.
//!
//! [`Editor`] owns the one true [`Project`]. The UI never mutates it directly:
//! it emits a [`Command`], [`Editor::apply`] validates and executes it, and the
//! command is recorded as a partial snapshot for undo. A rejected command
//! leaves the project byte-identical and records nothing.

use std::path::{Path, PathBuf};

use softladder_core::{
    lint, Diagnostic, ElementKind, PlacedElement, Project, Rung, ScanConfig, ScanEngine, Section,
    SectionLanguage, Severity, SimulationPanel, Symbol, VarRef,
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
        }
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
