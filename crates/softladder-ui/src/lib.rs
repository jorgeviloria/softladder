//! `eframe`/`egui` desktop editor skeleton: a menu bar (File / Run / Help), a
//! section list, a pannable and zoomable rung canvas, a watch table and a
//! status bar. Editing and file dialogs land in M1/M2; the start-up project is
//! `examples/traffic_light.slprj`, loaded relative to the working directory
//! with a silent fallback to an empty project. Layout-heavy methods carry
//! `#[rustfmt::skip]` so the four required panels fit the UI line budget.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

use std::path::Path;

use eframe::egui;
use softladder_core::{PlacedElement, Project, Rung, ScanEngine, Value, VarRef, VarStore};
use softladder_project::native;

/// Project opened at start-up, relative to the current working directory.
pub const STARTUP_PROJECT: &str = "examples/traffic_light.slprj";

/// Variables shown in the watch table.
const WATCH_VARS: [&str; 6] = ["%I0", "%I1", "%Q0", "%Q1", "%M0", "%MW0"];

/// Loads the start-up project, falling back to an empty project.
pub fn load_startup_project() -> Project {
    native::load(Path::new(STARTUP_PROJECT)).unwrap_or_default()
}

/// The SoftLadder editor application.
pub struct SoftLadderApp {
    engine: ScanEngine,
    selected_section: usize,
    watch: Vec<VarRef>,
    running: bool,
    cycles: u64,
    zoom: f32,
    pan: egui::Vec2,
    status: String,
    show_about: bool,
}

impl SoftLadderApp {
    /// Creates the editor and loads [`STARTUP_PROJECT`] when it exists.
    pub fn new(_context: &eframe::CreationContext<'_>) -> Self {
        Self::with_project(load_startup_project())
    }

    /// Creates the editor around an explicit project.
    #[rustfmt::skip]
    pub fn with_project(project: Project) -> Self {
        let status = format!("loaded `{STARTUP_PROJECT}`: {} section(s)", project.sections.len());
        Self {
            engine: ScanEngine::new(project),
            selected_section: 0,
            watch: WATCH_VARS.iter().filter_map(|text| text.parse().ok()).collect(),
            running: false,
            cycles: 0,
            zoom: 1.0,
            pan: egui::Vec2::ZERO,
            status,
            show_about: false,
        }
    }

    /// The project currently loaded in the editor.
    pub fn project(&self) -> &Project {
        self.engine.project()
    }
    /// Runs one scan when the editor is in run mode.
    #[rustfmt::skip]
    pub fn advance(&mut self) {
        if self.running { self.step(); }
    }

    /// Runs one scan and advances the simulated clock by one scan period.
    #[rustfmt::skip]
    fn step(&mut self) {
        self.cycles = self.cycles.saturating_add(1);
        let now_ms = self.cycles.saturating_mul(u64::from(self.project().scan.period_ms));
        self.engine.scan_once(now_ms);
    }

    fn note(&mut self, message: &str) {
        self.status = message.to_owned();
    }
    #[rustfmt::skip]
    fn menu_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Open…").clicked() { self.note("open: M2"); ui.close_menu(); }
                    if ui.button("Save").clicked() { self.note("save: M2"); ui.close_menu(); }
                    ui.separator();
                    if ui.button("Quit").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
                ui.menu_button("Run", |ui| {
                    if ui.button("Start").clicked() { self.running = true; ui.close_menu(); }
                    if ui.button("Stop").clicked() { self.running = false; ui.close_menu(); }
                    if ui.button("Single cycle").clicked() { self.step(); ui.close_menu(); }
                });
                ui.menu_button("Help", |ui| {
                    if ui.button("About").clicked() { self.show_about = true; ui.close_menu(); }
                });
            });
        });
    }
    #[rustfmt::skip]
    fn sections_panel(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("sections").default_width(200.0).show(ctx, |ui| {
            ui.heading("Sections");
            ui.separator();
            for (index, section) in self.engine.project().sections.iter().enumerate() {
                let label = format!("{} ({:?})", section.name, section.language);
                if ui.selectable_label(self.selected_section == index, label).clicked() {
                    self.selected_section = index;
                }
            }
        });
    }
    #[rustfmt::skip]
    fn watch_panel(&mut self, ctx: &egui::Context) {
        let watch = self.watch.clone();
        egui::SidePanel::right("watch").default_width(220.0).show(ctx, |ui| {
            ui.heading("Watch");
            ui.separator();
            for var in &watch {
                let current = self.engine.store().get(var);
                let text = current.map_or_else(|| "—".to_owned(), |value| format!("{value:?}"));
                ui.horizontal(|ui| {
                    ui.monospace(var.to_string());
                    ui.label(text);
                    if ui.small_button("toggle").clicked() {
                        let next = Value::Bit(!current.map(Value::as_bool).unwrap_or(false));
                        if self.engine.store_mut().set(var, next).is_err() {
                            self.note("variable out of range");
                        }
                    }
                });
            }
        });
    }
    #[rustfmt::skip]
    fn canvas_panel(&mut self, ctx: &egui::Context) {
        let painted = self.painted_section();
        egui::CentralPanel::default().show(ctx, |ui| {
            let sense = egui::Sense::click_and_drag();
            let (response, painter) = ui.allocate_painter(ui.available_size(), sense);
            let rect = response.rect;
            let background = egui::Color32::from_rgb(24, 26, 31);
            painter.rect_filled(rect, egui::CornerRadius::ZERO, background);
            if response.dragged() { self.pan += response.drag_delta(); }
            if response.hovered() {
                let scroll = ui.input(|input| input.raw_scroll_delta.y);
                if scroll != 0.0 {
                    self.zoom = (self.zoom * (1.0 + scroll * 0.001)).clamp(0.25, 4.0);
                }
            }
            let cell = 32.0 * self.zoom;
            let origin = rect.min + self.pan;
            let p = egui::pos2;
            let grid = egui::Stroke::new(1.0_f32, egui::Color32::from_gray(38));
            let (mut x, mut y) = (origin.x, origin.y);
            while x < rect.right() {
                painter.line_segment([p(x, rect.top()), p(x, rect.bottom())], grid);
                x += cell;
            }
            while y < rect.bottom() {
                painter.line_segment([p(rect.left(), y), p(rect.right(), y)], grid);
                y += cell;
            }
            for (index, (rung, states)) in painted.iter().enumerate() {
                paint_rung(&painter, origin, cell, index, rung, states);
            }
        });
    }

    /// Clones the selected section's rungs with the energised state of each
    /// element, so the canvas can be painted while pan and zoom are mutated.
    #[rustfmt::skip]
    fn painted_section(&self) -> Vec<(Rung, Vec<bool>)> {
        let store = self.engine.store();
        let Some(section) = self.engine.project().sections.get(self.selected_section) else {
            return Vec::new();
        };
        section.rungs.iter().filter_map(|id| self.engine.project().rung(*id)).map(|rung| {
            let states = rung.elements.iter().map(|e| powered(store, e)).collect();
            (rung.clone(), states)
        }).collect()
    }
}

/// `true` when an element's variable currently reads as a set bit.
#[rustfmt::skip]
fn powered(store: &VarStore, element: &PlacedElement) -> bool {
    element.var.as_ref().and_then(|v| store.get(v)).map(Value::as_bool).unwrap_or(false)
}

#[rustfmt::skip]
fn paint_rung(
    painter: &egui::Painter, origin: egui::Pos2, cell: f32, index: usize,
    rung: &Rung, states: &[bool],
) {
    let base_y = origin.y + 24.0 + index as f32 * cell * 1.8;
    let title = format!("#{} {}", rung.id, rung.label);
    let faint = egui::Color32::from_gray(140);
    let at = egui::pos2(origin.x + 6.0, base_y - 14.0);
    painter.text(at, egui::Align2::LEFT_TOP, title, egui::FontId::proportional(11.0), faint);
    let mono = egui::FontId::monospace(9.0);
    for (element, powered) in rung.elements.iter().zip(states) {
        let x = origin.x + 60.0 + f32::from(element.col) * cell;
        let y = base_y + f32::from(element.row) * cell * 0.9;
        let size = egui::vec2(cell * 0.8, cell * 0.5);
        let bounds = egui::Rect::from_min_size(egui::pos2(x, y), size);
        let on = egui::Color32::from_rgb(90, 220, 120);
        let color = if *powered { on } else { egui::Color32::from_gray(110) };
        let stroke = egui::Stroke::new(1.0_f32, color);
        painter.rect_stroke(bounds, egui::CornerRadius::ZERO, stroke, egui::StrokeKind::Inside);
        let name = element.var.as_ref().map_or(String::new(), ToString::to_string);
        let label = format!("{:?} {name}", element.kind);
        painter.text(bounds.center(), egui::Align2::CENTER_CENTER, label, mono.clone(), color);
    }
}

impl eframe::App for SoftLadderApp {
    #[rustfmt::skip]
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.advance();
        self.menu_bar(ctx);
        self.sections_panel(ctx);
        self.watch_panel(ctx);
        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                let state = if self.running { "RUN" } else { "STOP" };
                ui.label(format!("state: {state}   cycles: {}", self.cycles));
                ui.separator();
                ui.label(self.status.clone());
            });
        });
        self.canvas_panel(ctx);
        if self.show_about {
            let about = egui::Window::new("About").collapsible(false).resizable(false);
            about.show(ctx, |ui| {
                ui.label("SoftLadder — a clean-room Rust reimplementation of ClassicLadder.");
                ui.label("M0 skeleton: model, scan engine, project files and this editor.");
                ui.label("Credits: ClassicLadder by Marc Le Douarain (LGPL).");
                if ui.button("Close").clicked() { self.show_about = false; }
            });
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
    }
}
