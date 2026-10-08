//! The property strip under the ribbon: the selected element, compactly.
//!
//! `docs/UX.md` §9: a value that does not parse is refused **next to the field
//! that failed**, never in a modal, and the strip is where the element under the
//! cursor is edited while the inspector is closed. It carries the element's kind,
//! its tag field with the validation message, its parameters and the
//! vertical-link toggle.

use egui::{Align, Layout, RichText, Sense, Ui};
use softladder_core::VarKind;

use crate::app::EditorApp;
use crate::design::{mono, Tokens, TypeScale, SPACE_1, SPACE_2};

/// Draws the strip.
pub fn show(app: &mut EditorApp, ui: &mut Ui) {
    let tokens = app.tokens;
    let Some(element) = app.selected_element() else {
        ui.horizontal(|ui| {
            ui.add_space(SPACE_2);
            ui.label(
                RichText::new("No element selected")
                    .size(TypeScale::CAPTION)
                    .color(tokens.text_dim),
            );
            ui.add_space(SPACE_2);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if let Some(rung) = app.selected_rung {
                    ui.label(
                        RichText::new(format!("rung {rung}"))
                            .size(TypeScale::CAPTION)
                            .color(tokens.text_dim),
                    );
                }
            });
        });
        return;
    };
    let kind = element.kind;
    let (col, row) = (element.col, element.row);
    let linked = element.connected_with_top;
    let has_params = !element.params.is_empty();

    let mut pick: Option<String> = None;
    let mut toggle_link = false;
    ui.horizontal(|ui| {
        ui.add_space(SPACE_2);
        let (icon_rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 14.0), Sense::hover());
        crate::symbols::element_glyph(ui.painter(), icon_rect, &tokens, kind);
        ui.add_space(SPACE_1);
        ui.label(
            RichText::new(crate::canvas::describe(kind))
                .size(TypeScale::BODY)
                .color(tokens.text)
                .strong(),
        );
        ui.label(
            RichText::new(format!("col {col} · row {row}"))
                .size(TypeScale::CAPTION)
                .color(tokens.text_dim),
        );
        strip_separator(ui, &tokens);

        ui.label(
            RichText::new("Tag")
                .size(TypeScale::CAPTION)
                .color(tokens.text_dim),
        );
        let response = ui.add(
            egui::TextEdit::singleline(&mut app.var_buffer)
                .desired_width(130.0)
                .hint_text("none")
                .font(egui::TextStyle::Monospace),
        );
        if commit(&response, ui) {
            app.apply_var();
        }
        if let Some(var) = used_vars_menu(app, ui) {
            pick = Some(var);
        }
        if let Some(error) = app.var_error.clone() {
            error_text(ui, &tokens, &error);
        }

        if has_params {
            strip_separator(ui, &tokens);
            ui.label(
                RichText::new("Params")
                    .size(TypeScale::CAPTION)
                    .color(tokens.text_dim),
            );
            let response = ui.add(
                egui::TextEdit::singleline(&mut app.params_buffer)
                    .desired_width(150.0)
                    .hint_text("none")
                    .font(egui::TextStyle::Monospace),
            );
            if commit(&response, ui) {
                app.apply_params();
            }
        }

        strip_separator(ui, &tokens);
        toggle_link = ui
            .selectable_label(linked, "│ link")
            .on_hover_text("Toggle the vertical link to the row above (V)")
            .clicked();

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let kind = app.selected_element().and_then(|element| element.var);
            if let Some(var) = kind {
                ui.label(
                    mono(var.kind.mnemonic())
                        .size(TypeScale::CAPTION)
                        .color(tokens.text_dim),
                );
            }
        });
    });

    if let Some(var) = pick {
        app.var_buffer = var;
        app.apply_var();
    }
    if toggle_link {
        app.toggle_vertical_link();
    }
}

/// A vertical hairline between two strip groups.
fn strip_separator(ui: &mut Ui, tokens: &Tokens) {
    ui.add_space(SPACE_2);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(1.0, 16.0), Sense::hover());
    ui.painter()
        .vline(rect.center().x, rect.y_range(), tokens.hairline());
    ui.add_space(SPACE_2);
}

/// The validation message, drawn next to the field rather than in a modal.
fn error_text(ui: &mut Ui, tokens: &Tokens, message: &str) {
    ui.add_space(SPACE_1);
    ui.label(
        RichText::new(message)
            .size(TypeScale::CAPTION)
            .color(tokens.error),
    );
}

/// A menu of the variables and tags the project already uses.
///
/// Returns the spelling the user picked, so the caller applies it through the
/// editor rather than writing the buffer directly.
fn used_vars_menu(app: &mut EditorApp, ui: &mut Ui) -> Option<String> {
    let vars = crate::queries::used_vars(app.project());
    let mut pick = None;
    ui.menu_button("used…", |ui| {
        if vars.is_empty() {
            ui.label(RichText::new("No variables in the project yet").weak());
            return;
        }
        for var in &vars {
            let label = match crate::queries::symbol_for(app.project(), var) {
                Some(symbol) => format!("{var}   ({})", symbol.name),
                None => var.to_string(),
            };
            if ui.button(label).clicked() {
                pick = Some(var.to_string());
                ui.close_menu();
            }
        }
    });
    pick
}

/// Whether a field asked to commit: focus left it, or `Enter` was pressed.
fn commit(response: &egui::Response, ui: &Ui) -> bool {
    response.lost_focus()
        || (response.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)))
}

/// The kinds a tag field can name, for the strip's hint.
#[allow(dead_code)]
pub fn kind_hint(kind: VarKind) -> &'static str {
    kind.mnemonic()
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
    use softladder_core::{ElementKind, PlacedElement, Project, Rung, Section, VarRef};

    fn var(text: &str) -> VarRef {
        text.parse().expect("variable parses")
    }

    fn project() -> Project {
        let mut project = Project::new("strip");
        let mut main = Section::new(1, "Main");
        main.rungs.push(1);
        project.sections.push(main);
        let mut rung = Rung::new(1);
        rung.elements.push(PlacedElement::with_var(
            ElementKind::ContactNo,
            var("%I0"),
            0,
            0,
        ));
        rung.elements.push(PlacedElement::with_params(
            ElementKind::Timer {
                mode: softladder_core::TimerMode::On,
            },
            2,
            0,
            &["3000"],
        ));
        project.rungs.push(rung);
        project
    }

    #[test]
    fn drawing_the_strip_for_every_selection_is_panic_free() {
        let mut app = EditorApp::new(project());
        // An element with a variable, one with parameters, and nothing.
        app.select(1, Some((0, 0)));
        crate::panels::test_frame(&ctx(), &mut app, TEST_SIZE);
        app.select(1, Some((2, 0)));
        crate::panels::test_frame(&ctx(), &mut app, TEST_SIZE);
        app.selection = Some((9, 9));
        app.load_properties();
        crate::panels::test_frame(&ctx(), &mut app, TEST_SIZE);
        app.selected_rung = None;
        crate::panels::test_frame(&ctx(), &mut app, TEST_SIZE);
        // And with a validation error in flight, which the strip shows inline.
        app.select(1, Some((0, 0)));
        app.var_error = Some("`%?` is not a variable".to_owned());
        crate::panels::test_frame(&ctx(), &mut app, TEST_SIZE);
    }

    #[test]
    fn the_strip_never_writes_the_var_buffer_without_the_editor() {
        let mut app = EditorApp::new(project());
        app.select(1, Some((0, 0)));
        let before = app.editor.history_len();
        app.var_buffer = "%I5".to_owned();
        app.apply_var();
        assert_eq!(app.editor.history_len(), before + 1);
        assert_eq!(
            app.element_at(1, 0, 0).and_then(|element| element.var),
            Some(var("%I5"))
        );
        // A spelling the model refuses is reported and changes nothing.
        let history = app.editor.history_len();
        app.var_buffer = "%?".to_owned();
        app.apply_var();
        assert!(app.var_error.is_some());
        assert_eq!(app.editor.history_len(), history);
    }

    #[test]
    fn the_kind_hint_names_the_variable_family() {
        assert_eq!(kind_hint(VarKind::PhysIn), "I");
        assert_eq!(kind_hint(VarKind::MemWord), "MW");
        assert_eq!(kind_hint(VarKind::TimerIec), "TM");
    }
}
