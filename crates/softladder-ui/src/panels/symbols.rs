//! The Symbols tab: the project's symbol table, editable and saved back.

use egui::{Color32, RichText};

use crate::app::{EditorApp, SymbolDraft};

/// Draws the symbol table editor.
pub fn show(app: &mut EditorApp, ui: &mut egui::Ui) {
    if app.symbols_source != app.project().symbols {
        app.reload_symbols();
    }
    ui.label(RichText::new("Symbols are part of the project and are saved with it.").weak());
    let mut remove = None;
    egui::Grid::new("symbols_grid")
        .num_columns(4)
        .striped(true)
        .show(ui, |ui| {
            for heading in ["Name", "Variable", "Comment", ""] {
                ui.label(RichText::new(heading).strong());
            }
            ui.end_row();
            for (index, draft) in app.symbols.iter_mut().enumerate() {
                ui.add(egui::TextEdit::singleline(&mut draft.name).desired_width(70.0));
                let response = ui.add(
                    egui::TextEdit::singleline(&mut draft.var)
                        .desired_width(70.0)
                        .hint_text("none"),
                );
                if !draft.var.trim().is_empty()
                    && draft.var.trim().parse::<softladder_core::VarRef>().is_err()
                {
                    response.on_hover_text("This is not a variable yet");
                }
                ui.add(egui::TextEdit::singleline(&mut draft.comment).desired_width(110.0));
                if ui
                    .small_button("✖")
                    .on_hover_text("Remove symbol")
                    .clicked()
                {
                    remove = Some(index);
                }
                ui.end_row();
            }
        });
    if let Some(index) = remove {
        if index < app.symbols.len() {
            app.symbols.remove(index);
        }
    }
    ui.horizontal_wrapped(|ui| {
        if ui.button("Add symbol").clicked() {
            app.symbols.push(SymbolDraft::default());
        }
        if ui.button("Apply").clicked() {
            app.apply_symbols();
        }
        if ui.button("Revert").clicked() {
            app.reload_symbols();
        }
    });
    if let Some(error) = app.symbols_error.clone() {
        ui.colored_label(Color32::from_rgb(230, 90, 90), error);
    }
}
