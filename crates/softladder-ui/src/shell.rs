//! The application shell: the ribbon, the document tabs and the dispatch to the
//! document in the middle.
//!
//! See `docs/UX.md` §3. The panels on the sides belong to `panels::*`; this file
//! only decides what the window looks like around them.

use egui::{Align, Layout, RichText, Ui};

use crate::app::{CentreTab, EditorApp};
use crate::design::{quiet_pill, rule, TypeScale, SPACE_1, SPACE_2, SPACE_3};
use crate::shortcuts::Action;
use crate::{canvas, panels};

/// The menu bar and the command groups under it.
pub fn ribbon(app: &mut EditorApp, ui: &mut Ui) {
    let tokens = app.tokens;
    menu_bar(app, ui);
    rule(ui, &tokens);
    ui.add_space(SPACE_1);
    ui.horizontal(|ui| {
        ui.add_space(SPACE_2);

        group(ui, "History", |ui| {
            command(app, ui, "Undo", Action::Undo, "Ctrl+Z");
            command(app, ui, "Redo", Action::Redo, "Ctrl+Shift+Z");
        });
        separator(ui, &tokens);
        group(ui, "File", |ui| {
            command(app, ui, "New", Action::New, "Ctrl+N");
            command(app, ui, "Open", Action::Open, "Ctrl+O");
            command(app, ui, "Save", Action::Save, "Ctrl+S");
            command(app, ui, "Save as", Action::SaveAs, "Ctrl+Shift+S");
        });
        separator(ui, &tokens);
        group(ui, "Online", |ui| {
            let running = app.is_running();
            if ui
                .add(
                    egui::Button::new(if running { "Stop" } else { "Run" })
                        .min_size(egui::vec2(52.0, 20.0)),
                )
                .on_hover_text(if running {
                    "Stop the bench (Ctrl+R)"
                } else {
                    "Run the bench (Ctrl+R)"
                })
                .clicked()
            {
                app.handle(Action::RunStop);
            }
            command(app, ui, "Step", Action::SingleScan, "Ctrl+T");
            command(app, ui, "Fill bench", Action::AutoFillBench, "Ctrl+Shift+A");
            if running {
                let cycles = app.bench().cycles();
                quiet_pill(ui, tokens.run, &format!("RUN · {cycles}"));
            } else {
                quiet_pill(ui, tokens.text_dim, "STOP");
            }
            if !app.forces.is_empty() {
                quiet_pill(ui, tokens.warning, &format!("{} forced", app.forces.len()));
            }
        });
        separator(ui, &tokens);
        group(ui, "View", |ui| {
            command(app, ui, "−", Action::ZoomOut, "Ctrl+-");
            command(app, ui, "+", Action::ZoomIn, "Ctrl++");
            command(app, ui, "1:1", Action::ZoomReset, "Ctrl+0");
            if ui
                .selectable_label(app.show_addresses, "Addresses")
                .on_hover_text("Show the address under every tag name")
                .clicked()
            {
                app.show_addresses = !app.show_addresses;
            }
            if ui
                .selectable_label(app.theme == crate::design::Theme::Dark, "Dark")
                .on_hover_text("Switch the palette")
                .clicked()
            {
                app.theme = app.tokens.toggled().theme;
            }
        });
        separator(ui, &tokens);

        // The element palette lives in the ribbon, the way every vendor tool
        // puts its instruction set next to the commands.
        ui.vertical(|ui| {
            ui.label(
                RichText::new("Insert")
                    .size(TypeScale::CAPTION)
                    .color(tokens.text_dim),
            );
            panels::palette::show(app, ui);
        });
    });
    ui.add_space(SPACE_1);
    rule(ui, &tokens);
}

/// One labelled command group, with the label under the buttons' row.
fn group(ui: &mut Ui, title: &str, contents: impl FnOnce(&mut Ui)) {
    ui.vertical(|ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = SPACE_1;
            contents(ui);
        });
        ui.label(
            RichText::new(title)
                .size(TypeScale::CAPTION)
                .color(ui.visuals().weak_text_color()),
        );
    });
}

/// A vertical separator between groups.
fn separator(ui: &mut Ui, tokens: &crate::design::Tokens) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(SPACE_2, 34.0), egui::Sense::hover());
    ui.painter()
        .vline(rect.center().x, rect.y_range(), tokens.hairline());
}

/// One command button with its shortcut in the tooltip.
fn command(app: &mut EditorApp, ui: &mut Ui, label: &str, action: Action, shortcut: &str) {
    if ui
        .button(label)
        .on_hover_text(format!("{label}  ({shortcut})\n{}", describe(&action)))
        .clicked()
    {
        app.handle(action);
    }
}

/// A one-line explanation of a command, for the tooltip.
fn describe(action: &Action) -> &'static str {
    match action {
        Action::Undo => "Undo the last edit",
        Action::Redo => "Redo the last undone edit",
        Action::New => "Start an empty project",
        Action::Open => "Open a project or a ClassicLadder file",
        Action::Save => "Save the project",
        Action::SaveAs => "Save the project under a new name",
        Action::RunStop => "Run or stop the simulation bench",
        Action::SingleScan => "Advance the bench by one scan",
        Action::AutoFillBench => "Build the bench from the program's physical variables",
        Action::ZoomIn => "Zoom the canvas in",
        Action::ZoomOut => "Zoom the canvas out",
        Action::ZoomReset => "Reset the zoom and the pan",
        _ => "Run this command",
    }
}

/// The menu bar: the same commands, with the names the vendors use.
fn menu_bar(app: &mut EditorApp, ui: &mut Ui) {
    egui::menu::bar(ui, |ui| {
        ui.menu_button("File", |ui| {
            menu_command(app, ui, "New", Action::New);
            menu_command(app, ui, "Open…", Action::Open);
            ui.separator();
            menu_command(app, ui, "Save", Action::Save);
            menu_command(app, ui, "Save as…", Action::SaveAs);
            ui.separator();
            if ui.button("Quit").clicked() {
                ui.close_menu();
                app.request_quit();
            }
        });
        ui.menu_button("Edit", |ui| {
            let undo = app.editor().undo_label().map(str::to_owned);
            let redo = app.editor().redo_label().map(str::to_owned);
            let undo_label = undo.map_or_else(|| "Undo".to_owned(), |l| format!("Undo {l}"));
            let redo_label = redo.map_or_else(|| "Redo".to_owned(), |l| format!("Redo {l}"));
            if ui
                .add_enabled(app.editor().can_undo(), egui::Button::new(undo_label))
                .clicked()
            {
                ui.close_menu();
                app.handle(Action::Undo);
            }
            if ui
                .add_enabled(app.editor().can_redo(), egui::Button::new(redo_label))
                .clicked()
            {
                ui.close_menu();
                app.handle(Action::Redo);
            }
            ui.separator();
            menu_command(app, ui, "Delete element", Action::Delete);
        });
        ui.menu_button("Insert", |ui| {
            for kind in crate::palette::all_kinds() {
                let Some(entry) = crate::palette::entry_for_kind(kind) else {
                    continue;
                };
                let (label, tooltip, letter) = (entry.label, entry.tooltip, entry.letter);
                if ui
                    .button(label)
                    .on_hover_text(format!("{tooltip}  ({letter})"))
                    .clicked()
                {
                    ui.close_menu();
                    app.handle(Action::Pick(kind));
                }
            }
        });
        ui.menu_button("Online", |ui| {
            menu_command(app, ui, "Run / Stop", Action::RunStop);
            menu_command(app, ui, "Single scan", Action::SingleScan);
            menu_command(
                app,
                ui,
                "Fill the bench from the program",
                Action::AutoFillBench,
            );
        });
        ui.menu_button("View", |ui| {
            for tab in CentreTab::ALL {
                let selected = app.centre_tab == tab;
                if ui.selectable_label(selected, tab.label()).clicked() {
                    ui.close_menu();
                    app.centre_tab = tab;
                }
            }
            ui.separator();
            if ui
                .selectable_label(app.show_addresses, "Show addresses")
                .clicked()
            {
                app.show_addresses = !app.show_addresses;
            }
            if ui
                .selectable_label(app.show_inspector, "Inspector")
                .clicked()
            {
                app.show_inspector = !app.show_inspector;
            }
            if ui
                .selectable_label(app.theme == crate::design::Theme::Dark, "Dark theme")
                .clicked()
            {
                app.theme = app.tokens.toggled().theme;
            }
            ui.separator();
            menu_command(app, ui, "Zoom in", Action::ZoomIn);
            menu_command(app, ui, "Zoom out", Action::ZoomOut);
            menu_command(app, ui, "Reset the view", Action::ZoomReset);
        });
        ui.menu_button("Tools", |ui| {
            if ui.button("Simulation bench").clicked() {
                ui.close_menu();
                app.centre_tab = CentreTab::Bench;
            }
            if ui.button("Check the program").clicked() {
                ui.close_menu();
                app.refresh_diagnostics();
            }
            if ui.button("Build the bench from the program").clicked() {
                ui.close_menu();
                app.handle(Action::AutoFillBench);
            }
        });
        ui.menu_button("Help", |ui| {
            menu_command(app, ui, "Keyboard shortcuts", Action::ShortcutHelp);
            if ui.button("About SoftLadder").clicked() {
                ui.close_menu();
                app.show_about = true;
            }
        });
        let project = app
            .editor()
            .path()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| "untitled".to_owned());
        let dirty = if app.editor().is_dirty() { " *" } else { "" };
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(
                RichText::new(format!("{project}{dirty}"))
                    .size(TypeScale::CAPTION)
                    .color(app.tokens.text_dim),
            );
        });
    });
}

/// A menu entry that runs a command.
fn menu_command(app: &mut EditorApp, ui: &mut Ui, label: &str, action: Action) {
    if ui.button(label).clicked() {
        ui.close_menu();
        app.handle(action);
    }
}

/// The centre of the window: the document tabs and the open document.
pub fn centre(app: &mut EditorApp, ui: &mut Ui) {
    tab_bar(app, ui);
    let tokens = app.tokens;
    rule(ui, &tokens);
    match app.centre_tab {
        CentreTab::Ladder => {
            panels::properties::show(app, ui);
            rule(ui, &tokens);
            canvas::show(app, ui);
        }
        CentreTab::Tags => panels::tags::show(app, ui),
        CentreTab::Bench => panels::bench::show(app, ui),
        CentreTab::Watch => panels::watch::show(app, ui),
        CentreTab::Problems => panels::problems::show(app, ui),
    }
}

/// The document tab strip.
fn tab_bar(app: &mut EditorApp, ui: &mut Ui) {
    let tokens = app.tokens;
    let problems = app.editor().problems().len();
    ui.horizontal(|ui| {
        ui.add_space(SPACE_2);
        for tab in CentreTab::ALL {
            let label = match tab {
                CentreTab::Problems if problems > 0 => format!("{} ({problems})", tab.label()),
                CentreTab::Watch if !app.watch.is_empty() => {
                    format!("{} ({})", tab.label(), app.watch.len())
                }
                _ => tab.label().to_owned(),
            };
            let active = app.centre_tab == tab;
            let text = if active {
                RichText::new(label).color(tokens.accent).strong()
            } else {
                RichText::new(label).color(tokens.text)
            };
            let response = ui.selectable_label(active, text);
            if response.clicked() {
                app.centre_tab = tab;
            }
            if active {
                // A 2 px accent underline, as TIA and Studio 5000 draw the open tab.
                let rect = response.rect;
                ui.painter().hline(
                    rect.x_range(),
                    rect.bottom() + 1.0,
                    egui::Stroke::new(2.0_f32, tokens.accent),
                );
            }
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.add_space(SPACE_3);
            let problems = app.editor().problems();
            let errors = problems
                .iter()
                .filter(|diagnostic| diagnostic.severity == softladder_core::Severity::Error)
                .count();
            let warnings = problems.len() - errors;
            if warnings > 0 {
                quiet_pill(ui, tokens.warning, &format!("{warnings} warning(s)"));
            }
            if errors > 0 {
                quiet_pill(ui, tokens.error, &format!("{errors} error(s)"));
            }
            if problems.is_empty() {
                quiet_pill(ui, tokens.run, "no problems");
            }
        });
    });
}
