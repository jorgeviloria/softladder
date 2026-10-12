//! The application shell: the ribbon, the document tabs and the dispatch to the
//! document in the middle.
//!
//! See `docs/UX.md` §3. The panels on the sides belong to `panels::*`; this file
//! only decides what the window looks like around them.

use egui::{Align, Layout, RichText, Ui};

use crate::app::{CentreTab, EditorApp};
use crate::design::{quiet_pill, rule, TypeScale, SPACE_1, SPACE_2, SPACE_3};
use crate::panels::icons::{self, Icon};
use crate::shortcuts::Action;

use crate::{canvas, panels};

/// The menu bar and the command bar under it.
///
/// One row of icon buttons, grouped by hairlines: history, file, run, view.
/// Every button carries its name, its shortcut and a one-line description in
/// the tooltip. The instruction palette is not here: it sits on top of the
/// document it inserts into (see [`centre`]), so the ribbon never clips it.
pub fn ribbon(app: &mut EditorApp, ui: &mut Ui) {
    let tokens = app.tokens;
    menu_bar(app, ui);
    rule(ui, &tokens);
    ui.add_space(SPACE_1);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = SPACE_1;
        ui.add_space(SPACE_2);

        let can_undo = app.editor().can_undo();
        let can_redo = app.editor().can_redo();
        command(
            app,
            ui,
            Icon::Undo,
            "Undo",
            Action::Undo,
            "Ctrl+Z",
            can_undo,
        );
        command(
            app,
            ui,
            Icon::Redo,
            "Redo",
            Action::Redo,
            "Ctrl+Shift+Z",
            can_redo,
        );
        separator(ui, &tokens);
        command(app, ui, Icon::New, "New", Action::New, "Ctrl+N", true);
        command(app, ui, Icon::Open, "Open", Action::Open, "Ctrl+O", true);
        command(app, ui, Icon::Save, "Save", Action::Save, "Ctrl+S", true);
        command(
            app,
            ui,
            Icon::SaveAs,
            "Save as",
            Action::SaveAs,
            "Ctrl+Shift+S",
            true,
        );
        separator(ui, &tokens);

        run_button(app, ui);
        command(
            app,
            ui,
            Icon::Step,
            "Single scan",
            Action::SingleScan,
            "Ctrl+T",
            true,
        );
        command(
            app,
            ui,
            Icon::Bench,
            "Fill the bench",
            Action::AutoFillBench,
            "Ctrl+Shift+A",
            true,
        );
        separator(ui, &tokens);

        command(
            app,
            ui,
            Icon::ZoomOut,
            "Zoom out",
            Action::ZoomOut,
            "Ctrl+-",
            true,
        );
        command(
            app,
            ui,
            Icon::ZoomIn,
            "Zoom in",
            Action::ZoomIn,
            "Ctrl++",
            true,
        );
        command(
            app,
            ui,
            Icon::ZoomFit,
            "Reset the view",
            Action::ZoomReset,
            "Ctrl+0",
            true,
        );
        separator(ui, &tokens);

        if icon_button(ui, &tokens, Icon::Address, app.show_addresses, true)
            .on_hover_text("Show addresses\nShow the address under every tag name")
            .clicked()
        {
            app.show_addresses = !app.show_addresses;
        }
        let dark = app.theme == crate::design::Theme::Dark;
        let (icon, tip) = if dark {
            (Icon::Sun, "Light theme\nSwitch to the light palette")
        } else {
            (Icon::Moon, "Dark theme\nSwitch to the dark palette")
        };
        if icon_button(ui, &tokens, icon, false, true)
            .on_hover_text(tip)
            .clicked()
        {
            app.theme = app.tokens.toggled().theme;
        }

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.add_space(SPACE_3);
            if !app.forces.is_empty() {
                quiet_pill(ui, tokens.warning, &format!("{} forced", app.forces.len()));
            }
        });
    });
    ui.add_space(SPACE_1);
}

/// Side of a command button.
const BUTTON: f32 = 28.0;
/// Side of the icon inside a command button.
const BUTTON_ICON: f32 = 16.0;

/// A square icon button with a visible state: `selected` draws it pressed in,
/// `enabled == false` greys it out and makes it inert.
pub fn icon_button(
    ui: &mut Ui,
    tokens: &crate::design::Tokens,
    icon: Icon,
    selected: bool,
    enabled: bool,
) -> egui::Response {
    let sense = if enabled {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(egui::vec2(BUTTON, BUTTON), sense);
    let hovered = enabled && response.hovered();
    let (fill, stroke) = if selected {
        (
            tokens.accent_soft,
            egui::Stroke::new(1.0_f32, tokens.accent),
        )
    } else if hovered {
        (tokens.surface, tokens.hairline())
    } else {
        (egui::Color32::TRANSPARENT, egui::Stroke::NONE)
    };
    ui.painter().rect(
        rect,
        egui::CornerRadius::same(crate::design::RADIUS_CONTROL),
        fill,
        stroke,
        egui::StrokeKind::Inside,
    );
    let ink = if !enabled {
        tokens.text_dim.gamma_multiply(0.5)
    } else if selected {
        tokens.accent
    } else {
        tokens.text
    };
    let glyph = egui::Rect::from_center_size(rect.center(), egui::vec2(BUTTON_ICON, BUTTON_ICON));
    icons::draw_ink(ui.painter(), glyph, ink, icon);
    response
}

/// The Run / Stop button: the one command whose state must be unmistakable, so
/// it is filled in the colour of what it will do and labelled in words.
fn run_button(app: &mut EditorApp, ui: &mut Ui) {
    let tokens = app.tokens;
    let running = app.is_running();
    let (icon, label, fill, tip) = if running {
        (Icon::Stop, "Stop", tokens.stop, "Stop the bench (Ctrl+R)")
    } else {
        (Icon::Play, "Run", tokens.run, "Run the bench (Ctrl+R)")
    };
    let font = egui::FontId::new(TypeScale::BODY, crate::design::semibold_family(ui.ctx()));
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font, egui::Color32::WHITE);
    let width = SPACE_2 + BUTTON_ICON - 4.0 + SPACE_1 + galley.size().x + SPACE_3;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, BUTTON), egui::Sense::click());
    let fill = if response.hovered() {
        fill.gamma_multiply(0.85)
    } else {
        fill
    };
    ui.painter().rect_filled(
        rect,
        egui::CornerRadius::same(crate::design::RADIUS_CONTROL),
        fill,
    );
    let glyph = egui::Rect::from_min_size(
        egui::pos2(rect.left() + SPACE_2, rect.center().y - 6.0),
        egui::vec2(12.0, 12.0),
    );
    icons::draw_ink(ui.painter(), glyph, egui::Color32::WHITE, icon);
    ui.painter().galley(
        egui::pos2(
            glyph.right() + SPACE_1 + 2.0,
            rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        egui::Color32::WHITE,
    );
    if response.on_hover_text(tip).clicked() {
        app.handle(Action::RunStop);
    }
}

/// A vertical hairline between command groups.
fn separator(ui: &mut Ui, tokens: &crate::design::Tokens) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(SPACE_3, 20.0), egui::Sense::hover());
    ui.painter()
        .vline(rect.center().x, rect.y_range(), tokens.hairline());
}

/// One command button with its name and shortcut in the tooltip.
fn command(
    app: &mut EditorApp,
    ui: &mut Ui,
    icon: Icon,
    label: &str,
    action: Action,
    shortcut: &str,
    enabled: bool,
) {
    let tokens = app.tokens;
    if icon_button(ui, &tokens, icon, false, enabled)
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
                if ui.selectable_label(selected, tab_label(app, tab)).clicked() {
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
                .selectable_label(app.show_bench, "Bench window")
                .clicked()
            {
                app.show_bench = !app.show_bench;
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
                app.show_bench = true;
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
        let project =
            crate::queries::project_label(&app.editor().project().name, app.editor().path());
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
            // The instruction set sits on top of the document it inserts into
            // and follows its language: the ladder palette for a ladder
            // section, the chart's tools for an SFC one. It wraps instead of
            // clipping, so every family stays reachable on a narrow window.
            let sfc = crate::sfc::is_sfc(app.project(), app.selected_section);
            ui.add_space(SPACE_1);
            if sfc {
                crate::sfc::palette(app, ui);
            } else {
                panels::palette::show(app, ui);
            }
            rule(ui, &tokens);
            // The property strip edits the selected ladder element only while
            // the inspector, which does the same job, is closed.
            if !sfc && !app.show_inspector {
                panels::properties::show(app, ui);
                rule(ui, &tokens);
            }
            canvas::show(app, ui);
        }
        CentreTab::Tags => panels::tags::show(app, ui),
        CentreTab::Watch => panels::watch::show(app, ui),
        CentreTab::Problems => panels::problems::show(app, ui),
    }
}

/// The label of a document tab.
///
/// An SFC section is its own document, so the program tab reads `SFC` while
/// one is open rather than keeping the ladder's name on a chart.
fn tab_label(app: &EditorApp, tab: CentreTab) -> String {
    let problems = app.editor().problems().len();
    match tab {
        CentreTab::Ladder if crate::sfc::is_sfc(app.project(), app.selected_section) => {
            "SFC".to_owned()
        }
        CentreTab::Problems if problems > 0 => format!("Problems ({problems})"),
        CentreTab::Watch if !app.watch.is_empty() => {
            format!("Watch & force ({})", app.watch.len())
        }
        _ => tab.label().to_owned(),
    }
}

/// The document tab strip.
fn tab_bar(app: &mut EditorApp, ui: &mut Ui) {
    let tokens = app.tokens;
    ui.horizontal(|ui| {
        ui.add_space(SPACE_2);
        for tab in CentreTab::ALL {
            let label = tab_label(app, tab);
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
            if tab == CentreTab::Watch {
                if let Some(drag) =
                    response.dnd_release_payload::<crate::panels::watch::WatchDrag>()
                {
                    let var = drag.var.clone();
                    if crate::panels::watch::push_row(&mut app.watch, var.clone()) {
                        app.note(&format!("watching {var}"));
                    } else {
                        app.note(&format!("{var} is already in the watch table"));
                    }
                    app.centre_tab = CentreTab::Watch;
                }
                if response
                    .dnd_hover_payload::<crate::panels::watch::WatchDrag>()
                    .is_some()
                {
                    ui.painter().rect_stroke(
                        response.rect.expand(2.0),
                        egui::CornerRadius::same(2),
                        egui::Stroke::new(1.0_f32, tokens.accent),
                        egui::StrokeKind::Inside,
                    );
                }
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
