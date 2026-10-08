//! The Problems document: the diagnostics list.
//!
//! `docs/UX.md` §9: problems are surfaced in three places — the rung header, this
//! document with a count in its tab label, and the status bar — and clicking any
//! of them selects the offending network and cell. The list also carries the
//! simulation-panel warning (`SL-W020`) and the importer's warnings
//! (`SL-W030`…`SL-W033`), because a project imported from ClassicLadder must
//! report what was skipped rather than hide it.

use egui::{Align, Layout, RichText, Sense, Stroke, Ui};
use softladder_core::{Diagnostic, Project, Severity};

use crate::app::{CentreTab, EditorApp};
use crate::design::{
    empty_state, pill, section_header, Tokens, TypeScale, RADIUS_CONTROL, SPACE_1, SPACE_2,
};
use crate::panels::icons::{self, Icon};
use crate::queries;

/// One rendered row of the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProblemRow {
    /// How bad it is.
    pub severity: Severity,
    /// The diagnostic code (`SL-E001`).
    pub code: String,
    /// What went wrong.
    pub message: String,
    /// Where it is: `section Main · rung 1 start_stop`.
    pub location: String,
    /// The rung the row points at, when it resolves.
    pub rung: Option<u32>,
    /// The cell the row points at, when the message names one.
    pub cell: Option<(u8, u8)>,
}

/// The marker drawn for a severity.
pub fn severity_mark(severity: Severity) -> &'static str {
    match severity {
        Severity::Error => "⛔",
        Severity::Warning => "⚠",
        Severity::Info => "ℹ",
    }
}

/// The word written next to the marker, so colour is not the only signal.
pub fn severity_label(severity: Severity) -> &'static str {
    match severity {
        Severity::Error => "Error",
        Severity::Warning => "Warning",
        Severity::Info => "Info",
    }
}

/// The name of the section at `index`, or a numbered fallback.
fn section_name(project: &Project, index: usize) -> String {
    project
        .sections
        .get(index)
        .map(|section| section.name.clone())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| format!("section {}", index + 1))
}

/// Where a diagnostic points, as the operator reads it.
pub fn location_of(project: &Project, diagnostic: &Diagnostic) -> String {
    let Some(target) = queries::problem_target(project, diagnostic) else {
        return match diagnostic.section {
            Some(index) => section_name(project, index),
            None => "project".to_owned(),
        };
    };
    let Some(found) = queries::find_rung(project, target.rung) else {
        return format!("rung {}", target.rung);
    };
    let label = project
        .rung(target.rung)
        .map(|rung| rung.label.clone())
        .unwrap_or_default();
    let position = found.position + 1;
    let section = section_name(project, found.section);
    match &target.cell {
        Some((col, row)) => format!("{section} · rung {position} {label} · col {col} row {row}"),
        None if label.trim().is_empty() => format!("{section} · rung {position}"),
        None => format!("{section} · rung {position} {label}"),
    }
}

/// Every diagnostic the document shows: the editor's, the bench's and the
/// simulation panel's, in that order and without repeats.
pub fn diagnostics(app: &EditorApp) -> Vec<Diagnostic> {
    let mut all: Vec<Diagnostic> = app.editor().problems().to_vec();
    for diagnostic in app.bench().diagnostics() {
        if !all.contains(diagnostic) {
            all.push(diagnostic.clone());
        }
    }
    all
}

/// Builds the rendered rows for the current project.
pub fn rows(project: &Project, diagnostics: &[Diagnostic]) -> Vec<ProblemRow> {
    diagnostics
        .iter()
        .map(|diagnostic| {
            let target = queries::problem_target(project, diagnostic);
            ProblemRow {
                severity: diagnostic.severity,
                code: diagnostic.code.to_owned(),
                message: diagnostic.message.clone(),
                location: location_of(project, diagnostic),
                rung: target.map(|target| target.rung),
                cell: target.and_then(|target| target.cell),
            }
        })
        .collect()
}

/// The summary line: `2 errors, 3 warnings`.
pub fn summary(rows: &[ProblemRow]) -> String {
    let errors = rows
        .iter()
        .filter(|row| row.severity == Severity::Error)
        .count();
    let warnings = rows
        .iter()
        .filter(|row| row.severity == Severity::Warning)
        .count();
    let infos = rows.len().saturating_sub(errors + warnings);
    if rows.is_empty() {
        return "No problems".to_owned();
    }
    let mut parts = Vec::new();
    if errors > 0 {
        parts.push(format!("{errors} error{}", plural(errors)));
    }
    if warnings > 0 {
        parts.push(format!("{warnings} warning{}", plural(warnings)));
    }
    if infos > 0 {
        parts.push(format!("{infos} note{}", plural(infos)));
    }
    parts.join(", ")
}

/// `""` for one, `"s"` otherwise.
fn plural(count: usize) -> &'static str {
    if count == 1 {
        ""
    } else {
        "s"
    }
}

/// Draws the diagnostics list.
pub fn show(app: &mut EditorApp, ui: &mut Ui) {
    let tokens = app.tokens;
    section_header(ui, &tokens, "Problems");
    let diagnostics = diagnostics(app);
    let rows = rows(app.project(), &diagnostics);

    summary_line(ui, &tokens, &rows);

    if rows.is_empty() {
        empty_state(
            ui,
            &tokens,
            "No problems",
            "The program checks out: every rung the sections name exists and every element is placed.",
            "Run Online - Check the program after an edit to refresh this list.",
        );
        return;
    }

    let mut go: Option<(u32, Option<(u8, u8)>)> = None;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Grid::new("problems_table")
                .num_columns(4)
                .spacing([SPACE_2, SPACE_1])
                .striped(true)
                .min_col_width(60.0)
                .show(ui, |ui| {
                    for title in ["", "Code", "Message", "Location"] {
                        ui.label(
                            RichText::new(title)
                                .size(TypeScale::CAPTION)
                                .color(tokens.text_dim)
                                .strong(),
                        );
                    }
                    ui.end_row();

                    for row in &rows {
                        let colour = tokens.severity(row.severity);
                        let response = severity_cell(ui, colour, row.severity);
                        ui.label(
                            RichText::new(&row.code)
                                .monospace()
                                .size(TypeScale::CAPTION)
                                .color(colour)
                                .strong(),
                        );
                        ui.label(
                            RichText::new(&row.message)
                                .size(TypeScale::BODY)
                                .color(tokens.text),
                        );
                        let target = ui
                            .with_layout(Layout::right_to_left(Align::Center), |ui| {
                                let location = ui
                                    .label(
                                        RichText::new(&row.location)
                                            .size(TypeScale::CAPTION)
                                            .color(tokens.text_dim),
                                    )
                                    .on_hover_text("Click to show the offending rung");
                                if row.rung.is_some() {
                                    let (rect, _) = ui.allocate_exact_size(
                                        egui::vec2(12.0, 12.0),
                                        Sense::hover(),
                                    );
                                    icons::draw(ui.painter(), rect, &tokens, Icon::Rung);
                                }
                                location
                            })
                            .inner;
                        if (response.clicked() || target.clicked()) && row.rung.is_some() {
                            go = Some((row.rung.unwrap_or(0), row.cell));
                        }
                        ui.end_row();
                    }
                });
        });

    if let Some((rung, cell)) = go {
        app.centre_tab = CentreTab::Ladder;
        app.select(rung, cell);
    }
}

/// The severity cell: a mark and the word, so the list is readable in greyscale.
fn severity_cell(ui: &mut Ui, colour: egui::Color32, severity: Severity) -> egui::Response {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), Sense::hover());
        icons::severity_mark(ui.painter(), rect, colour, severity_mark(severity));
        ui.label(
            RichText::new(severity_label(severity))
                .size(TypeScale::CAPTION)
                .color(colour),
        );
    })
    .response
}

/// The summary line and the freshness of the list.
fn summary_line(ui: &mut Ui, tokens: &Tokens, rows: &[ProblemRow]) {
    let errors = rows
        .iter()
        .filter(|row| row.severity == Severity::Error)
        .count();
    let warnings = rows
        .iter()
        .filter(|row| row.severity == Severity::Warning)
        .count();
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new(summary(rows))
                .size(TypeScale::BODY)
                .color(tokens.text)
                .strong(),
        );
        if errors > 0 {
            pill(ui, tokens.error, &format!("{errors} error(s)"));
        }
        if warnings > 0 {
            pill(ui, tokens.warning, &format!("{warnings} warning(s)"));
        }
        if rows.is_empty() {
            pill(ui, tokens.run, "clean");
        }
    });
    ui.add_space(SPACE_1);
}

/// A framed card around the whole list, matching the other documents.
#[allow(dead_code)]
fn framed(ui: &mut Ui, tokens: &Tokens, contents: impl FnOnce(&mut Ui)) {
    egui::Frame::new()
        .fill(tokens.panel)
        .stroke(Stroke::new(1.0_f32, tokens.border))
        .corner_radius(egui::CornerRadius::same(RADIUS_CONTROL))
        .inner_margin(egui::Margin::same(SPACE_1 as i8))
        .show(ui, contents);
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
        let mut project = Project::new("problems");
        let mut main = Section::new(1, "Main");
        main.rungs.push(10);
        let mut sub = Section::new(2, "Sub");
        sub.rungs.push(404);
        project.sections.push(main);
        project.sections.push(sub);
        project.rungs.push(Rung {
            id: 10,
            label: "start_stop".to_owned(),
            elements: vec![PlacedElement::with_var(
                softladder_core::ElementKind::ContactNo,
                var("%I0"),
                0,
                0,
            )],
            ..Rung::new(10)
        });
        project.symbols.push(Symbol {
            name: "start".to_owned(),
            var: Some(var("%I0")),
            comment: String::new(),
            unit: None,
        });
        project
    }

    fn diagnostic(section: Option<usize>, rung: Option<usize>, message: &str) -> Diagnostic {
        Diagnostic {
            severity: Severity::Error,
            code: "SL-E009",
            section,
            rung,
            message: message.to_owned(),
        }
    }

    #[test]
    fn a_row_carries_the_code_the_message_and_the_location() {
        let project = project();
        let rows = rows(&project, &[diagnostic(Some(0), Some(0), "two on one cell")]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].code, "SL-E009");
        assert_eq!(rows[0].message, "two on one cell");
        assert_eq!(rows[0].location, "Main · rung 1 start_stop");
        assert_eq!(rows[0].rung, Some(10));
        assert_eq!(rows[0].cell, None);
    }

    #[test]
    fn a_duplicate_cell_diagnostic_names_the_cell() {
        let project = project();
        let rows = rows(
            &project,
            &[diagnostic(
                Some(0),
                Some(0),
                "two elements are placed on cell (col 3, row 1)",
            )],
        );
        assert_eq!(rows[0].cell, Some((3, 1)));
        assert!(rows[0].location.contains("col 3 row 1"));
    }

    #[test]
    fn a_diagnostic_that_does_not_resolve_still_has_a_location() {
        let project = project();
        // Section 1's rung 404 does not exist.
        let dangling = rows(&project, &[diagnostic(Some(1), Some(0), "dangling")]);
        assert_eq!(dangling[0].rung, None);
        assert_eq!(dangling[0].location, "Sub");
        let whole = rows(&project, &[diagnostic(None, None, "whole project")]);
        assert_eq!(whole[0].location, "project");
        // A diagnostic that names a section the project does not have still
        // reads as a section, so the row is never blank.
        let missing = rows(&project, &[diagnostic(Some(9), None, "no such section")]);
        assert_eq!(missing[0].location, "section 10");
    }

    #[test]
    fn the_location_names_the_rung_number_and_its_label() {
        let project = project();
        assert_eq!(
            location_of(&project, &diagnostic(Some(0), Some(0), "x")),
            "Main · rung 1 start_stop"
        );
        // A section with only one rung still reads as rung 1.
        assert_eq!(
            location_of(&project, &diagnostic(Some(0), Some(0), "x")),
            location_of(&project, &diagnostic(Some(0), Some(0), "y"))
        );
    }

    #[test]
    fn the_summary_counts_every_severity() {
        let project = project();
        let rows = rows(
            &project,
            &[
                diagnostic(Some(0), Some(0), "a"),
                Diagnostic {
                    severity: Severity::Warning,
                    code: "SL-W020",
                    section: None,
                    rung: None,
                    message: "b".to_owned(),
                },
                Diagnostic {
                    severity: Severity::Info,
                    code: "SL-I001",
                    section: None,
                    rung: None,
                    message: "c".to_owned(),
                },
            ],
        );
        assert_eq!(summary(&rows), "1 error, 1 warning, 1 note");
        assert_eq!(summary(&[]), "No problems");
        assert_eq!(
            summary(&rows[..1]),
            "1 error",
            "a single error has no plural"
        );
    }

    #[test]
    fn every_severity_has_a_marker_a_word_and_a_colour() {
        let tokens = Tokens::light();
        for severity in [Severity::Error, Severity::Warning, Severity::Info] {
            assert!(!severity_mark(severity).is_empty());
            assert!(!severity_label(severity).is_empty());
            assert_eq!(tokens.severity(severity).a(), 255);
        }
        assert_eq!(severity_mark(Severity::Error), "⛔");
        assert_eq!(severity_mark(Severity::Warning), "⚠");
    }

    #[test]
    fn drawing_the_problems_list_is_panic_free() {
        let mut app = EditorApp::new(project());
        app.centre_tab = CentreTab::Problems;
        crate::panels::test_frame(&ctx(), &mut app, TEST_SIZE);
        // A project whose diagnostics do not resolve, and an empty one.
        let mut broken = EditorApp::new(Project::new("broken"));
        broken.centre_tab = CentreTab::Problems;
        broken.select_rung(404, Some((0, 0)));
        crate::panels::test_frame(&ctx(), &mut broken, TEST_SIZE);
        let mut empty = EditorApp::new(Project::new("empty"));
        empty.centre_tab = CentreTab::Problems;
        crate::panels::test_frame(&ctx(), &mut empty, TEST_SIZE);
    }
}
