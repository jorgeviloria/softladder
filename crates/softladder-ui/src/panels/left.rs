//! The left panel: sections and the rungs of the selected section.

use egui::RichText;

use crate::app::EditorApp;
use crate::queries;

/// Draws the section list, the rung list and their editors.
pub fn show(app: &mut EditorApp, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.heading("Sections");
        if ui
            .small_button("＋")
            .on_hover_text("Add a section")
            .clicked()
        {
            app.add_section("New section");
        }
    });
    ui.separator();
    sections(app, ui);
    ui.add_space(6.0);
    ui.separator();
    rungs(app, ui);
}

/// The section list plus the rename and remove controls.
fn sections(app: &mut EditorApp, ui: &mut egui::Ui) {
    let sections: Vec<(usize, String)> = app
        .project()
        .sections
        .iter()
        .enumerate()
        .map(|(index, section)| (index, format!("{} (#{})", section.name, section.id)))
        .collect();
    for (index, label) in &sections {
        if ui
            .selectable_label(app.selected_section == *index, label)
            .clicked()
        {
            app.select_section(*index);
        }
    }
    if sections.is_empty() {
        ui.label(RichText::new("No sections yet").weak());
        return;
    }
    let Some(section) = app.project().sections.get(app.selected_section) else {
        return;
    };
    let id = section.id;
    let name = section.name.clone();
    if app.section_name_target != Some(id) {
        app.section_name_target = Some(id);
        app.section_name_buffer = name;
    }
    let mut remove = false;
    ui.horizontal(|ui| {
        ui.label("Name");
        let response =
            ui.add(egui::TextEdit::singleline(&mut app.section_name_buffer).desired_width(110.0));
        if response.lost_focus() {
            let buffer = app.section_name_buffer.clone();
            app.rename_section(app.selected_section, &buffer);
        }
        remove = ui
            .small_button("✖")
            .on_hover_text("Remove section")
            .clicked();
    });
    if remove {
        app.remove_section(app.selected_section);
    }
}

/// The rung list of the selected section plus insert/delete and text editing.
fn rungs(app: &mut EditorApp, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.heading("Rungs");
        let section = app.selected_section;
        if ui
            .small_button("＋")
            .on_hover_text("Insert a rung below the selection")
            .clicked()
        {
            let position = selected_position(app).map_or(0, |found| found.position + 1);
            app.insert_rung(section, position);
        }
        if ui
            .small_button("−")
            .on_hover_text("Delete the selected rung")
            .clicked()
        {
            if let Some(rung) = app.selected_rung {
                let section = app.selected_section;
                app.delete_rung(section, rung);
            }
        }
    });
    let ids = queries::section_rungs(app.project(), app.selected_section);
    if ids.is_empty() {
        ui.label(RichText::new("No rungs in this section").weak());
        return;
    }
    let problems = app.editor.problems().to_vec();
    for id in &ids {
        let (label, comment, broken) = match app.project().rung(*id) {
            Some(rung) => (
                rung.label.clone(),
                rung.comment.clone(),
                queries::rung_has_problem(app.project(), &problems, *id),
            ),
            None => continue,
        };
        let marker = if broken { "!" } else { " " };
        let text = if comment.is_empty() {
            format!("{marker} #{id} {label}")
        } else {
            format!("{marker} #{id} {label} — {comment}")
        };
        let response = ui.selectable_label(app.selected_rung == Some(*id), text);
        if response.clicked() {
            app.select_rung(*id, None);
        }
    }
    let Some(rung) = app.selected_rung else {
        return;
    };
    let (label, comment) = match app.project().rung(rung) {
        Some(rung) => (rung.label.clone(), rung.comment.clone()),
        None => return,
    };
    if app.rung_text_target != Some(rung) {
        app.rung_text_target = Some(rung);
        app.rung_label_buffer = label;
        app.rung_comment_buffer = comment;
    }
    ui.horizontal(|ui| {
        ui.label("Label");
        let response =
            ui.add(egui::TextEdit::singleline(&mut app.rung_label_buffer).desired_width(90.0));
        if response.lost_focus() {
            let label = app.rung_label_buffer.clone();
            let comment = app.rung_comment_buffer.clone();
            app.set_rung_text(rung, &label, &comment);
        }
    });
    ui.horizontal(|ui| {
        ui.label("Note");
        let response =
            ui.add(egui::TextEdit::singleline(&mut app.rung_comment_buffer).desired_width(130.0));
        if response.lost_focus() {
            let label = app.rung_label_buffer.clone();
            let comment = app.rung_comment_buffer.clone();
            app.set_rung_text(rung, &label, &comment);
        }
    });
}

/// Where the selected rung sits inside the selected section.
fn selected_position(app: &EditorApp) -> Option<queries::RungRef> {
    app.selected_rung
        .and_then(|rung| queries::find_rung(app.project(), rung))
}
