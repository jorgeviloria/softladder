//! The Watch tab: the variables the selected rung uses, with live values.

use egui::RichText;

use crate::app::EditorApp;
use crate::queries;

/// Draws the watch table.
pub fn show(app: &mut EditorApp, ui: &mut egui::Ui) {
    let vars = queries::watch_vars(app.project(), app.selected_rung);
    let heading = match app.selected_rung {
        Some(rung) => format!("Variables of rung #{rung}"),
        None => "Variables".to_owned(),
    };
    ui.label(RichText::new(heading).strong());
    if vars.is_empty() {
        ui.label(RichText::new("This rung uses no variables.").weak());
        return;
    }
    egui::Grid::new("watch_grid")
        .num_columns(4)
        .striped(true)
        .show(ui, |ui| {
            for heading in ["Variable", "Value", "Symbol", "Comment"] {
                ui.label(RichText::new(heading).strong());
            }
            ui.end_row();
            for var in &vars {
                ui.monospace(var.to_string());
                ui.monospace(queries::value_text(app.bench.engine().store().get(var)));
                match queries::symbol_for(app.project(), var) {
                    Some(symbol) => {
                        ui.label(symbol.name.clone());
                        ui.label(symbol.comment.clone());
                    }
                    None => {
                        ui.label("");
                        ui.label("");
                    }
                }
                ui.end_row();
            }
        });
}
