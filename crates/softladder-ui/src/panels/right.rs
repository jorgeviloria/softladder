//! The right-hand tabbed panel: Bench, Watch, Problems and Symbols.

use crate::app::{EditorApp, RightTab};
use crate::panels;
use crate::queries;

/// Draws the tab bar and the selected tab.
pub fn show(app: &mut EditorApp, ui: &mut egui::Ui) {
    let (errors, warnings) = queries::problem_counts(app.editor.problems());
    ui.horizontal_wrapped(|ui| {
        for tab in [
            RightTab::Bench,
            RightTab::Watch,
            RightTab::Problems,
            RightTab::Symbols,
        ] {
            let label = match tab {
                RightTab::Problems => format!("{} ({})", tab.title(), errors + warnings),
                _ => tab.title().to_owned(),
            };
            if ui.selectable_label(app.right_tab == tab, label).clicked() {
                app.right_tab = tab;
            }
        }
    });
    ui.separator();
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| match app.right_tab {
            RightTab::Bench => panels::bench::show(app, ui),
            RightTab::Watch => panels::watch::show(app, ui),
            RightTab::Problems => panels::problems::show(app, ui),
            RightTab::Symbols => panels::symbols::show(app, ui),
        });
}
