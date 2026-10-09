//! Undo records: the minimal before/after snapshot of what a command touched.
//!
//! The obvious way to implement undo is to clone the whole [`Project`] before
//! every command. That is easy but wasteful: a project holds every rung, symbol
//! and panel widget, so a 1,000-entry history would keep 1,000 full copies of
//! the program, and a single contact edit would copy all of it.
//!
//! Instead, each [`Edit`] snapshots **only what the command changed**:
//!
//! * an element edit stores the `before`/`after` of the one rung it touched
//!   (a rung is a small value: an id, two strings and a short element list);
//! * an insert stores the inserted rung plus its position in the section and in
//!   the flat rung pool, which is all that is needed to take it out again;
//! * a delete stores the removed rung plus its two positions;
//! * a section add/remove stores that section, plus the pool rungs it owned;
//! * an SFC edit stores the sequential page it touched, or the one step or
//!   transition that changed in place;
//! * the symbol table, the panel and the scan config are stored as whole
//!   before/after values because those commands do replace them wholesale.
//!
//! Replay is defensive: a missing rung or section is ignored rather than
//! panicked on, so a corrupted history can never take the editor down.

use softladder_core::{
    Project, Rung, ScanConfig, Section, SequentialPage, SimulationPanel, Step, Symbol, Transition,
};

/// Direction a recorded edit is replayed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Direction {
    /// Reverse the command.
    Undo,
    /// Re-apply the command.
    Redo,
}

/// The change a single command performed, as before/after snapshots.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Edit {
    /// A rung was replaced in place (its elements, label or comment).
    Rung {
        /// Id of the rung.
        id: u32,
        /// State before the command.
        before: Rung,
        /// State after the command.
        after: Rung,
    },
    /// A rung was added to a section and appended to the project rung pool.
    InsertRung {
        /// Section that received the rung.
        section: u32,
        /// Position inside `Section::rungs`.
        index: usize,
        /// Position inside `Project::rungs`.
        pool_index: usize,
        /// The inserted rung.
        rung: Rung,
    },
    /// A rung was removed from a section, and from the pool when unshared.
    RemoveRung {
        /// Section the rung was removed from.
        section: u32,
        /// Position it occupied inside `Section::rungs`.
        index: usize,
        /// The removed rung.
        rung: Rung,
        /// Position it occupied inside `Project::rungs`, when it was dropped
        /// from the pool; `None` when another section still referenced it.
        pool_index: Option<usize>,
    },
    /// A section's rung order changed.
    MoveRung {
        /// Section that was re-ordered.
        section: u32,
        /// Order before the command.
        before: Vec<u32>,
        /// Order after the command.
        after: Vec<u32>,
    },
    /// A section was added.
    InsertSection {
        /// Position inside `Project::sections`.
        index: usize,
        /// The added section.
        section: Section,
    },
    /// A section, and the pool rungs it owned exclusively, were removed.
    RemoveSection {
        /// Position the section occupied in `Project::sections`.
        index: usize,
        /// The removed section.
        section: Section,
        /// The pool rungs it owned, with their ascending pool positions.
        rungs: Vec<(usize, Rung)>,
    },
    /// A section was renamed.
    SectionName {
        /// Id of the section.
        id: u32,
        /// Name before the command.
        before: String,
        /// Name after the command.
        after: String,
    },
    /// The sequential page of an SFC section was added, removed or replaced.
    ///
    /// A page is a small value — the same order of magnitude as the rung an
    /// element edit snapshots — so the commands that change its shape (inserting
    /// or removing a step or a transition, renumbering one, editing its comment,
    /// adding or removing the page itself) record it whole. The alternative, a
    /// diff of the two vectors, would be more code for no measurable saving.
    SfcPage {
        /// Id of the section that owns the page.
        section: u32,
        /// Page before the command; `None` when the section had none.
        before: Option<SequentialPage>,
        /// Page after the command; `None` when the command removed it.
        after: Option<SequentialPage>,
    },
    /// One step of a sequential page changed in place.
    SfcStep {
        /// Id of the section that owns the page.
        section: u32,
        /// Position inside `SequentialPage::steps`.
        index: usize,
        /// Step before the command.
        before: Step,
        /// Step after the command.
        after: Step,
    },
    /// One transition of a sequential page changed in place.
    SfcTransition {
        /// Id of the section that owns the page.
        section: u32,
        /// Position inside `SequentialPage::transitions`.
        index: usize,
        /// Transition before the command.
        before: Transition,
        /// Transition after the command.
        after: Transition,
    },
    /// The symbol table was replaced.
    Symbols {
        /// Table before the command.
        before: Vec<Symbol>,
        /// Table after the command.
        after: Vec<Symbol>,
    },
    /// The simulation panel was replaced.
    Panel {
        /// Panel before the command.
        before: SimulationPanel,
        /// Panel after the command.
        after: SimulationPanel,
    },
    /// The scan configuration was replaced.
    Scan {
        /// Configuration before the command.
        before: ScanConfig,
        /// Configuration after the command.
        after: ScanConfig,
    },
}

impl Edit {
    /// Replays the recorded change against `project`.
    pub(crate) fn replay(&self, project: &mut Project, direction: Direction) {
        match self {
            Edit::Rung { id, before, after } => {
                if let Some(rung) = project.rung_mut(*id) {
                    *rung = match direction {
                        Direction::Undo => before.clone(),
                        Direction::Redo => after.clone(),
                    };
                }
            }
            Edit::InsertRung {
                section,
                index,
                pool_index,
                rung,
            } => match direction {
                Direction::Redo => {
                    insert_rung_into_section(project, *section, *index, rung.id);
                    insert_rung_into_pool(project, *pool_index, rung.clone());
                }
                Direction::Undo => {
                    remove_rung_from_section(project, *section, rung.id);
                    remove_rung_from_pool(project, rung.id);
                }
            },
            Edit::RemoveRung {
                section,
                index,
                rung,
                pool_index,
            } => match direction {
                Direction::Undo => {
                    insert_rung_into_section(project, *section, *index, rung.id);
                    if let Some(pool_index) = pool_index {
                        insert_rung_into_pool(project, *pool_index, rung.clone());
                    }
                }
                Direction::Redo => {
                    remove_rung_from_section(project, *section, rung.id);
                    if pool_index.is_some() {
                        remove_rung_from_pool(project, rung.id);
                    }
                }
            },
            Edit::MoveRung {
                section,
                before,
                after,
            } => {
                if let Some(target) = section_mut(project, *section) {
                    target.rungs = match direction {
                        Direction::Undo => before.clone(),
                        Direction::Redo => after.clone(),
                    };
                }
            }
            Edit::InsertSection { index, section } => match direction {
                Direction::Redo => insert_section(project, *index, section.clone()),
                Direction::Undo => remove_section(project, section.id),
            },
            Edit::RemoveSection {
                index,
                section,
                rungs,
            } => match direction {
                Direction::Undo => {
                    insert_section(project, *index, section.clone());
                    for (pool_index, rung) in rungs {
                        insert_rung_into_pool(project, *pool_index, rung.clone());
                    }
                }
                Direction::Redo => {
                    remove_section(project, section.id);
                    for (_, rung) in rungs {
                        remove_rung_from_pool(project, rung.id);
                    }
                }
            },
            Edit::SectionName { id, before, after } => {
                if let Some(target) = section_mut(project, *id) {
                    target.name = match direction {
                        Direction::Undo => before.clone(),
                        Direction::Redo => after.clone(),
                    };
                }
            }
            Edit::SfcPage {
                section,
                before,
                after,
            } => {
                if let Some(target) = section_mut(project, *section) {
                    target.sequential_page = match direction {
                        Direction::Undo => before.clone(),
                        Direction::Redo => after.clone(),
                    };
                }
            }
            Edit::SfcStep {
                section,
                index,
                before,
                after,
            } => {
                if let Some(page) = page_mut(project, *section) {
                    if let Some(slot) = page.steps.get_mut(*index) {
                        *slot = match direction {
                            Direction::Undo => before.clone(),
                            Direction::Redo => after.clone(),
                        };
                    }
                }
            }
            Edit::SfcTransition {
                section,
                index,
                before,
                after,
            } => {
                if let Some(page) = page_mut(project, *section) {
                    if let Some(slot) = page.transitions.get_mut(*index) {
                        *slot = match direction {
                            Direction::Undo => before.clone(),
                            Direction::Redo => after.clone(),
                        };
                    }
                }
            }
            Edit::Symbols { before, after } => {
                project.symbols = match direction {
                    Direction::Undo => before.clone(),
                    Direction::Redo => after.clone(),
                };
            }
            Edit::Panel { before, after } => {
                project.simulation = match direction {
                    Direction::Undo => before.clone(),
                    Direction::Redo => after.clone(),
                };
            }
            Edit::Scan { before, after } => {
                project.scan = match direction {
                    Direction::Undo => *before,
                    Direction::Redo => *after,
                };
            }
        }
    }
}

/// Looks up a section by id, mutably.
fn section_mut(project: &mut Project, id: u32) -> Option<&mut Section> {
    project.sections.iter_mut().find(|section| section.id == id)
}

/// Looks up the sequential page of a section by id, mutably.
fn page_mut(project: &mut Project, id: u32) -> Option<&mut SequentialPage> {
    section_mut(project, id)?.sequential_page.as_mut()
}

/// Inserts a rung into the flat pool, clamping the position into range.
fn insert_rung_into_pool(project: &mut Project, index: usize, rung: Rung) {
    let index = index.min(project.rungs.len());
    project.rungs.insert(index, rung);
}

/// Removes a rung from the flat pool by id, if present.
fn remove_rung_from_pool(project: &mut Project, id: u32) {
    if let Some(index) = project.rungs.iter().position(|rung| rung.id == id) {
        project.rungs.remove(index);
    }
}

/// Inserts a section, clamping the position into range.
fn insert_section(project: &mut Project, index: usize, section: Section) {
    let index = index.min(project.sections.len());
    project.sections.insert(index, section);
}

/// Removes a section by id, if present.
fn remove_section(project: &mut Project, id: u32) {
    if let Some(index) = project.sections.iter().position(|section| section.id == id) {
        project.sections.remove(index);
    }
}

/// Inserts a rung id into a section's execution order, clamping the position.
fn insert_rung_into_section(project: &mut Project, section: u32, index: usize, rung: u32) {
    if let Some(target) = section_mut(project, section) {
        let index = index.min(target.rungs.len());
        target.rungs.insert(index, rung);
    }
}

/// Removes a rung id from a section's execution order, if present.
fn remove_rung_from_section(project: &mut Project, section: u32, rung: u32) {
    if let Some(target) = section_mut(project, section) {
        if let Some(index) = target.rungs.iter().position(|id| *id == rung) {
            target.rungs.remove(index);
        }
    }
}
