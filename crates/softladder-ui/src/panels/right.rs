//! The right-hand inspector: the properties of whatever is selected.
//!
//! `docs/UX.md` §3 puts the **inspector** under the project tree, and §9 says
//! nothing may be silent: a value that does not parse is refused next to the
//! field that failed, not in a modal. The inspector is *context sensitive* — the
//! element under the cursor, the selected rung, the selected section or the
//! selected bench widget — and it deliberately does **not** repeat the Bench,
//! Watch, Problems or PLC-tags documents, which are centre documents now.

use egui::{RichText, Sense, Ui};
use softladder_core::{SectionLanguage, SimulationPanel, VarRef};

use crate::app::EditorApp;
use crate::design::{
    mono, quiet_pill, section_header, Tokens, TypeScale, SPACE_1, SPACE_2, SPACE_3,
};
use crate::panels::bench::{self, Widget};
use crate::panels::icons::{self, Icon};
use crate::queries;

/// The hint shown when the cursor is on no element.
pub const NO_SELECTION: &str = "Select an element to edit its properties";

/// A read-only property row: a dim label and a value.
fn property(ui: &mut Ui, tokens: &Tokens, label: &str, value: impl Into<String>) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(label)
                .size(TypeScale::CAPTION)
                .color(tokens.text_dim),
        );
        ui.label(
            RichText::new(value.into())
                .size(TypeScale::BODY)
                .color(tokens.text),
        );
    });
}

/// A monospaced property row, for addresses and expressions.
fn property_mono(ui: &mut Ui, tokens: &Tokens, label: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(label)
                .size(TypeScale::CAPTION)
                .color(tokens.text_dim),
        );
        ui.label(mono(value).color(tokens.text));
    });
}

/// The validation message of a field, drawn next to it rather than in a modal.
fn field_error(ui: &mut Ui, tokens: &Tokens, message: &str) {
    ui.horizontal_wrapped(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), Sense::hover());
        icons::draw(ui.painter(), rect, tokens, Icon::Error);
        ui.label(
            RichText::new(message)
                .size(TypeScale::CAPTION)
                .color(tokens.error),
        );
    });
}

/// Draws the inspector.
pub fn show(app: &mut EditorApp, ui: &mut Ui) {
    let tokens = app.tokens;
    section_header(ui, &tokens, "Inspector");
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // A sequential section has its own inspector: the selected step, the
            // selected transition or the page. It is checked first because an SFC
            // document has no rungs and no bench widget, so nothing below could
            // describe what the user clicked.
            if crate::sfc::is_sfc(app.project(), app.selected_section) {
                crate::sfc::inspector(app, ui);
                return;
            }
            if app.centre_tab == crate::app::CentreTab::Bench && bench::selected_widget().is_some()
            {
                bench_widget(app, ui);
                return;
            }
            if app.selected_element().is_some() {
                element(app, ui);
                return;
            }
            if app.selected_rung.is_some() {
                rung(app, ui);
                return;
            }
            if !app.project().sections.is_empty() {
                section(app, ui);
                return;
            }
            empty(ui, &tokens);
        });
}

/// The hint shown when there is nothing to inspect.
fn empty(ui: &mut Ui, tokens: &Tokens) {
    ui.add_space(SPACE_3);
    ui.horizontal_wrapped(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), Sense::hover());
        icons::draw(ui.painter(), rect, tokens, Icon::Pointer);
        ui.label(RichText::new(NO_SELECTION).color(tokens.text_dim));
    });
    ui.add_space(SPACE_2);
    ui.label(
        RichText::new(
            "Click a cell on the ladder, a rung in the project tree, or a widget on the bench.",
        )
        .size(TypeScale::CAPTION)
        .color(tokens.text_dim),
    );
}

/// The properties of the element under the cursor.
fn element(app: &mut EditorApp, ui: &mut Ui) {
    let Some(element) = app.selected_element() else {
        empty(ui, &app.tokens);
        return;
    };
    let tokens = app.tokens;
    let kind = element.kind;
    let rung = app.selected_rung;
    let cell = (element.col, element.row);

    ui.horizontal_wrapped(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(20.0, 16.0), Sense::hover());
        crate::symbols::element_glyph(ui.painter(), rect, &tokens, kind);
    });
    ui.add_space(SPACE_1);
    property(ui, &tokens, "Element", crate::canvas::describe(kind));
    property_mono(
        ui,
        &tokens,
        "Cell",
        &format!("col {} · row {}", cell.0, cell.1),
    );

    ui.add_space(SPACE_2);
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("Tag")
                .size(TypeScale::CAPTION)
                .color(tokens.text_dim),
        );
        let response = ui.add(
            egui::TextEdit::singleline(&mut app.var_buffer)
                .desired_width(f32::INFINITY)
                .hint_text("%I0 or a tag name")
                .font(egui::TextStyle::Monospace),
        );
        if commit(&response, ui) {
            app.apply_var();
        }
    });
    // A tag the project already names is offered as a shortcut spelling.
    let mut pick: Option<String> = None;
    ui.horizontal_wrapped(|ui| {
        ui.menu_button("Tag…", |ui| {
            let vars = queries::used_vars(app.project());
            if vars.is_empty() {
                ui.label(RichText::new("No variables in the project yet").weak());
                return;
            }
            for var in vars {
                let label = match queries::symbol_for(app.project(), &var) {
                    Some(symbol) => format!("{var}   ({})", symbol.name),
                    None => var.to_string(),
                };
                if ui.button(label).clicked() {
                    pick = Some(var.to_string());
                    ui.close_menu();
                }
            }
        });
        if let Some(var) = pick {
            app.var_buffer = var;
            app.apply_var();
        }
    });
    if let Some(error) = app.var_error.clone() {
        field_error(ui, &tokens, &error);
    }

    ui.add_space(SPACE_2);
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("Params")
                .size(TypeScale::CAPTION)
                .color(tokens.text_dim),
        );
        let response = ui.add(
            egui::TextEdit::singleline(&mut app.params_buffer)
                .desired_width(f32::INFINITY)
                .hint_text("none"),
        );
        if commit(&response, ui) {
            app.apply_params();
        }
    });
    if !element.params.is_empty() {
        ui.label(
            RichText::new("Separate parameters with a space.")
                .size(TypeScale::CAPTION)
                .color(tokens.text_dim),
        );
    }

    ui.add_space(SPACE_2);
    let linked = element.connected_with_top;
    let mut want_link = linked;
    if ui
        .checkbox(&mut want_link, "Vertical link to the row above")
        .on_hover_text("Draw the connection that feeds this cell from the row above")
        .changed()
    {
        app.toggle_vertical_link();
    }

    ui.add_space(SPACE_2);
    ui.horizontal_wrapped(|ui| {
        if ui
            .button("Add to watch table")
            .on_hover_text("Monitor this variable in the Watch & force document")
            .clicked()
        {
            add_to_watch(app, &element.var);
        }
        if ui
            .button("Delete")
            .on_hover_text("Remove the element (Ctrl+Z undoes it)")
            .clicked()
        {
            app.delete_selection();
        }
    });

    if let Some(rung) = rung {
        ui.add_space(SPACE_1);
        ui.label(
            RichText::new(format!("on rung {rung}, col {} row {}", cell.0, cell.1))
                .size(TypeScale::CAPTION)
                .color(tokens.text_dim),
        );
    }
}

/// Adds `var` to the watch table once, and opens the document so the result is
/// visible.
fn add_to_watch(app: &mut EditorApp, var: &Option<VarRef>) {
    let Some(var) = var.clone() else {
        app.note("this element has no variable to watch");
        return;
    };
    if !app.watch.iter().any(|row| row.var == var) {
        app.watch.push(crate::panels::watch::row_for(var.clone()));
    }
    app.centre_tab = crate::app::CentreTab::Watch;
    app.note(&format!("watching {var}"));
}

/// The properties of the selected rung.
fn rung(app: &mut EditorApp, ui: &mut Ui) {
    let Some(id) = app.selected_rung else {
        return;
    };
    let Some((label, comment, elements)) = app.project().rung(id).map(|rung| {
        (
            rung.label.clone(),
            rung.comment.clone(),
            rung.elements.len(),
        )
    }) else {
        empty(ui, &app.tokens);
        return;
    };
    let tokens = app.tokens;
    let position = queries::find_rung(app.project(), id).map(|found| found.position + 1);

    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), Sense::hover());
        icons::draw(ui.painter(), rect, &tokens, Icon::Rung);
        ui.label(
            RichText::new(match position {
                Some(position) => format!("Rung {position}"),
                None => format!("Rung {id}"),
            })
            .size(TypeScale::EMPHASIS)
            .color(tokens.text)
            .strong(),
        );
    });
    if label.trim().is_empty() {
        property(ui, &tokens, "Label", "—");
    } else {
        property(ui, &tokens, "Label", label.clone());
    }
    if comment.trim().is_empty() {
        property(ui, &tokens, "Comment", "—");
    } else {
        ui.label(
            RichText::new("Comment")
                .size(TypeScale::CAPTION)
                .color(tokens.text_dim),
        );
        ui.label(RichText::new(&comment).color(tokens.text));
    }
    property(ui, &tokens, "Elements", elements.to_string());

    ui.add_space(SPACE_2);
    ui.horizontal_wrapped(|ui| {
        if ui
            .button("Edit the label and the comment…")
            .on_hover_text("Open the rung editor in the project tree")
            .clicked()
        {
            app.rung_label_buffer = label;
            app.rung_comment_buffer = comment;
            app.rung_text_target = Some(id);
        }
        if ui
            .button("Show on the ladder")
            .on_hover_text("Switch the centre to the Ladder document")
            .clicked()
        {
            app.centre_tab = crate::app::CentreTab::Ladder;
        }
    });

    ui.add_space(SPACE_2);
    let problems = app.editor.problems().to_vec();
    let count = queries::rung_problem_count(app.project(), &problems, id);
    if count == 0 {
        quiet_pill(ui, tokens.run, "no problems on this rung");
    } else {
        quiet_pill(ui, tokens.error, &format!("{count} problem(s)"));
    }
}

/// The properties of the selected section.
fn section(app: &mut EditorApp, ui: &mut Ui) {
    let index = app.selected_section;
    let Some((id, name, language, rungs)) = app.project().sections.get(index).map(|section| {
        (
            section.id,
            section.name.clone(),
            section.language,
            queries::section_rungs(app.project(), index).len(),
        )
    }) else {
        empty(ui, &app.tokens);
        return;
    };
    let tokens = app.tokens;
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), Sense::hover());
        icons::draw(ui.painter(), rect, &tokens, Icon::Section);
        ui.label(
            RichText::new(&name)
                .size(TypeScale::EMPHASIS)
                .color(tokens.text)
                .strong(),
        );
    });
    property(ui, &tokens, "Id", id.to_string());
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("Language")
                .size(TypeScale::CAPTION)
                .color(tokens.text_dim),
        );
        quiet_pill(ui, tokens.accent, language_label(language));
    });
    property(ui, &tokens, "Rungs", rungs.to_string());

    ui.add_space(SPACE_2);
    ui.horizontal_wrapped(|ui| {
        if ui
            .button("Rename…")
            .on_hover_text("Rename the section in the project tree")
            .clicked()
        {
            app.section_name_buffer = name;
            app.section_name_target = Some(id);
        }
        if ui
            .button("New section")
            .on_hover_text("Append an empty ladder section")
            .clicked()
        {
            app.add_section("New section");
        }
    });
}

/// The full name of a section language.
pub fn language_label(language: SectionLanguage) -> &'static str {
    match language {
        SectionLanguage::Ladder => "LADDER",
        SectionLanguage::Sfc => "SFC",
    }
}

/// The properties of the selected bench widget.
///
/// Editing a widget goes through [`bench::update_panel`], so the bench document
/// and the inspector never disagree about the project's layout.
fn bench_widget(app: &mut EditorApp, ui: &mut Ui) {
    let Some(widget) = bench::selected_widget() else {
        return;
    };
    let tokens = app.tokens;
    let panel = app.project().simulation.clone();
    match widget {
        Widget::Switch(index) => {
            let Some(switch) = panel.switches.get(index).cloned() else {
                bench::select_widget(None);
                return;
            };
            widget_header(
                ui,
                &tokens,
                Icon::Bench,
                "Switch",
                &switch.label,
                &switch.var,
            );
            let mut momentary = switch.momentary;
            if ui
                .checkbox(&mut momentary, "Momentary push-button")
                .on_hover_text("A push-button springs back after one scan")
                .changed()
            {
                bench::update_panel(app, |panel| {
                    if let Some(target) = panel.switches.get_mut(index) {
                        target.momentary = momentary;
                        return true;
                    }
                    false
                });
            }
            let mut label = switch.label.clone();
            if ui
                .add(egui::TextEdit::singleline(&mut label).desired_width(f32::INFINITY))
                .changed()
            {
                bench::update_panel(app, |panel| {
                    if let Some(target) = panel.switches.get_mut(index) {
                        target.label = label.clone();
                        return true;
                    }
                    false
                });
            }
            property_mono(ui, &tokens, "Address", &switch.var.to_string());
        }
        Widget::Lamp(index) => {
            let Some(lamp) = panel.lamps.get(index).cloned() else {
                bench::select_widget(None);
                return;
            };
            widget_header(ui, &tokens, Icon::Bench, "Lamp", &lamp.label, &lamp.var);
            let mut label = lamp.label.clone();
            if ui
                .add(egui::TextEdit::singleline(&mut label).desired_width(f32::INFINITY))
                .changed()
            {
                bench::update_panel(app, |panel| {
                    if let Some(target) = panel.lamps.get_mut(index) {
                        target.label = label.clone();
                        return true;
                    }
                    false
                });
            }
            property_mono(ui, &tokens, "Address", &lamp.var.to_string());
            let reading = app.bench.readings().get(index).cloned();
            property(
                ui,
                &tokens,
                "Live",
                queries::value_text(reading.map(|reading| reading.value)),
            );
        }
        Widget::Analog(index) => {
            let Some(analog) = panel.analogs.get(index).cloned() else {
                bench::select_widget(None);
                return;
            };
            widget_header(
                ui,
                &tokens,
                Icon::Bench,
                "Slider",
                &analog.label,
                &analog.var,
            );
            property_mono(ui, &tokens, "Address", &analog.var.to_string());
            range_editor(app, ui, index, analog.min, analog.max);
        }
        Widget::Gauge(index) => {
            let Some(gauge) = panel.gauges.get(index).cloned() else {
                bench::select_widget(None);
                return;
            };
            widget_header(ui, &tokens, Icon::Bench, "Gauge", &gauge.label, &gauge.var);
            property_mono(ui, &tokens, "Address", &gauge.var.to_string());
            range_editor(app, ui, index, gauge.min, gauge.max);
        }
    }

    ui.add_space(SPACE_2);
    if ui
        .button("Remove this widget")
        .on_hover_text("Take it off the bench; the program is untouched")
        .clicked()
    {
        bench::update_panel(app, |panel| remove_widget(panel, widget));
        bench::select_widget(None);
    }
}

/// Draws a bench widget's header: its icon, kind and tag.
fn widget_header(ui: &mut Ui, tokens: &Tokens, icon: Icon, kind: &str, label: &str, var: &VarRef) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), Sense::hover());
        icons::draw(ui.painter(), rect, tokens, icon);
        ui.label(
            RichText::new(kind)
                .size(TypeScale::EMPHASIS)
                .color(tokens.text)
                .strong(),
        );
    });
    property(ui, tokens, "Label", label);
    property_mono(ui, tokens, "Variable", &var.to_string());
}

/// The min/max editor shared by the sliders and the gauges.
fn range_editor(app: &mut EditorApp, ui: &mut Ui, index: usize, min: i32, max: i32) {
    let tokens = app.tokens;
    let mut low = min;
    let mut high = max;
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("Range")
                .size(TypeScale::CAPTION)
                .color(tokens.text_dim),
        );
        changed |= ui.add(egui::DragValue::new(&mut low)).changed();
        changed |= ui.add(egui::DragValue::new(&mut high)).changed();
    });
    if changed && low <= high {
        bench::update_panel(app, |panel| {
            if let Some(analog) = panel.analogs.get_mut(index) {
                analog.min = low;
                analog.max = high;
                return true;
            }
            if let Some(gauge) = panel.gauges.get_mut(index) {
                gauge.min = low;
                gauge.max = high;
                return true;
            }
            false
        });
    }
}

/// Removes a widget from the panel; `true` when it was there.
fn remove_widget(panel: &mut SimulationPanel, widget: Widget) -> bool {
    let (list, index) = match widget {
        Widget::Switch(index) => (0, index),
        Widget::Lamp(index) => (1, index),
        Widget::Analog(index) => (2, index),
        Widget::Gauge(index) => (3, index),
    };
    let len = match list {
        0 => panel.switches.len(),
        1 => panel.lamps.len(),
        2 => panel.analogs.len(),
        _ => panel.gauges.len(),
    };
    if index >= len {
        return false;
    }
    match list {
        0 => {
            panel.switches.remove(index);
        }
        1 => {
            panel.lamps.remove(index);
        }
        2 => {
            panel.analogs.remove(index);
        }
        _ => {
            panel.gauges.remove(index);
        }
    }
    true
}

/// Whether a text field asked to commit: focus left it, or `Enter` was pressed.
fn commit(response: &egui::Response, ui: &Ui) -> bool {
    response.lost_focus()
        || (response.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)))
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
    use softladder_core::{ElementKind, Project, Rung, Section};

    fn project() -> Project {
        let mut project = Project::new("inspector");
        let mut main = Section::new(1, "Main");
        main.rungs.push(1);
        project.sections.push(main);
        let mut rung = Rung::new(1);
        rung.label = "start_stop".to_owned();
        rung.elements.push(softladder_core::PlacedElement::with_var(
            ElementKind::ContactNo,
            "%I0".parse().expect("variable parses"),
            0,
            0,
        ));
        project.rungs.push(rung);
        project
    }

    #[test]
    fn the_inspector_is_context_sensitive_and_draws_every_state() {
        let mut app = EditorApp::new(project());
        // An element under the cursor.
        app.select(1, Some((0, 0)));
        crate::panels::test_frame(&ctx(), &mut app, TEST_SIZE);
        // A rung with no element on the cursor.
        app.selection = Some((9, 9));
        app.load_properties();
        crate::panels::test_frame(&ctx(), &mut app, TEST_SIZE);
        // The section.
        app.selected_rung = None;
        app.selection = None;
        crate::panels::test_frame(&ctx(), &mut app, TEST_SIZE);
        // Nothing at all: an empty project, and the bench document open with a
        // widget selected that the project does not have.
        let mut empty = EditorApp::new(Project::new("empty"));
        empty.show_tab(crate::app::RightTab::Bench);
        crate::panels::test_frame(&ctx(), &mut empty, TEST_SIZE);
        bench::select_widget(Some(Widget::Switch(4)));
        crate::panels::test_frame(&ctx(), &mut empty, TEST_SIZE);
        bench::select_widget(Some(Widget::Analog(2)));
        crate::panels::test_frame(&ctx(), &mut empty, TEST_SIZE);
        bench::select_widget(None);
    }

    #[test]
    fn every_section_language_has_a_readable_name() {
        assert_eq!(language_label(SectionLanguage::Ladder), "LADDER");
        assert_eq!(language_label(SectionLanguage::Sfc), "SFC");
    }

    #[test]
    fn removing_a_widget_takes_it_off_the_panel_only() {
        let mut panel = SimulationPanel::default();
        panel.switches.push(softladder_core::SimSwitch {
            var: "%I0".parse().expect("variable parses"),
            label: "start".to_owned(),
            momentary: false,
        });
        assert!(remove_widget(&mut panel, Widget::Switch(0)));
        assert!(panel.switches.is_empty());
        // A stale index is refused instead of panicking.
        assert!(!remove_widget(&mut panel, Widget::Switch(3)));
        assert!(!remove_widget(&mut panel, Widget::Lamp(0)));
        assert!(!remove_widget(&mut panel, Widget::Analog(7)));
        assert!(!remove_widget(&mut panel, Widget::Gauge(4)));
    }

    #[test]
    fn adding_a_variable_to_the_watch_table_happens_once() {
        let mut app = EditorApp::new(project());
        app.select(1, Some((0, 0)));
        let var = app.selected_element().and_then(|element| element.var);
        add_to_watch(&mut app, &var);
        add_to_watch(&mut app, &var);
        assert_eq!(app.watch.len(), 1, "the same variable is not watched twice");
        assert_eq!(app.centre_tab, crate::app::CentreTab::Watch);
        // An element with no variable says so instead of watching nothing.
        add_to_watch(&mut app, &None);
        assert_eq!(app.watch.len(), 1);
        assert!(app.status().contains("no variable"));
    }
}
