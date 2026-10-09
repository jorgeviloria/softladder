//! The left panel: the project tree.
//!
//! `docs/UX.md` §1 puts a **project tree** on the left of every vendor tool,
//! because editing never starts from a flat list of rungs: the tree is the map of
//! the project. This module builds that map — the PLC, its program, one node per
//! section with the language it is written in, the rungs under it as
//! `1 start_stop` with a one-line comment and an error dot when the rung has
//! problems, and then the project's documents (PLC tags, simulation bench,
//! watch & force, problems) with their counts.
//!
//! The *model* ([`TreeRow`] and [`tree`]) is a pure function of the project and
//! the problems list, so it is unit-tested without a window; the drawing only
//! walks it. Expansion state is thread-local view state (the redesign must not
//! grow [`EditorApp`]), and the inline editors reuse the editor's own buffers
//! (`EditorApp::section_name_target`, `EditorApp::rung_text_target`) so that
//! renaming a section and editing a rung's label and comment stay reachable from
//! the tree, where the old panel kept them.

use std::cell::RefCell;
use std::collections::HashSet;

use egui::{Align, Layout, RichText, Sense, Ui};
use softladder_core::{Diagnostic, Project, Section, SectionLanguage, Severity, VarRef};

use crate::app::{CentreTab, EditorApp};
use crate::design::{section_header, Tokens, TypeScale, SPACE_1, SPACE_2};
use crate::palette::SfcTool;
use crate::panels::icons::{self, Icon};
use crate::panels::problems::{self, SfcElement};
use crate::queries;

// Which project-tree nodes are folded shut.
//
// Expansion is pure view state; a node is identified by the project's own
// stable id, so folding survives an edit that renumbers the rows around it.
thread_local! {
    static COLLAPSED: RefCell<HashSet<u64>> = RefCell::new(HashSet::new());
}

/// The kind of thing a tree row is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Node {
    /// The programmable controller: the root of the project.
    Plc,
    /// The program that holds the sections.
    Program,
    /// A section, with the language it is written in.
    Section,
    /// A rung inside a ladder section.
    Rung,
    /// A page of a sequential section.
    Page,
    /// A step of a sequential page.
    Step,
    /// A transition of a sequential page.
    Transition,
    /// The PLC tag table.
    Tags,
    /// The simulation bench.
    Bench,
    /// The watch and force table.
    Watch,
    /// The diagnostics list.
    Problems,
}

impl Node {
    /// Whether the node can be expanded and collapsed.
    pub fn is_expandable(self) -> bool {
        matches!(self, Node::Plc | Node::Program | Node::Section | Node::Page)
    }

    /// A stable identity for the node, for the expansion set.
    ///
    /// A page is identified by its section *and* its page number, because two
    /// sections may both hold a page 0; the caller encodes the pair into `index`
    /// with [`page_key`].
    pub fn key(self, index: usize) -> u64 {
        match self {
            Node::Plc => 1,
            Node::Program => 2,
            Node::Section => 0x10_0000 + index as u64,
            Node::Rung => 0x20_0000 + index as u64,
            Node::Page => 0x30_0000 + index as u64,
            Node::Step => 0x40_0000 + index as u64,
            Node::Transition => 0x50_0000 + index as u64,
            Node::Tags => 3,
            Node::Bench => 4,
            Node::Watch => 5,
            Node::Problems => 6,
        }
    }
}

/// The expansion key of a page: its section index and its page number.
///
/// A page number alone is not unique across sections, and the expansion set is
/// keyed by one integer, so the pair is packed into one. `4096` is far above any
/// page number a chart stores.
pub fn page_key(section: usize, page: u32) -> usize {
    section.saturating_mul(4096).saturating_add(page as usize)
}

/// One row of the project tree, as [`tree`] produces it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeRow {
    /// What the row is.
    pub node: Node,
    /// Index of the owning section, for a section, rung, page or element row.
    pub section: Option<usize>,
    /// Id of the rung, for a rung row.
    pub rung: Option<u32>,
    /// Page number, for a sequential row.
    pub page: Option<u32>,
    /// Number of the step, for a step row.
    pub step: Option<u32>,
    /// Number of the transition, for a transition row.
    pub transition: Option<u32>,
    /// Nesting depth, `0` for the root.
    pub depth: usize,
    /// The text drawn on the row.
    pub text: String,
    /// A one-line secondary line (a rung comment, a section's language).
    pub comment: String,
    /// A count shown at the right of the row.
    pub count: Option<usize>,
    /// A count of diagnostics shown at the right in the error colour.
    pub errors: Option<usize>,
    /// Whether the row can be folded.
    pub expandable: bool,
    /// Whether the row's children are currently hidden.
    pub collapsed: bool,
}

/// Whether `node` is folded shut.
fn is_collapsed(node: Node, index: usize) -> bool {
    COLLAPSED.with(|collapsed| collapsed.borrow().contains(&node.key(index)))
}

/// Folds or unfolds `node`.
fn toggle_collapsed(node: Node, index: usize) {
    COLLAPSED.with(|collapsed| {
        let mut collapsed = collapsed.borrow_mut();
        let key = node.key(index);
        if !collapsed.remove(&key) {
            collapsed.insert(key);
        }
    });
}

/// The language label the vendors print next to a block name.
pub fn language_label(language: SectionLanguage) -> &'static str {
    match language {
        SectionLanguage::Ladder => "LAD",
        SectionLanguage::Sfc => "SFC",
    }
}

/// Shortens `text` to at most `max` characters, ending with an ellipsis.
///
/// A tree row shows one line; the full comment is in the rung's own header and
/// in the inspector, so truncating here costs nothing and keeps the tree
/// readable.
pub fn truncate(text: &str, max: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max {
        return trimmed.to_owned();
    }
    let kept: String = trimmed.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", kept.trim_end())
}

/// The rung title: its position inside the section and its label.
///
/// The number is the position the operator counts to (`1 start_stop`), not the
/// rung's internal id, which is meaningless on a printout.
pub fn rung_title(position: usize, label: &str) -> String {
    let label = label.trim();
    if label.is_empty() {
        format!("{} (unnamed)", position + 1)
    } else {
        format!("{} {}", position + 1, label)
    }
}

/// Builds the whole tree: the program with its sections and rungs, then the
/// project's documents with their counts.
///
/// `collapsed_probe` answers whether a node is folded, so the model can be
/// tested with any expansion state and the drawing can pass the thread-local
/// one.
pub fn tree(
    project: &Project,
    problems: &[Diagnostic],
    collapsed_probe: impl Fn(Node, usize) -> bool,
) -> Vec<TreeRow> {
    let collapsed_probe = &collapsed_probe;
    let root_collapsed = collapsed_probe(Node::Plc, 0);
    let mut rows = vec![TreeRow {
        node: Node::Plc,
        section: None,
        rung: None,
        page: None,
        step: None,
        transition: None,
        depth: 0,
        text: if project.name.trim().is_empty() {
            "SoftLadder PLC".to_owned()
        } else {
            project.name.clone()
        },
        comment: String::new(),
        count: None,
        errors: None,
        expandable: true,
        collapsed: root_collapsed,
    }];
    if root_collapsed {
        return rows;
    }

    let program_collapsed = collapsed_probe(Node::Program, 0);
    rows.push(TreeRow {
        node: Node::Program,
        section: None,
        rung: None,
        page: None,
        step: None,
        transition: None,
        depth: 1,
        text: "Program".to_owned(),
        comment: String::new(),
        count: Some(project.sections.len()),
        errors: None,
        expandable: true,
        collapsed: program_collapsed,
    });

    if !program_collapsed {
        for (index, section) in project.sections.iter().enumerate() {
            if section.language == SectionLanguage::Sfc {
                sequential_rows(
                    project,
                    problems,
                    collapsed_probe,
                    index,
                    section,
                    &mut rows,
                );
                continue;
            }
            let rungs = queries::section_rungs(project, index);
            let errors: usize = rungs
                .iter()
                .map(|rung| queries::rung_problem_count(project, problems, *rung))
                .sum();
            let collapsed = collapsed_probe(Node::Section, index);
            rows.push(TreeRow {
                node: Node::Section,
                section: Some(index),
                rung: None,
                page: None,
                step: None,
                transition: None,
                depth: 2,
                text: section.name.clone(),
                comment: language_label(section.language).to_owned(),
                count: Some(rungs.len()),
                errors: (errors > 0).then_some(errors),
                expandable: true,
                collapsed,
            });
            if collapsed {
                continue;
            }
            for (position, rung) in rungs.iter().enumerate() {
                let (label, comment) = project
                    .rung(*rung)
                    .map(|rung| (rung.label.clone(), rung.comment.clone()))
                    .unwrap_or_default();
                let rung_errors = queries::rung_problem_count(project, problems, *rung);
                rows.push(TreeRow {
                    node: Node::Rung,
                    section: Some(index),
                    rung: Some(*rung),
                    page: None,
                    step: None,
                    transition: None,
                    depth: 3,
                    text: rung_title(position, &label),
                    comment: truncate(&comment, 40),
                    count: None,
                    errors: (rung_errors > 0).then_some(rung_errors),
                    expandable: false,
                    collapsed: false,
                });
            }
        }
    }

    rows.push(document_row(
        Node::Tags,
        "PLC tags",
        project.symbols.len(),
        None,
    ));
    rows.push(document_row(
        Node::Bench,
        "Simulation bench",
        project.simulation.len(),
        None,
    ));
    rows.push(document_row(Node::Watch, "Watch & force", 0, None));
    let errors = problems
        .iter()
        .filter(|diagnostic| diagnostic.severity == Severity::Error)
        .count();
    rows.push(document_row(
        Node::Problems,
        "Problems",
        problems.len(),
        (errors > 0).then_some(errors),
    ));
    rows
}

/// A document row of the tree.
fn document_row(node: Node, text: &str, count: usize, errors: Option<usize>) -> TreeRow {
    TreeRow {
        node,
        section: None,
        rung: None,
        page: None,
        step: None,
        transition: None,
        depth: 1,
        text: text.to_owned(),
        comment: String::new(),
        count: Some(count),
        errors,
        expandable: false,
        collapsed: false,
    }
}

/// The pages of a sequential section, with their steps and transitions.
///
/// `docs/UX.md` §12: a sequential section shows its pages where a ladder section
/// shows its rungs, so the tree is the same map of the program in both languages.
/// A section whose page has not been drawn yet shows no children at all — the
/// document in the middle is where its "Add a page" state lives.
fn sequential_rows(
    project: &Project,
    problems: &[Diagnostic],
    collapsed_probe: &dyn Fn(Node, usize) -> bool,
    index: usize,
    section: &Section,
    rows: &mut Vec<TreeRow>,
) {
    let Some(page) = section.sequential_page.as_ref() else {
        rows.push(TreeRow {
            node: Node::Section,
            section: Some(index),
            rung: None,
            page: None,
            step: None,
            transition: None,
            depth: 2,
            text: section.name.clone(),
            comment: language_label(section.language).to_owned(),
            count: Some(0),
            errors: None,
            expandable: true,
            collapsed: collapsed_probe(Node::Section, index),
        });
        return;
    };

    let numbers = page_numbers(page);
    let collapsed = collapsed_probe(Node::Section, index);
    let errors = sequential_errors(problems, index, None, None);
    rows.push(TreeRow {
        node: Node::Section,
        section: Some(index),
        rung: None,
        page: None,
        step: None,
        transition: None,
        depth: 2,
        text: section.name.clone(),
        comment: language_label(section.language).to_owned(),
        count: Some(numbers.len()),
        errors: (errors > 0).then_some(errors),
        expandable: true,
        collapsed,
    });
    if collapsed {
        return;
    }

    for number in numbers {
        let steps: Vec<&softladder_core::Step> = page
            .steps
            .iter()
            .filter(|step| step.page == number)
            .collect();
        let transitions: Vec<&softladder_core::Transition> = page
            .transitions
            .iter()
            .filter(|transition| transition.page == number)
            .collect();
        let page_collapsed = collapsed_probe(Node::Page, page_key(index, number));
        let count = steps.len() + transitions.len();
        let page_errors = sequential_errors(problems, index, Some(number), None);
        let comment = if number == page.number {
            page.comment.clone()
        } else {
            String::new()
        };
        rows.push(TreeRow {
            node: Node::Page,
            section: Some(index),
            rung: None,
            page: Some(number),
            step: None,
            transition: None,
            depth: 3,
            text: format!("Page {number}"),
            comment: truncate(&comment, 20),
            count: Some(count),
            errors: (page_errors > 0).then_some(page_errors),
            expandable: true,
            collapsed: page_collapsed,
        });
        if page_collapsed {
            continue;
        }
        for step in steps {
            let errors = sequential_errors(
                problems,
                index,
                Some(number),
                Some(SfcElement::Step(step.number)),
            );
            rows.push(TreeRow {
                node: Node::Step,
                section: Some(index),
                rung: None,
                page: Some(number),
                step: Some(step.number),
                transition: None,
                depth: 4,
                text: if step.is_initial {
                    format!("{} · initial", step.number)
                } else {
                    format!("{} step", step.number)
                },
                comment: format!("x {} · y {}", step.x, step.y),
                count: None,
                errors: (errors > 0).then_some(errors),
                expandable: false,
                collapsed: false,
            });
        }
        for transition in transitions {
            let errors = sequential_errors(
                problems,
                index,
                Some(number),
                Some(SfcElement::Transition(transition.number)),
            );
            let condition = transition
                .condition
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_else(|| "always".to_owned());
            rows.push(TreeRow {
                node: Node::Transition,
                section: Some(index),
                rung: None,
                page: Some(number),
                step: None,
                transition: Some(transition.number),
                depth: 4,
                text: format!("T{}", transition.number),
                comment: truncate(&condition, 40),
                count: None,
                errors: (errors > 0).then_some(errors),
                expandable: false,
                collapsed: false,
            });
        }
    }
    let _ = project;
}

/// Every page number a chart uses, in ascending order.
fn page_numbers(page: &softladder_core::SequentialPage) -> Vec<u32> {
    let mut numbers = vec![page.number];
    for step in &page.steps {
        if !numbers.contains(&step.page) {
            numbers.push(step.page);
        }
    }
    for transition in &page.transitions {
        if !numbers.contains(&transition.page) {
            numbers.push(transition.page);
        }
    }
    numbers.sort_unstable();
    numbers
}

/// The diagnostics that point at a sequential row.
///
/// A page is counted when the diagnostic names it, and an element when it names
/// that element; a section with no page collects its own diagnostics.
fn sequential_errors(
    problems: &[Diagnostic],
    section: usize,
    page: Option<u32>,
    element: Option<SfcElement>,
) -> usize {
    problems
        .iter()
        .filter(|diagnostic| {
            if diagnostic.section != Some(section) {
                return false;
            }
            let Some(target) = problems::sfc_target_of(diagnostic) else {
                return false;
            };
            match (page, element) {
                (Some(page), Some(element)) => {
                    target.page == Some(page) && target.element == Some(element)
                }
                (Some(page), None) => target.page == Some(page),
                _ => true,
            }
        })
        .count()
}

/// Draws the project tree and the inline editors it opens.
pub fn show(app: &mut EditorApp, ui: &mut Ui) {
    let tokens = app.tokens;
    section_header(ui, &tokens, "Project");
    let problems = app.editor.problems().to_vec();
    let rows = tree(app.project(), &problems, is_collapsed);
    let watch_count = app.watch.len();
    let selected_rung = app.selected_rung;
    let selected_section = app.selected_section;
    let centre_tab = app.centre_tab;
    let sfc_view = crate::sfc::view();

    let mut request = TreeRequest::default();
    egui::ScrollArea::both()
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for row in &rows {
                let count = if row.node == Node::Watch {
                    Some(watch_count)
                } else {
                    row.count
                };
                let selected = match row.node {
                    Node::Section => row.section == Some(selected_section),
                    Node::Rung => row.rung.is_some() && row.rung == selected_rung,
                    Node::Page => {
                        row.section == Some(selected_section)
                            && sfc_view.page == row.page
                            && matches!(
                                sfc_view.selection,
                                Some(crate::sfc::Selection::Page(_)) | None
                            )
                    }
                    Node::Step => {
                        row.section == Some(selected_section)
                            && sfc_view.selection == row.step.map(crate::sfc::Selection::Step)
                    }
                    Node::Transition => {
                        row.section == Some(selected_section)
                            && sfc_view.selection
                                == row.transition.map(crate::sfc::Selection::Transition)
                    }
                    Node::Tags => centre_tab == CentreTab::Tags,
                    Node::Bench => centre_tab == CentreTab::Bench,
                    Node::Watch => centre_tab == CentreTab::Watch,
                    Node::Problems => centre_tab == CentreTab::Problems,
                    Node::Plc | Node::Program => false,
                };
                draw_row(ui, &tokens, row, count, selected, &mut request);
            }
        });

    request.apply(app);

    // The inline editors, both driven by the editor's own scratch buffers.
    section_rename(app, ui);
    rung_editor(app, ui);
}

/// What a click on the tree asked for, applied after the tree is drawn so the
/// editor is never borrowed while a row is being painted.
#[derive(Default)]
struct TreeRequest {
    toggle: Option<(Node, usize)>,
    select_section: Option<usize>,
    select_rung: Option<u32>,
    /// A sequential element to open: its section, its page and what to select.
    select_sequential: Option<(usize, u32, Option<crate::sfc::Selection>)>,
    centre: Option<CentreTab>,
    menu: Option<MenuChoice>,
}

/// What a row's context menu asked for.
enum MenuChoice {
    /// Open the rename editor for a section.
    RenameSection(usize),
    /// Append a new section.
    AddSection,
    /// Remove a section.
    RemoveSection(usize),
    /// Insert an empty rung at a position.
    InsertRung { section: usize, position: usize },
    /// Open the label/comment editor for a rung.
    EditRung(u32),
    /// Delete a rung.
    DeleteRung { section: usize, rung: u32 },
}

impl TreeRequest {
    /// Runs the requested edits through the editor.
    fn apply(self, app: &mut EditorApp) {
        if let Some((node, index)) = self.toggle {
            toggle_collapsed(node, index);
        }
        match self.menu {
            Some(MenuChoice::RenameSection(index)) => {
                if let Some((id, name)) = app
                    .project()
                    .sections
                    .get(index)
                    .map(|section| (section.id, section.name.clone()))
                {
                    app.section_name_buffer = name;
                    app.section_name_target = Some(id);
                }
            }
            Some(MenuChoice::AddSection) => app.add_section("New section"),
            Some(MenuChoice::RemoveSection(index)) => app.remove_section(index),
            Some(MenuChoice::InsertRung { section, position }) => {
                app.insert_rung(section, position)
            }
            Some(MenuChoice::EditRung(rung)) => {
                if let Some((label, comment)) = app
                    .project()
                    .rung(rung)
                    .map(|rung| (rung.label.clone(), rung.comment.clone()))
                {
                    app.rung_label_buffer = label;
                    app.rung_comment_buffer = comment;
                    app.rung_text_target = Some(rung);
                }
            }
            Some(MenuChoice::DeleteRung { section, rung }) => app.delete_rung(section, rung),
            None => {}
        }
        if let Some((section, page, selection)) = self.select_sequential {
            app.centre_tab = CentreTab::Ladder;
            app.select_section(section);
            crate::sfc::focus(app, page, selection);
        } else if let Some(rung) = self.select_rung {
            app.centre_tab = CentreTab::Ladder;
            app.select_rung(rung, None);
        } else if let Some(index) = self.select_section {
            app.centre_tab = CentreTab::Ladder;
            app.select_section(index);
        }
        if let Some(tab) = self.centre {
            app.centre_tab = tab;
        }
    }
}

/// Draws one row and records what the user did with it.
fn draw_row(
    ui: &mut Ui,
    tokens: &Tokens,
    row: &TreeRow,
    count: Option<usize>,
    selected: bool,
    request: &mut TreeRequest,
) {
    let height = match row.node {
        Node::Rung | Node::Step | Node::Transition => 24.0,
        _ => 26.0,
    };
    let indent = SPACE_2 + row.depth as f32 * SPACE_2;
    let full = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(egui::vec2(full, height), Sense::click());

    if selected {
        ui.painter()
            .rect_filled(rect, egui::CornerRadius::ZERO, tokens.accent_soft);
        ui.painter().rect_filled(
            egui::Rect::from_min_size(rect.min, egui::vec2(3.0, rect.height())),
            egui::CornerRadius::ZERO,
            tokens.accent,
        );
    } else if response.hovered() {
        ui.painter()
            .rect_filled(rect, egui::CornerRadius::ZERO, tokens.surface);
    }

    // A context menu turns a right-click into the edit the old panel offered as
    // a text field, so renaming a section and editing a rung's label and comment
    // stay reachable from the tree.
    let mut menu: Option<MenuChoice> = None;
    if row.node == Node::Section {
        response.context_menu(|ui| {
            menu = section_menu(ui);
        });
    } else if row.node == Node::Rung {
        let rung = row.rung.unwrap_or(0);
        let section = row.section.unwrap_or(0);
        let position = row
            .text
            .split_whitespace()
            .next()
            .and_then(|number| number.parse::<usize>().ok())
            .unwrap_or(1);
        response.context_menu(|ui| {
            menu = rung_menu(ui, section, rung, position);
        });
    }
    if let Some(choice) = menu {
        request.menu = Some(choice);
    }

    let content = egui::Rect::from_min_size(
        egui::pos2(rect.left() + indent, rect.top()),
        egui::vec2((full - indent).max(0.0), rect.height()),
    );
    ui.scope_builder(egui::UiBuilder::new().max_rect(content), |ui| {
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = SPACE_1;
            if row.expandable {
                if chevron(ui, tokens, row.collapsed) {
                    let index = match row.node {
                        Node::Page => page_key(row.section.unwrap_or(0), row.page.unwrap_or(0)),
                        _ => row.section.unwrap_or(0),
                    };
                    request.toggle = Some((row.node, index));
                }
            } else {
                ui.add_space(12.0 + SPACE_1);
            }
            draw_row_body(ui, tokens, row, count, selected);
        });
    });

    if response.clicked() {
        match row.node {
            Node::Section => request.select_section = row.section,
            Node::Rung => request.select_rung = row.rung,
            Node::Page => {
                if let (Some(section), Some(page)) = (row.section, row.page) {
                    request.select_sequential = Some((section, page, None));
                }
            }
            Node::Step => {
                if let (Some(section), Some(page), Some(step)) = (row.section, row.page, row.step) {
                    request.select_sequential =
                        Some((section, page, Some(crate::sfc::Selection::Step(step))));
                }
            }
            Node::Transition => {
                if let (Some(section), Some(page), Some(transition)) =
                    (row.section, row.page, row.transition)
                {
                    request.select_sequential = Some((
                        section,
                        page,
                        Some(crate::sfc::Selection::Transition(transition)),
                    ));
                }
            }
            Node::Tags => request.centre = Some(CentreTab::Tags),
            Node::Bench => request.centre = Some(CentreTab::Bench),
            Node::Watch => request.centre = Some(CentreTab::Watch),
            Node::Problems => request.centre = Some(CentreTab::Problems),
            Node::Program => request.centre = Some(CentreTab::Ladder),
            Node::Plc => {}
        }
    }
}

/// The icon, the text and the trailing count of a row.
fn draw_row_body(
    ui: &mut Ui,
    tokens: &Tokens,
    row: &TreeRow,
    count: Option<usize>,
    selected: bool,
) {
    let emphasis = matches!(row.node, Node::Plc | Node::Program | Node::Section);
    let size = if emphasis {
        TypeScale::EMPHASIS
    } else {
        TypeScale::BODY
    };
    let colour = if selected { tokens.accent } else { tokens.text };
    match row.node {
        Node::Plc => {
            row_icon(ui, tokens, Icon::Plc);
            ui.label(RichText::new(&row.text).size(size).color(colour).strong());
        }
        Node::Program => {
            row_icon(ui, tokens, Icon::Program);
            ui.label(RichText::new(&row.text).size(size).color(colour));
        }
        Node::Section => {
            row_icon(ui, tokens, Icon::Section);
            ui.label(RichText::new(&row.text).size(size).color(colour));
            ui.label(
                RichText::new(&row.comment)
                    .size(TypeScale::CAPTION)
                    .color(tokens.text_dim),
            );
        }
        Node::Rung => {
            row_icon(ui, tokens, Icon::Rung);
            ui.label(RichText::new(&row.text).size(size).color(colour));
            if !row.comment.is_empty() {
                ui.label(
                    RichText::new(&row.comment)
                        .size(TypeScale::CAPTION)
                        .color(tokens.text_dim),
                );
            }
        }
        Node::Page => {
            row_icon(ui, tokens, Icon::Section);
            ui.label(RichText::new(&row.text).size(size).color(colour));
            if !row.comment.is_empty() {
                ui.label(
                    RichText::new(&row.comment)
                        .size(TypeScale::CAPTION)
                        .color(tokens.text_dim),
                );
            }
        }
        Node::Step => {
            sfc_glyph(
                ui,
                tokens,
                if row.text.contains("initial") {
                    SfcTool::InitialStep
                } else {
                    SfcTool::Step
                },
            );
            ui.label(RichText::new(&row.text).size(size).color(colour));
        }
        Node::Transition => {
            sfc_glyph(ui, tokens, SfcTool::Transition);
            ui.label(RichText::new(&row.text).size(size).color(colour));
            if !row.comment.is_empty() {
                ui.label(
                    RichText::new(&row.comment)
                        .size(TypeScale::CAPTION)
                        .color(tokens.text_dim),
                );
            }
        }
        Node::Tags => {
            row_icon(ui, tokens, Icon::Tags);
            ui.label(RichText::new(&row.text).size(size).color(colour));
        }
        Node::Bench => {
            row_icon(ui, tokens, Icon::Bench);
            ui.label(RichText::new(&row.text).size(size).color(colour));
        }
        Node::Watch => {
            row_icon(ui, tokens, Icon::Watch);
            ui.label(RichText::new(&row.text).size(size).color(colour));
        }
        Node::Problems => {
            row_icon(ui, tokens, Icon::Problems);
            ui.label(RichText::new(&row.text).size(size).color(colour));
        }
    }
    row_trailing(ui, tokens, row, count);
}

/// A 12 pt sequential glyph at the left of a row, from the palette's own set.
fn sfc_glyph(ui: &mut Ui, tokens: &Tokens, tool: SfcTool) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), Sense::hover());
    crate::sfc::tool_glyph(ui.painter(), rect, tokens, tool);
}

/// A 12 pt icon at the left of a row.
fn row_icon(ui: &mut Ui, tokens: &Tokens, icon: Icon) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), Sense::hover());
    icons::draw(ui.painter(), rect, tokens, icon);
}

/// The problem marker and the count at the right of a row.
fn row_trailing(ui: &mut Ui, tokens: &Tokens, row: &TreeRow, count: Option<usize>) {
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        if let Some(errors) = row.errors {
            let text = if errors == 1 {
                "1 problem".to_owned()
            } else {
                format!("{errors} problems")
            };
            ui.label(
                RichText::new(text)
                    .size(TypeScale::CAPTION)
                    .color(tokens.error),
            );
            let (rect, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), Sense::hover());
            icons::draw(ui.painter(), rect, tokens, Icon::Error);
        }
        if let Some(count) = count {
            ui.label(
                RichText::new(count.to_string())
                    .size(TypeScale::CAPTION)
                    .color(tokens.text_dim),
            );
        }
    });
}

/// A fold chevron; returns `true` when it was clicked.
fn chevron(ui: &mut Ui, tokens: &Tokens, collapsed: bool) -> bool {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), Sense::click());
    let icon = if collapsed {
        Icon::ChevronRight
    } else {
        Icon::ChevronDown
    };
    icons::draw(ui.painter(), rect, tokens, icon);
    response.clicked()
}

/// The context menu of a section row.
///
/// Returns `None` while the menu is open and nothing has been chosen, so the
/// row's edit is applied on the frame the button is pressed and never twice.
fn section_menu(ui: &mut Ui) -> Option<MenuChoice> {
    let mut choice = None;
    if ui.button("Rename this section…").clicked() {
        choice = Some(MenuChoice::RenameSection(0));
        ui.close_menu();
    }
    if ui.button("New section").clicked() {
        choice = Some(MenuChoice::AddSection);
        ui.close_menu();
    }
    if ui.button("Remove this section").clicked() {
        choice = Some(MenuChoice::RemoveSection(0));
        ui.close_menu();
    }
    choice
}

/// The context menu of a rung row.
fn rung_menu(ui: &mut Ui, section: usize, rung: u32, position: usize) -> Option<MenuChoice> {
    let mut choice = None;
    if ui.button("Edit the label and the comment…").clicked() {
        choice = Some(MenuChoice::EditRung(rung));
        ui.close_menu();
    }
    if ui.button("Insert a rung below").clicked() {
        choice = Some(MenuChoice::InsertRung { section, position });
        ui.close_menu();
    }
    if ui.button("Delete this rung").clicked() {
        choice = Some(MenuChoice::DeleteRung { section, rung });
        ui.close_menu();
    }
    choice
}

/// The section rename editor, drawn while a rename is in progress.
fn section_rename(app: &mut EditorApp, ui: &mut Ui) {
    let Some(id) = app.section_name_target else {
        return;
    };
    let index = app
        .project()
        .sections
        .iter()
        .position(|section| section.id == id);
    let Some(index) = index else {
        app.section_name_target = None;
        return;
    };
    let tokens = app.tokens;
    let mut commit = false;
    let mut cancel = false;
    egui::Frame::new()
        .fill(tokens.surface)
        .stroke(tokens.hairline())
        .corner_radius(egui::CornerRadius::same(crate::design::RADIUS_CONTROL))
        .inner_margin(egui::Margin::same(SPACE_2 as i8))
        .show(ui, |ui| {
            ui.label(
                RichText::new("Section name")
                    .size(TypeScale::CAPTION)
                    .color(tokens.text_dim),
            );
            let response = ui.add(
                egui::TextEdit::singleline(&mut app.section_name_buffer)
                    .desired_width(ui.available_width()),
            );
            if response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                commit = true;
            }
            ui.horizontal(|ui| {
                if ui.button("Rename").clicked() {
                    commit = true;
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
        });
    if commit {
        let name = app.section_name_buffer.clone();
        app.rename_section(index, &name);
        app.section_name_target = None;
    } else if cancel {
        app.section_name_target = None;
    }
}

/// The rung label/comment editor, drawn while an edit is in progress.
fn rung_editor(app: &mut EditorApp, ui: &mut Ui) {
    let Some(rung) = app.rung_text_target else {
        return;
    };
    if app.project().rung(rung).is_none() {
        app.rung_text_target = None;
        return;
    }
    let tokens = app.tokens;
    let mut commit = false;
    let mut cancel = false;
    egui::Frame::new()
        .fill(tokens.surface)
        .stroke(tokens.hairline())
        .corner_radius(egui::CornerRadius::same(crate::design::RADIUS_CONTROL))
        .inner_margin(egui::Margin::same(SPACE_2 as i8))
        .show(ui, |ui| {
            egui::Grid::new("rung_text_editor")
                .num_columns(2)
                .spacing([SPACE_2, SPACE_1])
                .show(ui, |ui| {
                    ui.label(
                        RichText::new("Label")
                            .size(TypeScale::CAPTION)
                            .color(tokens.text_dim),
                    );
                    ui.add(
                        egui::TextEdit::singleline(&mut app.rung_label_buffer).desired_width(140.0),
                    );
                    ui.end_row();
                    ui.label(
                        RichText::new("Comment")
                            .size(TypeScale::CAPTION)
                            .color(tokens.text_dim),
                    );
                    ui.add(
                        egui::TextEdit::singleline(&mut app.rung_comment_buffer)
                            .desired_width(140.0),
                    );
                    ui.end_row();
                });
            ui.horizontal(|ui| {
                if ui.button("Save").clicked() {
                    commit = true;
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
        });
    if commit {
        let label = app.rung_label_buffer.clone();
        let comment = app.rung_comment_buffer.clone();
        app.set_rung_text(rung, &label, &comment);
        app.rung_text_target = None;
    } else if cancel {
        app.rung_text_target = None;
    }
}

/// The variables of the rung the canvas is showing, in the order it shows them.
///
/// The tree prints them under the selected rung, so the operator can see the
/// signals of the network without opening the watch table.
/// The variables the selected rung uses, in first-use order.
///
/// The canvas shows them in place, so the tree no longer prints them; the helper
/// stays because it is the natural way for a panel to ask.
#[allow(dead_code)]
pub fn selected_rung_vars(app: &EditorApp) -> Vec<VarRef> {
    let Some(rung) = app.selected_rung_ref() else {
        return Vec::new();
    };
    let mut vars: Vec<VarRef> = Vec::new();
    for element in &rung.elements {
        if let Some(var) = element.var.as_ref() {
            if !vars.contains(var) {
                vars.push(var.clone());
            }
        }
    }
    vars
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The window the headless frames are laid out in.
    const TEST_SIZE: egui::Vec2 = egui::vec2(1440.0, 900.0);

    /// A context for the headless frames, shared by the tests of this module.
    fn ctx() -> egui::Context {
        egui::Context::default()
    }
    use softladder_core::{PlacedElement, Rung, Section, Symbol, VarRef};

    fn var(text: &str) -> VarRef {
        text.parse().expect("variable parses")
    }

    fn project() -> Project {
        let mut project = Project::new("traffic");
        let mut main = Section::new(1, "Main");
        main.rungs.push(10);
        main.rungs.push(20);
        let mut sub = Section::new(2, "Sub");
        sub.language = SectionLanguage::Sfc;
        project.sections.push(main);
        project.sections.push(sub);
        project.rungs.push(Rung {
            id: 10,
            label: "start_stop".to_owned(),
            comment: "Start/stop with self-hold, as the panel shows it".to_owned(),
            elements: vec![PlacedElement::with_var(
                softladder_core::ElementKind::ContactNo,
                var("%I0"),
                0,
                0,
            )],
            ..Rung::new(10)
        });
        project.rungs.push(Rung {
            id: 20,
            ..Rung::new(20)
        });
        project.symbols.push(Symbol {
            name: "start".to_owned(),
            var: Some(var("%I0")),
            comment: String::new(),
            unit: None,
        });
        project
            .simulation
            .switches
            .push(softladder_core::SimSwitch {
                var: var("%I1"),
                label: "start".to_owned(),
                momentary: false,
            });
        project
    }

    fn problem(section: usize, position: usize) -> Diagnostic {
        Diagnostic {
            severity: Severity::Error,
            code: "SL-E001",
            section: Some(section),
            rung: Some(position),
            message: "boom".to_owned(),
        }
    }

    #[test]
    fn the_tree_reads_like_the_vendor_trees() {
        let project = project();
        let problems = vec![problem(0, 0)];
        let rows = tree(&project, &problems, |_, _| false);
        let nodes: Vec<Node> = rows.iter().map(|row| row.node).collect();
        assert_eq!(
            nodes,
            vec![
                Node::Plc,
                Node::Program,
                Node::Section,
                Node::Rung,
                Node::Rung,
                Node::Section,
                Node::Tags,
                Node::Bench,
                Node::Watch,
                Node::Problems,
            ]
        );
        // The root carries the project's name, the sections their language.
        assert_eq!(rows[0].text, "traffic");
        assert_eq!(rows[0].depth, 0);
        assert_eq!(rows[2].text, "Main");
        assert_eq!(rows[2].comment, "LAD");
        assert_eq!(rows[5].comment, "SFC");
        // A rung is "number label", with the comment truncated to one line.
        assert_eq!(rows[3].text, "1 start_stop");
        assert_eq!(rows[4].text, "2 (unnamed)");
        assert!(rows[3].comment.chars().count() <= 40);
        assert!(rows[3].comment.ends_with('…'), "a long comment is elided");
        // The counts: rungs per section, tags, bench widgets, problems.
        assert_eq!(rows[2].count, Some(2));
        assert_eq!(rows[6].count, Some(1), "one tag");
        assert_eq!(rows[7].count, Some(1), "one bench widget");
        assert_eq!(rows[9].count, Some(1), "one problem");
        // The error dot lands on the offending rung and on its section.
        assert_eq!(rows[3].errors, Some(1));
        assert_eq!(rows[2].errors, Some(1));
        assert_eq!(rows[4].errors, None);
    }

    #[test]
    fn a_collapsed_node_hides_its_children() {
        let project = project();
        let collapsed_program = tree(&project, &[], |node, _| node == Node::Program);
        let nodes: Vec<Node> = collapsed_program.iter().map(|row| row.node).collect();
        assert_eq!(
            nodes,
            vec![
                Node::Plc,
                Node::Program,
                Node::Tags,
                Node::Bench,
                Node::Watch,
                Node::Problems
            ],
            "a folded program hides the sections but keeps the documents"
        );

        let collapsed_root = tree(&project, &[], |node, _| node == Node::Plc);
        assert_eq!(collapsed_root.len(), 1);

        let collapsed_section = tree(&project, &[], |node, index| {
            node == Node::Section && index == 0
        });
        let rungs = collapsed_section
            .iter()
            .filter(|row| row.node == Node::Rung)
            .count();
        assert_eq!(rungs, 0, "the folded section hides its rungs");
    }

    #[test]
    fn an_empty_project_still_has_a_root_and_the_documents() {
        let rows = tree(&Project::new(""), &[], |_, _| false);
        assert_eq!(rows[0].text, "SoftLadder PLC");
        assert_eq!(rows[1].text, "Program");
        assert_eq!(rows[1].count, Some(0));
        assert!(rows.iter().any(|row| row.node == Node::Tags));
        assert!(rows.iter().any(|row| row.node == Node::Problems));
    }

    #[test]
    fn a_section_that_references_a_missing_rung_is_skipped_not_panicked() {
        let mut project = Project::new("broken");
        let mut section = Section::new(1, "Main");
        section.rungs.push(404);
        project.sections.push(section);
        let rows = tree(&project, &[], |_, _| false);
        assert!(!rows.iter().any(|row| row.node == Node::Rung));
        assert_eq!(rows[2].count, Some(0));
    }

    #[test]
    fn comments_are_truncated_to_a_single_line() {
        assert_eq!(truncate("short", 10), "short");
        assert_eq!(truncate("  padded  ", 10), "padded");
        assert_eq!(truncate("a very long comment", 8), "a very…");
        assert_eq!(truncate("", 8), "");
        // Multi-byte characters are counted, not sliced.
        assert_eq!(truncate("áéíóú", 3), "áé…");
    }

    #[test]
    fn rung_titles_use_the_position_not_the_id() {
        assert_eq!(rung_title(0, "start_stop"), "1 start_stop");
        assert_eq!(rung_title(2, "  "), "3 (unnamed)");
        assert_eq!(rung_title(9, "last"), "10 last");
    }

    #[test]
    fn every_language_has_a_label() {
        assert_eq!(language_label(SectionLanguage::Ladder), "LAD");
        assert_eq!(language_label(SectionLanguage::Sfc), "SFC");
    }

    #[test]
    fn node_keys_are_unique_across_the_whole_tree() {
        let nodes = [
            Node::Plc,
            Node::Program,
            Node::Section,
            Node::Rung,
            Node::Page,
            Node::Step,
            Node::Transition,
            Node::Tags,
            Node::Bench,
            Node::Watch,
            Node::Problems,
        ];
        let mut keys: Vec<u64> = nodes.iter().map(|node| node.key(0)).collect();
        keys.push(Node::Section.key(1));
        keys.push(Node::Page.key(page_key(1, 0)));
        let count = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), count, "two nodes share an identity");
        assert_ne!(Node::Section.key(0), Node::Section.key(1));
        assert_ne!(
            Node::Page.key(page_key(0, 0)),
            Node::Page.key(page_key(1, 0))
        );
    }

    #[test]
    fn every_expandable_node_is_drawn_with_a_chevron() {
        assert!(Node::Plc.is_expandable());
        assert!(Node::Program.is_expandable());
        assert!(Node::Section.is_expandable());
        assert!(Node::Page.is_expandable());
        assert!(!Node::Rung.is_expandable());
        assert!(!Node::Step.is_expandable());
        assert!(!Node::Transition.is_expandable());
        assert!(!Node::Tags.is_expandable());
    }

    /// A project with a sequential chart: one page, two steps, a transition
    /// between them and a second step on another page.
    fn sfc_project() -> Project {
        use softladder_core::{SequentialPage, Step, Transition};
        let mut project = project();
        let mut page = SequentialPage::new(0, "start-up and stop");
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
            x: 1,
            y: 0,
            page: 5,
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
        if let Some(section) = project.sections.get_mut(1) {
            section.sequential_page = Some(page);
        }
        project
    }

    /// A diagnostic that points into a sequential chart.
    fn sfc_problem(page: u32, element: &str) -> Diagnostic {
        Diagnostic {
            severity: Severity::Warning,
            code: "SL-W011",
            section: Some(1),
            rung: None,
            message: format!(
                "SFC section `Sub` page {page} {element}: no condition, so it fires whenever its \
                 source steps are active"
            ),
        }
    }

    #[test]
    fn a_sequential_section_shows_its_pages_instead_of_rungs() {
        let project = sfc_project();
        let rows = tree(&project, &[], |_, _| false);
        let nodes: Vec<Node> = rows.iter().map(|row| row.node).collect();
        assert_eq!(
            nodes,
            vec![
                Node::Plc,
                Node::Program,
                Node::Section,
                Node::Rung,
                Node::Rung,
                Node::Section,
                Node::Page,
                Node::Step,
                Node::Step,
                Node::Transition,
                Node::Page,
                Node::Step,
                Node::Tags,
                Node::Bench,
                Node::Watch,
                Node::Problems,
            ]
        );
        // The sequential section counts its pages, not its rungs.
        let section = rows
            .iter()
            .find(|row| row.node == Node::Section && row.section == Some(1))
            .expect("the sequential section");
        assert_eq!(section.comment, "SFC");
        assert_eq!(section.count, Some(2), "two pages");
        // A page names its comment and counts its elements.
        let first = &rows[6];
        assert_eq!(first.text, "Page 0");
        assert_eq!(first.comment, "start-up and stop");
        assert_eq!(first.count, Some(3));
        assert_eq!(first.page, Some(0));
        assert!(first.expandable);
        // Steps and transitions name their number and cell.
        assert_eq!(rows[7].text, "0 · initial");
        assert_eq!(rows[7].step, Some(0));
        assert_eq!(rows[8].text, "1 step");
        assert_eq!(rows[8].comment, "x 0 · y 2");
        assert_eq!(rows[9].text, "T0");
        assert_eq!(rows[9].transition, Some(0));
        assert_eq!(rows[9].comment, "%I0");
        // The second page holds its own step and no comment of its own.
        assert_eq!(rows[10].text, "Page 5");
        assert_eq!(rows[10].comment, "");
        assert_eq!(rows[10].count, Some(1));
        assert_eq!(rows[11].step, Some(2));
    }

    #[test]
    fn sequential_diagnostics_land_on_the_element_they_name() {
        let project = sfc_project();
        let problems = vec![sfc_problem(0, "transition 0"), sfc_problem(0, "step 1")];
        let rows = tree(&project, &problems, |_, _| false);
        let section = rows
            .iter()
            .find(|row| row.node == Node::Section && row.section == Some(1))
            .expect("the section");
        assert_eq!(section.errors, Some(2), "both problems belong to it");
        let page = rows
            .iter()
            .find(|row| row.node == Node::Page && row.page == Some(0))
            .expect("page 0");
        assert_eq!(page.errors, Some(2));
        let step = rows
            .iter()
            .find(|row| row.node == Node::Step && row.step == Some(1))
            .expect("step 1");
        assert_eq!(step.errors, Some(1));
        let transition = rows
            .iter()
            .find(|row| row.node == Node::Transition)
            .expect("the transition");
        assert_eq!(transition.errors, Some(1));
        // The step that was not named stays clean.
        let other = rows
            .iter()
            .find(|row| row.node == Node::Step && row.step == Some(0))
            .expect("step 0");
        assert_eq!(other.errors, None);
        // The transition of the other page is unaffected by page 0's problems.
        let second = rows
            .iter()
            .find(|row| row.node == Node::Page && row.page == Some(5))
            .expect("page 5");
        assert_eq!(second.errors, None);
    }

    #[test]
    fn folding_a_page_or_a_sequential_section_hides_its_children() {
        let project = sfc_project();
        let folded_page = tree(&project, &[], |node, index| {
            node == Node::Page && index == page_key(1, 0)
        });
        assert!(
            !folded_page
                .iter()
                .any(|row| row.node == Node::Step && row.page == Some(0)),
            "the folded page hides its steps"
        );
        assert!(
            folded_page
                .iter()
                .any(|row| row.node == Node::Step && row.page == Some(5)),
            "the other page stays open"
        );

        let folded_section = tree(&project, &[], |node, index| {
            node == Node::Section && index == 1
        });
        assert!(!folded_section.iter().any(|row| row.node == Node::Page));

        // A page of two sections is two independently foldable nodes.
        assert_ne!(page_key(0, 0), page_key(1, 0));
        assert_eq!(page_key(1, 7), 4096 + 7);
    }

    #[test]
    fn a_sequential_section_with_no_page_has_no_children() {
        let project = project();
        let rows = tree(&project, &[], |_, _| false);
        assert!(
            !rows
                .iter()
                .any(|row| { matches!(row.node, Node::Page | Node::Step | Node::Transition) }),
            "an undrawn chart shows nothing under its section"
        );
        let section = rows
            .iter()
            .find(|row| row.node == Node::Section && row.section == Some(1))
            .expect("the sequential section");
        assert_eq!(section.count, Some(0));
    }

    #[test]
    fn drawing_the_tree_of_a_sequential_project_is_panic_free() {
        let mut app = EditorApp::new(sfc_project());
        crate::sfc::open(&mut app, 1);
        crate::sfc::focus(&mut app, 0, Some(crate::sfc::Selection::Step(0)));
        crate::panels::test_frame(&ctx(), &mut app, TEST_SIZE);
        crate::sfc::focus(&mut app, 0, Some(crate::sfc::Selection::Transition(0)));
        crate::panels::test_frame(&ctx(), &mut app, TEST_SIZE);
        crate::sfc::reset_view();
    }

    #[test]
    fn drawing_the_tree_is_panic_free_for_an_empty_project() {
        let mut app = EditorApp::new(Project::new("empty"));
        crate::panels::test_frame(&ctx(), &mut app, TEST_SIZE);
        app.select_rung(404, Some((0, 0)));
        crate::panels::test_frame(&ctx(), &mut app, TEST_SIZE);
    }
}
