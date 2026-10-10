//! The status bar.
//!
//! `docs/UX.md` §3 and §9: one line, well spaced, never a run-on sentence. Left
//! to right it carries **one mode pill** (`SIM RUN`, `SIM SCAN`, `SIM STOP`,
//! `FREEZE`, `LOAD`), the cycle count, the last scan time and the project path
//! with a `*` while it is dirty; on the right the problem counts (clickable,
//! and they open the Problems document), the forced-values indicator and the
//! canvas zoom. The mode is a single word because a scan must never sit next
//! to an "offline" badge.

use egui::{Align, Color32, Layout, RichText, Sense, Ui};
use softladder_core::Severity;

use crate::app::{CentreTab, EditorApp};
use crate::design::{mono, pill, quiet_pill, Tokens, TypeScale, SPACE_1, SPACE_2, SPACE_3};
use crate::panels::icons::{self, Icon};
use crate::queries;

/// One piece of the left-hand run of the status bar, kept pure so the wording is
/// pinned by a test.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    /// The single mode pill (`SIM RUN`, `SIM SCAN`, `SIM STOP`, …).
    Mode(&'static str),
    /// The cycle counter.
    Cycles(u64),
    /// The last scan time.
    Scan(u32),
    /// The project path, with a `*` while it is dirty.
    Place(String),
}

/// The colour of the mode pill.
pub fn mode_colour(tokens: &Tokens, state: softladder_edit::RuntimeState) -> Color32 {
    use softladder_edit::RuntimeState;
    match state {
        RuntimeState::Run => tokens.run,
        RuntimeState::RunOneCycle => tokens.accent,
        RuntimeState::Stop => tokens.stop,
        RuntimeState::Freeze => tokens.warning,
        RuntimeState::Loading => tokens.text_dim,
    }
}

/// The project path, with a `*` while there are unsaved changes.
pub fn place_text(path: Option<&std::path::Path>, dirty: bool) -> String {
    let name = path
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "untitled".to_owned());
    if dirty {
        format!("{name} *")
    } else {
        name
    }
}

/// The left-hand run of the status bar.
pub fn segments(
    state: softladder_edit::RuntimeState,
    cycles: u64,
    last_scan_ms: f64,
    path: Option<&std::path::Path>,
    dirty: bool,
) -> Vec<Segment> {
    let scan = if last_scan_ms.is_finite() {
        (last_scan_ms.max(0.0) * 100.0).round() as u32
    } else {
        0
    };
    vec![
        Segment::Mode(queries::run_state_text(state)),
        Segment::Cycles(cycles),
        Segment::Scan(scan),
        Segment::Place(place_text(path, dirty)),
    ]
}

/// Draws the status bar.
pub fn show(app: &mut EditorApp, ui: &mut Ui) {
    let tokens = app.tokens;
    let state = app.bench.state();
    let problems = app.editor().problems();
    let errors = problems
        .iter()
        .filter(|diagnostic| diagnostic.severity == Severity::Error)
        .count();
    let warnings = problems
        .iter()
        .filter(|diagnostic| diagnostic.severity == Severity::Warning)
        .count();
    let info = problems.len().saturating_sub(errors + warnings);

    let mut show_problems = false;
    ui.horizontal(|ui| {
        ui.add_space(SPACE_2);
        let _ = pill(
            ui,
            mode_colour(&tokens, state),
            queries::run_state_text(state),
        )
        .on_hover_text(mode_tip(state));
        ui.add_space(SPACE_3);
        metric(ui, &tokens, "cycles", &app.bench().cycles().to_string());
        ui.add_space(SPACE_3);
        metric(
            ui,
            &tokens,
            "scan",
            &format!("{:.2} ms", app.last_scan_ms.max(0.0)),
        );
        ui.add_space(SPACE_3);
        ui.label(
            RichText::new(place_text(app.editor().path(), app.editor().is_dirty()))
                .size(TypeScale::CAPTION)
                .color(tokens.text_dim),
        );
        if !app.status().is_empty() {
            ui.add_space(SPACE_3);
            let last = ui.painter().add(egui::Shape::Noop);
            let (rect, _) = ui.allocate_exact_size(egui::vec2(1.0, 14.0), egui::Sense::hover());
            ui.painter().set(
                last,
                egui::Shape::line_segment(
                    [rect.center_top(), rect.center_bottom()],
                    tokens.hairline(),
                ),
            );
            ui.add_space(SPACE_1);
            ui.label(
                RichText::new(app.status())
                    .size(TypeScale::CAPTION)
                    .color(tokens.text),
            );
        }

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.add_space(SPACE_2);
            ui.label(
                RichText::new(format!("{:.0}%", app.camera.zoom * 100.0))
                    .size(TypeScale::CAPTION)
                    .color(tokens.text_dim),
            )
            .on_hover_text("Canvas zoom");
            if !app.forces.is_empty() {
                ui.add_space(SPACE_2);
                quiet_pill(ui, tokens.warning, &format!("{} forced", app.forces.len()));
            }
            ui.add_space(SPACE_2);
            show_problems = count_pill(ui, &tokens, Icon::Error, errors, tokens.error, "error");
            if count_pill(
                ui,
                &tokens,
                Icon::Warning,
                warnings,
                tokens.warning,
                "warning",
            ) {
                show_problems = true;
            }
            if info > 0 && count_pill(ui, &tokens, Icon::Info, info, tokens.text_dim, "note") {
                show_problems = true;
            }
            if problems.is_empty() {
                quiet_pill(ui, tokens.run, "no problems");
            }
        });
    });

    if show_problems {
        app.centre_tab = CentreTab::Problems;
    }
}

/// What the mode pill means, for the tooltip.
fn mode_tip(state: softladder_edit::RuntimeState) -> &'static str {
    use softladder_edit::RuntimeState;
    match state {
        RuntimeState::Run => "The simulator is scanning",
        RuntimeState::RunOneCycle => "The simulator is running a single scan",
        RuntimeState::Stop => "The simulator is stopped",
        RuntimeState::Freeze => "The simulator is frozen",
        RuntimeState::Loading => "The simulator is loading the program",
    }
}

/// A `name value` pair, with the value in monospace.
fn metric(ui: &mut Ui, tokens: &Tokens, name: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(name)
                .size(TypeScale::CAPTION)
                .color(tokens.text_dim),
        );
        ui.label(mono(value).color(tokens.text));
    });
}

/// A clickable problem count; returns whether it was clicked.
fn count_pill(
    ui: &mut Ui,
    tokens: &Tokens,
    icon: Icon,
    count: usize,
    colour: Color32,
    word: &str,
) -> bool {
    if count == 0 {
        return false;
    }
    let text = format!("{count} {word}{}", if count == 1 { "" } else { "s" });
    let response = ui
        .add(
            egui::Label::new(
                RichText::new(text)
                    .size(TypeScale::CAPTION)
                    .color(colour)
                    .strong(),
            )
            .sense(Sense::click()),
        )
        .on_hover_text("Show the Problems document");
    let rect = response.rect;
    ui.painter().rect_filled(
        rect.expand2(egui::vec2(6.0, 2.0)),
        egui::CornerRadius::same(2),
        colour.gamma_multiply(0.12),
    );
    let (icon_rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), Sense::hover());
    icons::draw(ui.painter(), icon_rect, tokens, icon);
    response.clicked()
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
    use softladder_edit::RuntimeState;
    use std::path::Path;

    #[test]
    fn the_status_bar_reads_as_separate_segments_not_a_sentence() {
        let path = Path::new("/tmp/plant.slprj");
        let parts = segments(RuntimeState::Run, 412, 0.031, Some(path), false);
        assert_eq!(
            parts,
            vec![
                Segment::Mode("SIM RUN"),
                Segment::Cycles(412),
                Segment::Scan(3),
                Segment::Place("/tmp/plant.slprj".to_owned()),
            ]
        );
    }

    #[test]
    fn a_stopped_bench_says_so_and_a_dirty_project_is_marked() {
        let path = Path::new("/tmp/plant.slprj");
        let parts = segments(RuntimeState::Stop, 0, 0.0, Some(path), true);
        assert_eq!(parts[0], Segment::Mode("SIM STOP"));
        assert!(matches!(&parts[3], Segment::Place(text) if text.ends_with(" *")));

        // No path at all still says something honest.
        let parts = segments(RuntimeState::Stop, 0, 0.0, None, false);
        assert_eq!(parts[3], Segment::Place("untitled".to_owned()));
    }

    #[test]
    fn a_single_scan_is_not_reported_as_offline() {
        let parts = segments(RuntimeState::RunOneCycle, 7, 0.0, None, false);
        assert_eq!(parts[0], Segment::Mode("SIM SCAN"));
    }

    #[test]
    fn a_nonsense_scan_time_is_rendered_as_zero() {
        for value in [f64::NAN, f64::INFINITY, -4.0] {
            let parts = segments(RuntimeState::Run, 0, value, None, false);
            assert_eq!(parts[2], Segment::Scan(0), "{value} did not clamp");
        }
        let parts = segments(RuntimeState::Run, 0, 1.5, None, false);
        assert_eq!(parts[2], Segment::Scan(150));
    }

    #[test]
    fn every_lifecycle_state_has_a_label_and_a_colour() {
        let tokens = Tokens::light();
        for state in [
            RuntimeState::Loading,
            RuntimeState::Stop,
            RuntimeState::Run,
            RuntimeState::RunOneCycle,
            RuntimeState::Freeze,
        ] {
            assert!(!queries::run_state_text(state).is_empty());
            assert_eq!(mode_colour(&tokens, state).a(), 255);
            assert!(!mode_tip(state).is_empty());
        }
        assert_eq!(queries::run_state_text(RuntimeState::Run), "SIM RUN");
        assert_eq!(mode_colour(&tokens, RuntimeState::Run), tokens.run);
        assert_eq!(mode_colour(&tokens, RuntimeState::Stop), tokens.stop);
        assert_eq!(mode_colour(&tokens, RuntimeState::Freeze), tokens.warning);
        assert_eq!(
            mode_colour(&tokens, RuntimeState::RunOneCycle),
            tokens.accent
        );
    }

    #[test]
    fn drawing_the_status_bar_is_panic_free() {
        use softladder_core::Project;
        let mut app = EditorApp::new(Project::new("empty"));
        crate::panels::test_frame(&ctx(), &mut app, TEST_SIZE);
        app.centre_tab = CentreTab::Problems;
        app.camera.set_zoom(2.0);
        app.forces
            .push(("%Q0".parse().expect("variable parses"), true));
        app.note("something happened");
        crate::panels::test_frame(&ctx(), &mut app, TEST_SIZE);
    }
}
