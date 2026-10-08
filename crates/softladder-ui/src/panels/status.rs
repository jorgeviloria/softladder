//! The bottom status bar: run state, cycles, scan time, path and problems.

use egui::RichText;

use crate::app::EditorApp;
use crate::queries;

/// Draws the status bar.
pub fn show(app: &mut EditorApp, ui: &mut egui::Ui) {
    let (errors, warnings) = queries::problem_counts(app.editor.problems());
    let text = queries::status_text(
        app.bench.state(),
        app.bench.cycles(),
        app.last_scan_ms,
        app.editor.path(),
        app.editor.is_dirty(),
        errors,
        warnings,
    );
    ui.horizontal(|ui| {
        ui.monospace(text);
        if !app.status().is_empty() {
            ui.separator();
            ui.label(RichText::new(app.status()).weak());
        }
    });
}
