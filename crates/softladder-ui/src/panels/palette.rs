//! The element palette, drawn above the canvas.

use crate::app::{EditorApp, Tool};
use crate::palette;
use crate::shortcuts::Action;

/// Draws the pointer tool and one button per palette entry.
pub fn show(app: &mut EditorApp, ui: &mut egui::Ui) {
    ui.horizontal_wrapped(|ui| {
        let pointer = app.tool == Tool::Select;
        if ui
            .selectable_label(pointer, "select")
            .on_hover_text("Pointer tool: click to select, drag to move")
            .clicked()
        {
            app.handle(Action::Cancel);
        }
        ui.separator();
        for entry in palette::entries() {
            let armed = app.tool == Tool::Place(entry.kind);
            let text = format!("{} {}", entry.label, entry.letter);
            if ui
                .selectable_label(armed, text)
                .on_hover_text(entry.tooltip)
                .clicked()
            {
                app.handle(Action::Pick(entry.kind));
            }
        }
    });
}
