//! The Problems tab: the editor's diagnostics, clickable to jump to the rung.

use egui::{Color32, RichText};
use softladder_core::Severity;

use crate::app::EditorApp;
use crate::queries;

/// Draws the problem list.
pub fn show(app: &mut EditorApp, ui: &mut egui::Ui) {
    let problems = app.editor.problems().to_vec();
    if problems.is_empty() {
        ui.label(RichText::new("No problems.").weak());
        return;
    }
    let mut go: Option<(u32, Option<(u8, u8)>)> = None;
    for diagnostic in &problems {
        let target = queries::problem_target(app.project(), diagnostic);
        let color = match diagnostic.severity {
            Severity::Error => Color32::from_rgb(230, 90, 90),
            Severity::Warning => Color32::from_rgb(230, 190, 90),
            Severity::Info => Color32::from_rgb(120, 170, 230),
        };
        let severity = match diagnostic.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Info => "info",
        };
        let text = format!("{severity} [{}]: {}", diagnostic.code, diagnostic.message);
        let response = ui
            .selectable_label(false, RichText::new(text).color(color))
            .on_hover_text("Click to show the offending rung");
        if response.clicked() {
            if let Some(target) = target {
                go = Some((target.rung, target.cell));
            }
        }
    }
    if let Some((rung, cell)) = go {
        app.select_rung(rung, cell);
    }
}
