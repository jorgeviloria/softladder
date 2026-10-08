//! The property strip above the canvas: the selected element's variable and
//! parameters, with validation feedback.

use egui::{Color32, RichText};

use crate::app::EditorApp;
use crate::queries;

/// Draws the strip.
pub fn show(app: &mut EditorApp, ui: &mut egui::Ui) {
    ui.horizontal_wrapped(|ui| {
        let Some(element) = app.selected_element() else {
            ui.label(RichText::new("No element selected").weak());
            return;
        };
        ui.label(RichText::new(crate::canvas::describe(element.kind)).strong());
        ui.label(format!("({}, {})", element.col, element.row));
        ui.separator();

        ui.label("var");
        let response = ui.add(
            egui::TextEdit::singleline(&mut app.var_buffer)
                .desired_width(110.0)
                .hint_text("none"),
        );
        let commit = response.lost_focus()
            || (response.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)));
        if commit {
            app.apply_var();
        }
        used_vars_menu(app, ui);
        if let Some(error) = app.var_error.clone() {
            ui.colored_label(Color32::from_rgb(230, 90, 90), error);
        }

        ui.separator();
        ui.label("params");
        let response = ui.add(
            egui::TextEdit::singleline(&mut app.params_buffer)
                .desired_width(150.0)
                .hint_text("none"),
        );
        let commit = response.lost_focus()
            || (response.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)));
        if commit {
            app.apply_params();
        }
    });
}

/// A menu listing the variables the project already uses, plus their symbols.
fn used_vars_menu(app: &mut EditorApp, ui: &mut egui::Ui) {
    let vars = queries::used_vars(app.project());
    ui.menu_button("used…", |ui| {
        if vars.is_empty() {
            ui.label(RichText::new("No variables in the project yet").weak());
            return;
        }
        for var in &vars {
            let label = match queries::symbol_for(app.project(), var) {
                Some(symbol) => format!("{var}   ({})", symbol.name),
                None => var.to_string(),
            };
            if ui.button(label).clicked() {
                app.var_buffer = var.to_string();
                app.apply_var();
                ui.close_menu();
            }
        }
    });
}
