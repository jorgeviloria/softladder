//! The editor application: view state, input handling and the frame loop.
//!
//! [`EditorApp`] owns a [`softladder_edit::Editor`] (the authoritative project,
//! history and problems) and a [`softladder_edit::Bench`] (the running program
//! plus the operator's positions), and nothing else of consequence. Selection,
//! the camera, the tool and the text buffers are view state: losing them costs
//! the user nothing, and every mutation goes through the editor.

use std::path::Path;
use std::time::{Duration, Instant};

use egui::Context;
use softladder_core::{
    Accessor, ElementKind, PlacedElement, Project, Rung, ScanConfig, Symbol, VarParseError, VarRef,
    VarStore,
};
use softladder_edit::{Bench, Editor, RuntimeState};

use crate::design::{Theme, Tokens, SPACE_1};
use crate::fileops;
use crate::layout::{self, Camera};
use crate::palette;
use crate::panels;
use crate::queries;
use crate::shortcuts::Action;

/// Which tab of the right-hand panel is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RightTab {
    /// The simulation bench.
    #[default]
    Bench,
    /// Live variable values.
    Watch,
    /// Diagnostics for the whole project.
    Problems,
    /// The project's symbol table.
    Symbols,
}

impl RightTab {
    /// Name shown on the tab.
    pub fn title(self) -> &'static str {
        match self {
            RightTab::Bench => "Bench",
            RightTab::Watch => "Watch",
            RightTab::Problems => "Problems",
            RightTab::Symbols => "Symbols",
        }
    }
}

/// What a click on the canvas does.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Tool {
    /// Select elements, drag them and edit their properties.
    Select,
    /// Place a palette element on the clicked cell.
    Place(ElementKind),
}

/// An operation that may have to discard unsaved changes first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pending {
    New,
    Open,
    Quit,
}

/// What the user answered in the unsaved-changes dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Choice {
    Save,
    Discard,
    Cancel,
}

/// One row of the symbol table while it is being edited.
///
/// The variable is kept as text so an unparseable spelling can be shown and
/// refused instead of silently dropped.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SymbolDraft {
    /// Symbolic name.
    pub name: String,
    /// Variable spelling, possibly empty or invalid.
    pub var: String,
    /// Human-readable comment.
    pub comment: String,
}

impl SymbolDraft {
    /// Drafts `symbol` for editing.
    pub fn from_symbol(symbol: &Symbol) -> Self {
        Self {
            name: symbol.name.clone(),
            var: symbol
                .var
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default(),
            comment: symbol.comment.clone(),
        }
    }

    /// Turns the draft back into a [`Symbol`], refusing a bad variable.
    pub fn to_symbol(&self) -> Result<Symbol, VarParseError> {
        let text = self.var.trim();
        let var = if text.is_empty() {
            None
        } else {
            Some(text.parse::<VarRef>()?)
        };
        Ok(Symbol {
            name: self.name.clone(),
            var,
            comment: self.comment.clone(),
            unit: None,
        })
    }
}

/// The canvas drag currently in progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Drag {
    /// Cell the drag started on.
    pub(crate) from: (u8, u8),
    /// Cell the pointer is over.
    pub(crate) over: Option<(u8, u8)>,
}

/// The SoftLadder editor, minus the window.
/// Which document the centre of the window shows.
///
/// The vendors all put documents — the program, the tag table, live values — in
/// the middle, side by side. See `docs/UX.md` §3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CentreTab {
    /// The ladder program.
    #[default]
    Ladder,
    /// The PLC tag table (the symbol table, promoted to a document).
    Tags,
    /// The simulation bench, laid out like an operator screen.
    Bench,
    /// The watch and force table.
    Watch,
    /// Diagnostics for the whole project.
    Problems,
}

impl CentreTab {
    /// Every tab, in the order they appear.
    pub const ALL: [CentreTab; 5] = [
        CentreTab::Ladder,
        CentreTab::Tags,
        CentreTab::Bench,
        CentreTab::Watch,
        CentreTab::Problems,
    ];

    /// The tab label.
    pub fn label(self) -> &'static str {
        match self {
            CentreTab::Ladder => "Ladder",
            CentreTab::Tags => "PLC tags",
            CentreTab::Bench => "Bench",
            CentreTab::Watch => "Watch & force",
            CentreTab::Problems => "Problems",
        }
    }
}

/// The whole editor: the documents, the bench and the view state.
///
/// It owns no program logic — every mutation goes through [`Editor`] — and no
/// rendering: `draw` composes the shell and hands each pane to `panels::*`.
pub struct EditorApp {
    /// The resolved design tokens of the active theme.
    pub(crate) tokens: Tokens,
    /// Which centre document is open.
    pub(crate) centre_tab: CentreTab,
    /// The theme the user chose.
    pub(crate) theme: Theme,
    /// Whether the canvas shows variable addresses as well as tag names.
    pub(crate) show_addresses: bool,
    /// Whether the right-hand inspector is visible.
    pub(crate) show_inspector: bool,
    /// Selected row of the PLC tag table.
    pub(crate) selected_tag: Option<usize>,
    /// Rows of the watch and force table.
    pub(crate) watch: Vec<crate::watch::WatchRow>,
    /// Variables currently forced, with the forced value.
    pub(crate) forces: Vec<(VarRef, bool)>,
    /// Buffer of the "add a tag" row.
    pub(crate) tag_draft: (String, String, String),
    pub(crate) editor: Editor,
    pub(crate) bench: Bench,
    pub(crate) selected_section: usize,
    pub(crate) selected_rung: Option<u32>,
    pub(crate) selection: Option<(u8, u8)>,
    pub(crate) camera: Camera,
    pub(crate) tool: Tool,
    pub(crate) right_tab: RightTab,
    pub(crate) show_right_panel: bool,
    pub(crate) var_buffer: String,
    pub(crate) params_buffer: String,
    pub(crate) var_error: Option<String>,
    pub(crate) symbols: Vec<SymbolDraft>,
    pub(crate) symbols_source: Vec<Symbol>,
    pub(crate) symbols_error: Option<String>,
    pub(crate) section_name_target: Option<u32>,
    pub(crate) section_name_buffer: String,
    pub(crate) rung_text_target: Option<u32>,
    pub(crate) rung_label_buffer: String,
    pub(crate) rung_comment_buffer: String,
    pub(crate) show_about: bool,
    pub(crate) show_shortcuts: bool,
    pub(crate) status: String,
    pub(crate) last_scan_ms: f64,
    edit_target: Option<(u32, u8, u8)>,
    pub(crate) drag: Option<Drag>,
    pending: Option<Pending>,
    force_close: bool,
    quit: bool,
    last_step: Option<Instant>,
    title: String,
}

impl EditorApp {
    /// Creates an editor around `project`.
    pub fn new(project: Project) -> Self {
        Self::with_editor(Editor::new(project))
    }

    /// Creates an editor around an already-built [`Editor`].
    pub fn with_editor(editor: Editor) -> Self {
        let bench = Bench::new(editor.project().clone());
        let mut app = Self {
            tokens: Tokens::light(),
            centre_tab: CentreTab::default(),
            theme: Theme::default(),
            show_addresses: false,
            show_inspector: true,
            selected_tag: None,
            watch: Vec::new(),
            forces: Vec::new(),
            tag_draft: (String::new(), String::new(), String::new()),
            editor,
            bench,
            selected_section: 0,
            selected_rung: None,
            selection: None,
            camera: Camera::new(),
            tool: Tool::Select,
            right_tab: RightTab::default(),
            show_right_panel: true,
            var_buffer: String::new(),
            params_buffer: String::new(),
            var_error: None,
            symbols: Vec::new(),
            symbols_source: Vec::new(),
            symbols_error: None,
            section_name_target: None,
            section_name_buffer: String::new(),
            rung_text_target: None,
            rung_label_buffer: String::new(),
            rung_comment_buffer: String::new(),
            show_about: false,
            show_shortcuts: false,
            status: String::new(),
            last_scan_ms: 0.0,
            edit_target: None,
            drag: None,
            pending: None,
            force_close: false,
            quit: false,
            last_step: None,
            title: String::new(),
        };
        app.reset_view();
        app.reload_symbols();
        app
    }

    /// The project being edited.
    pub fn project(&self) -> &Project {
        self.editor.project()
    }

    /// The authoritative editor.
    pub fn editor(&self) -> &Editor {
        &self.editor
    }

    /// The simulation bench.
    pub fn bench(&self) -> &Bench {
        &self.bench
    }

    /// The rung shown on the canvas, if the selection resolves to one.
    pub(crate) fn selected_rung_ref(&self) -> Option<&Rung> {
        self.selected_rung
            .and_then(|id| self.editor.project().rung(id))
    }

    /// The element under the cursor, if that cell holds one.
    pub(crate) fn selected_element(&self) -> Option<PlacedElement> {
        let rung = self.selected_rung?;
        let (col, row) = self.selection?;
        self.element_at(rung, col, row)
    }

    /// The element at `(col, row)` of `rung`, if any.
    pub(crate) fn element_at(&self, rung: u32, col: u8, row: u8) -> Option<PlacedElement> {
        self.editor
            .project()
            .rung(rung)?
            .elements
            .iter()
            .find(|element| element.col == col && element.row == row)
            .cloned()
    }

    /// Asks to close the application, prompting when there is unsaved work.
    pub fn request_quit(&mut self) {
        self.guard(Pending::Quit);
    }

    /// Writes a variable in the bench's store.
    ///
    /// This is how scripts, `.sltest`-style scenarios and the screenshot harness
    /// set an input without a mouse: it is the same store the bench scans.
    pub fn set_variable(
        &mut self,
        var: &VarRef,
        value: softladder_core::Value,
    ) -> Result<(), softladder_core::StoreError> {
        self.bench.engine_mut().store_mut().set(var, value)
    }

    /// Reads a variable out of the bench's store.
    pub fn variable(&self, var: &VarRef) -> Option<softladder_core::Value> {
        self.bench.engine().store().get(var)
    }

    /// Recomputes the Problems list from the project.
    pub fn refresh_diagnostics(&mut self) {
        self.editor.refresh_diagnostics();
        self.note("checked the program");
    }

    /// Opens a centre document: the program, the tags, the bench, the watch
    /// table or the diagnostics.
    ///
    /// The panels own these documents; this is the one entry point they need so
    /// that the headless screenshot harness can photograph each of them.
    pub fn show_document(&mut self, tab: CentreTab) {
        self.centre_tab = tab;
    }

    /// Chooses the palette, so a caller outside the panels (the screenshot
    /// harness) can photograph the dark theme as well as the light one.
    pub fn set_theme(&mut self, theme: Theme) {
        self.theme = theme;
        self.tokens = Tokens::for_theme(theme);
    }

    /// Selects the right-hand tab.
    pub fn show_tab(&mut self, tab: RightTab) {
        self.right_tab = tab;
        self.show_right_panel = true;
    }

    /// Selects a rung and, optionally, one of its cells.
    pub fn select(&mut self, rung: u32, cell: Option<(u8, u8)>) {
        self.select_rung(rung, cell);
    }

    /// Whether the bench is running.
    pub fn is_running(&self) -> bool {
        self.bench.state() == softladder_edit::RuntimeState::Run
    }

    /// The current frame, and everything the editor does between frames.
    pub fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        self.draw(ctx);
        self.advance_bench(ctx);
        if self.quit {
            self.quit = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    /// Draws one frame and handles its input.
    ///
    /// Split from [`EditorApp::update`] so a headless `egui::Context` can drive
    /// the whole interface — every widget and every painter call — without an
    /// `eframe::Frame`; the tests use exactly that.
    pub fn draw(&mut self, ctx: &Context) {
        // The tokens follow the theme; applying them every frame keeps a toggle
        // immediate without threading a "dirty style" flag through the panels.
        let tokens = Tokens::for_theme(self.theme);
        if tokens != self.tokens {
            self.tokens = tokens;
        }
        self.tokens.apply(ctx);
        self.handle_close_request(ctx);
        self.handle_shortcuts(ctx);
        egui::TopBottomPanel::top("ribbon").show(ctx, |ui| {
            ui.add_space(SPACE_1);
            crate::shell::ribbon(self, ui);
        });
        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| panels::status::show(self, ui));
        if self.show_inspector {
            egui::SidePanel::right("inspector")
                .resizable(true)
                .default_width(268.0)
                .min_width(200.0)
                .max_width(420.0)
                .show(ctx, |ui| panels::right::show(self, ui));
        }
        egui::SidePanel::left("left_panel")
            .resizable(true)
            .default_width(210.0)
            .min_width(160.0)
            .show(ctx, |ui| panels::left::show(self, ui));
        egui::CentralPanel::default().show(ctx, |ui| {
            crate::shell::centre(self, ui);
        });
        self.confirm_modal(ctx);
        self.windows(ctx);
        self.update_title(ctx);
    }

    // -- frame helpers ------------------------------------------------------

    /// Cancels the first close request while there are unsaved changes.
    fn handle_close_request(&mut self, ctx: &Context) {
        if !ctx.input(|input| input.viewport().close_requested()) || self.force_close {
            return;
        }
        if self.editor.is_dirty() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.pending = Some(Pending::Quit);
        } else {
            self.force_close = true;
        }
    }

    /// Runs exactly one bench cycle and records how long it took.
    pub(crate) fn bench_step(&mut self) {
        if self.bench.step().is_some() {
            self.last_scan_ms = self.bench.runtime().stats().last_ms;
        }
    }

    /// Hot-reloads the bench from the editor, keeping the operator's positions.
    ///
    /// Every edit goes through here, so the running program always follows the
    /// project while the switches and sliders stay where the operator left them
    /// (`Bench::reload` keeps them and the scan engine's store).
    fn reload_bench(&mut self) {
        let project = self.editor.project().clone();
        self.bench.reload(project);
    }

    /// Runs bench cycles while the runtime is scanning.
    ///
    /// A running bench scans once per configured scan period; a single-scan
    /// request scans on the very next frame.
    fn advance_bench(&mut self, ctx: &Context) {
        let period_ms = u64::from(self.editor.project().scan.period_ms.max(1));
        match self.bench.state() {
            RuntimeState::Run | RuntimeState::RunOneCycle => {
                let period = Duration::from_millis(period_ms);
                let due = self.last_step.is_none_or(|last| last.elapsed() >= period);
                if due {
                    self.bench_step();
                    self.last_step = Some(Instant::now());
                }
                ctx.request_repaint_after(period);
            }
            _ => self.last_step = None,
        }
    }

    /// Sends the window title only when it changes.
    fn update_title(&mut self, ctx: &Context) {
        let title = queries::window_title(
            &self.editor.project().name,
            self.editor.is_dirty(),
            self.editor.path(),
        );
        if title != self.title {
            self.title = title.clone();
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
        }
    }

    /// Applies every keyboard action of this frame.
    fn handle_shortcuts(&mut self, ctx: &Context) {
        let typing = ctx.wants_keyboard_input();
        let events: Vec<(egui::Key, egui::Modifiers)> = ctx.input(|input| {
            input
                .events
                .iter()
                .filter_map(|event| match event {
                    egui::Event::Key {
                        key,
                        pressed: true,
                        repeat: false,
                        modifiers,
                        ..
                    } => Some((*key, *modifiers)),
                    _ => None,
                })
                .collect()
        });
        for (key, modifiers) in events {
            if let Some(action) = crate::shortcuts::global_action(key, modifiers) {
                self.handle(action);
            } else if !typing && !crate::sfc::is_sfc(self.project(), self.selected_section) {
                // The ladder's editing keys. An SFC section owns its own map
                // (the sequential document applies it while it draws), so the two
                // never act on the same key press.
                if let Some(action) = crate::shortcuts::canvas_action(key, modifiers) {
                    self.handle(action);
                }
            }
        }
    }

    // -- commands -----------------------------------------------------------

    /// Performs one [`Action`].
    /// Runs one command, exactly as the menu, a toolbar button or a shortcut
    /// would. This is the entry point automation and the frame tests use.
    pub fn handle(&mut self, action: Action) {
        match action {
            Action::Undo => {
                if self.editor.undo() {
                    self.after_edit();
                    self.note("undo");
                }
            }
            Action::Redo => {
                if self.editor.redo() {
                    self.after_edit();
                    self.note("redo");
                }
            }
            Action::New => self.guard(Pending::New),
            Action::Open => self.guard(Pending::Open),
            Action::Save => self.save(),
            Action::SaveAs => self.save_as(),
            Action::RunStop => self.toggle_run(),
            Action::SingleScan => self.single_scan(),
            Action::AutoFillBench => self.auto_fill_bench(),
            Action::Delete => self.delete_selection(),
            Action::MoveLeft => self.nudge(-1, 0),
            Action::MoveRight => self.nudge(1, 0),
            Action::MoveUp => self.nudge(0, -1),
            Action::MoveDown => self.nudge(0, 1),
            Action::ToggleVerticalLink => self.toggle_vertical_link(),
            Action::ZoomIn => self.camera.zoom_in(),
            Action::ZoomOut => self.camera.zoom_out(),
            Action::ZoomReset => self.camera.reset(),
            Action::ShortcutHelp => self.show_shortcuts = true,
            Action::Cancel => {
                self.tool = Tool::Select;
                self.drag = None;
                self.selection = None;
                self.load_properties();
            }
            Action::Pick(kind) => {
                self.tool = Tool::Place(kind);
                self.note(&format!("place {}", palette::short_name(kind)));
            }
        }
    }

    /// Marks the project as edited: the bench must hot-reload the new program.
    pub(crate) fn after_edit(&mut self) {
        self.reload_bench();
        self.section_name_target = None;
        self.rung_text_target = None;
        self.load_properties();
    }

    /// Selects a section by index and the first rung it owns.
    pub(crate) fn select_section(&mut self, index: usize) {
        self.selected_section = index;
        self.selected_rung = queries::section_rungs(self.editor.project(), index)
            .first()
            .copied();
        self.selection = Some((0, 0));
        self.drag = None;
        self.load_properties();
    }

    /// Selects a rung by id, switching to the section that owns it.
    pub(crate) fn select_rung(&mut self, rung: u32, cell: Option<(u8, u8)>) {
        if let Some(found) = queries::find_rung(self.editor.project(), rung) {
            self.selected_section = found.section;
        }
        self.selected_rung = Some(rung);
        self.selection = cell.or(Some((0, 0)));
        self.drag = None;
        self.load_properties();
    }

    /// Moves the cursor, dragging the element under it along.
    pub(crate) fn nudge(&mut self, dx: i32, dy: i32) {
        let (col, row) = self.selection.unwrap_or((0, 0));
        let col = (i32::from(col) + dx).clamp(0, i32::from(layout::MAX_COL)) as u8;
        let row = (i32::from(row) + dy).clamp(0, i32::from(layout::MAX_ROW)) as u8;
        let target = (col, row);
        let Some(rung) = self.selected_rung else {
            self.selection = Some(target);
            return;
        };
        let Some(from) = self.selection else {
            self.selection = Some(target);
            return;
        };
        if from != target && self.element_at(rung, from.0, from.1).is_some() {
            if self.element_at(rung, target.0, target.1).is_some() {
                self.note("that cell is occupied");
            } else if let Err(error) = self.editor.move_element(rung, from, target) {
                self.note(&error.to_string());
            } else {
                self.after_edit();
            }
        }
        self.selection = Some(target);
        self.load_properties();
    }

    /// Deletes the element under the cursor.
    pub(crate) fn delete_selection(&mut self) {
        let (Some(rung), Some((col, row))) = (self.selected_rung, self.selection) else {
            return;
        };
        match self.editor.delete_element(rung, col, row) {
            Ok(()) => {
                self.after_edit();
                self.note("element deleted");
            }
            Err(error) => self.note(&error.to_string()),
        }
    }

    /// Toggles the vertical link of the cell under the cursor.
    pub(crate) fn toggle_vertical_link(&mut self) {
        let (Some(rung), Some((col, row))) = (self.selected_rung, self.selection) else {
            return;
        };
        let Some(element) = self.element_at(rung, col, row) else {
            self.note("no element on that cell");
            return;
        };
        let linked = !element.connected_with_top;
        match self.editor.set_vertical_link(rung, col, row, linked) {
            Ok(()) => {
                self.after_edit();
                self.note(if linked {
                    "vertical link set"
                } else {
                    "vertical link cleared"
                });
            }
            Err(error) => self.note(&error.to_string()),
        }
    }

    /// Puts the armed palette element on `(col, row)`.
    pub(crate) fn place_at(&mut self, col: u8, row: u8) {
        let Tool::Place(kind) = self.tool else {
            self.selection = Some((col, row));
            self.load_properties();
            return;
        };
        let Some(rung) = self.selected_rung else {
            self.note("select a rung first");
            return;
        };
        let command = palette::replace_command(rung, kind, col, row);
        match self.editor.apply(command) {
            Ok(()) => {
                self.selection = Some((col, row));
                self.tool = Tool::Select;
                self.after_edit();
                self.note(&format!("placed {}", palette::short_name(kind)));
            }
            Err(error) => self.note(&error.to_string()),
        }
    }

    /// Commits the variable field, refusing a spelling that does not parse.
    pub(crate) fn apply_var(&mut self) {
        let Some((rung, col, row)) = self.edit_target else {
            return;
        };
        let text = self.var_buffer.trim().to_owned();
        let var = if text.is_empty() {
            None
        } else {
            match text.parse::<VarRef>() {
                Ok(var) => Some(var),
                Err(error) => {
                    self.var_error = Some(error.to_string());
                    return;
                }
            }
        };
        self.var_error = None;
        let current = self
            .element_at(rung, col, row)
            .and_then(|element| element.var);
        if current.as_ref() == var.as_ref() {
            self.load_properties();
            return;
        }
        match self.editor.set_element_var(rung, col, row, var) {
            Ok(()) => {
                self.after_edit();
                self.note("variable set");
            }
            Err(error) => self.note(&error.to_string()),
        }
    }

    /// Commits the parameter field.
    pub(crate) fn apply_params(&mut self) {
        let Some((rung, col, row)) = self.edit_target else {
            return;
        };
        let params: Vec<String> = self
            .params_buffer
            .split_whitespace()
            .map(str::to_owned)
            .collect();
        let current = self
            .element_at(rung, col, row)
            .map(|element| element.params)
            .unwrap_or_default();
        if current == params {
            return;
        }
        let borrowed: Vec<&str> = params.iter().map(String::as_str).collect();
        match self.editor.set_element_params(rung, col, row, &borrowed) {
            Ok(()) => {
                self.after_edit();
                self.note("parameters set");
            }
            Err(error) => self.note(&error.to_string()),
        }
    }

    /// Refreshes the property buffers from the element under the cursor.
    pub(crate) fn load_properties(&mut self) {
        match self.selected_element() {
            Some(element) => {
                self.var_buffer = element
                    .var
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_default();
                self.params_buffer = element.params.join(" ");
                self.var_error = None;
                self.edit_target = self
                    .selected_rung
                    .map(|rung| (rung, element.col, element.row));
            }
            None => {
                self.var_buffer.clear();
                self.params_buffer.clear();
                self.var_error = None;
                self.edit_target = None;
            }
        }
    }

    // -- bench --------------------------------------------------------------

    /// Starts the bench when it is stopped and stops it when it is running.
    pub(crate) fn toggle_run(&mut self) {
        if self.bench.state().is_scanning() {
            self.bench.stop();
            self.note("stopped");
        } else {
            self.bench.start();
            self.last_step = None;
            self.note("running");
        }
    }

    /// Asks for exactly one scan.
    /// Advances the bench by exactly one scan, as the menu and the shortcut do.
    pub fn single_scan(&mut self) {
        self.bench.run_one_cycle();
        self.last_step = None;
        self.note("single scan");
    }

    /// Rebuilds the bench panel from the program's physical variables.
    pub(crate) fn auto_fill_bench(&mut self) {
        match self.editor.auto_fill_panel() {
            Ok(()) => {
                self.after_edit();
                self.note("bench auto-filled from the program");
            }
            Err(error) => self.note(&error.to_string()),
        }
    }

    /// Replaces the scan configuration.
    pub(crate) fn set_scan_config(&mut self, scan: ScanConfig) {
        if self.editor.project().scan == scan {
            return;
        }
        match self.editor.set_scan_config(scan) {
            Ok(()) => {
                self.after_edit();
                self.note("scan configuration set");
            }
            Err(error) => self.note(&error.to_string()),
        }
    }

    /// Replaces the bench panel with `panel`.
    pub(crate) fn set_panel(&mut self, panel: softladder_core::SimulationPanel) {
        if self.editor.project().simulation == panel {
            return;
        }
        match self.editor.set_panel(panel) {
            Ok(()) => self.after_edit(),
            Err(error) => self.note(&error.to_string()),
        }
    }

    // -- project structure --------------------------------------------------

    /// Appends a section named `name`.
    pub(crate) fn add_section(&mut self, name: &str) {
        match self
            .editor
            .add_section(name, softladder_core::SectionLanguage::Ladder)
        {
            Ok(id) => {
                self.after_edit();
                let index = self
                    .editor
                    .project()
                    .sections
                    .iter()
                    .position(|section| section.id == id);
                if let Some(index) = index {
                    self.select_section(index);
                }
                self.note("section added");
            }
            Err(error) => self.note(&error.to_string()),
        }
    }

    /// Removes the section at `index`.
    pub(crate) fn remove_section(&mut self, index: usize) {
        let Some(id) = self
            .editor
            .project()
            .sections
            .get(index)
            .map(|section| section.id)
        else {
            return;
        };
        match self.editor.remove_section(id) {
            Ok(()) => {
                self.after_edit();
                let last = self.editor.project().sections.len().saturating_sub(1);
                self.select_section(index.min(last));
                self.note("section removed");
            }
            Err(error) => self.note(&error.to_string()),
        }
    }

    /// Renames the section at `index`.
    pub(crate) fn rename_section(&mut self, index: usize, name: &str) {
        let Some(id) = self
            .editor
            .project()
            .sections
            .get(index)
            .map(|section| section.id)
        else {
            return;
        };
        match self.editor.rename_section(id, name) {
            Ok(()) => self.after_edit(),
            Err(error) => self.note(&error.to_string()),
        }
    }

    /// Inserts an empty rung at `position` in the section at `index`.
    pub(crate) fn insert_rung(&mut self, index: usize, position: usize) {
        let Some(section) = self
            .editor
            .project()
            .sections
            .get(index)
            .map(|section| section.id)
        else {
            return;
        };
        match self.editor.insert_rung(section, position) {
            Ok(id) => {
                self.after_edit();
                self.select_section(index);
                self.select_rung(id, Some((0, 0)));
                self.note("rung inserted");
            }
            Err(error) => self.note(&error.to_string()),
        }
    }

    /// Deletes `rung` from the section at `index`.
    pub(crate) fn delete_rung(&mut self, index: usize, rung: u32) {
        let Some(section) = self
            .editor
            .project()
            .sections
            .get(index)
            .map(|section| section.id)
        else {
            return;
        };
        match self.editor.delete_rung(section, rung) {
            Ok(()) => {
                self.after_edit();
                let rungs = queries::section_rungs(self.editor.project(), index);
                self.selected_rung = rungs.first().copied();
                self.selection = Some((0, 0));
                self.load_properties();
                self.note("rung deleted");
            }
            Err(error) => self.note(&error.to_string()),
        }
    }

    /// Replaces a rung's label and comment.
    pub(crate) fn set_rung_text(&mut self, rung: u32, label: &str, comment: &str) {
        if self
            .editor
            .project()
            .rung(rung)
            .is_some_and(|target| target.label == label && target.comment == comment)
        {
            return;
        }
        match self.editor.set_rung_text(rung, label, comment) {
            Ok(()) => self.after_edit(),
            Err(error) => self.note(&error.to_string()),
        }
    }

    // -- symbols ------------------------------------------------------------

    /// Rebuilds the drafts from the project's symbol table.
    pub(crate) fn reload_symbols(&mut self) {
        self.symbols_source = self.editor.project().symbols.clone();
        self.symbols = self
            .symbols_source
            .iter()
            .map(SymbolDraft::from_symbol)
            .collect();
        self.symbols_error = None;
    }

    /// Writes the drafts back through [`Editor::set_symbols`].
    /// Applies the symbol drafts to the editor.
    ///
    /// The PLC tags document edits symbols through the table itself, so this is
    /// only exercised by tests today; it stays because it is the obvious entry
    /// point for a bulk symbol edit.
    #[cfg(test)]
    pub(crate) fn apply_symbols(&mut self) {
        let mut symbols = Vec::with_capacity(self.symbols.len());
        for (index, draft) in self.symbols.iter().enumerate() {
            match draft.to_symbol() {
                Ok(symbol) => symbols.push(symbol),
                Err(error) => {
                    self.symbols_error = Some(format!(
                        "row {}: `{}` is not a variable: {error}",
                        index + 1,
                        draft.var
                    ));
                    return;
                }
            }
        }
        self.symbols_error = None;
        match self.editor.set_symbols(symbols) {
            Ok(()) => {
                self.after_edit();
                self.note("symbols saved");
            }
            Err(error) => self.note(&error.to_string()),
        }
    }

    // -- files --------------------------------------------------------------

    /// Asks before discarding unsaved changes, then performs `pending`.
    fn guard(&mut self, pending: Pending) {
        if self.editor.is_dirty() {
            self.pending = Some(pending);
        } else {
            self.perform(pending);
        }
    }

    /// Performs `pending`, whatever the dirty flag says.
    fn perform(&mut self, pending: Pending) {
        match pending {
            Pending::New => {
                self.editor = Editor::new(Project::new("untitled"));
                self.after_load();
                self.note("new project");
            }
            Pending::Open => {
                if let Some(path) = fileops::open_dialog() {
                    self.open_path(&path);
                }
            }
            Pending::Quit => {
                self.force_close = true;
                self.quit = true;
            }
        }
    }

    /// Opens `path` through [`Editor::open`].
    pub(crate) fn open_path(&mut self, path: &Path) {
        match Editor::open(path) {
            Ok(editor) => {
                self.editor = editor;
                self.after_load();
                self.status = format!("opened {}", path.display());
            }
            Err(error) => self.note(&format!("open failed: {error}")),
        }
    }

    /// Saves to the current path, or asks for one when there is none.
    pub(crate) fn save(&mut self) {
        if self.editor.path().is_none() {
            self.save_as();
            return;
        }
        match self.editor.save_current() {
            Ok(()) => self.note("saved"),
            Err(error) => self.note(&format!("save failed: {error}")),
        }
    }

    /// Asks for a path and saves to it.
    pub(crate) fn save_as(&mut self) {
        let suggestion =
            fileops::suggested_name(self.editor.path(), self.editor.project().name.as_str());
        let Some(path) = fileops::save_dialog(self.editor.path(), &suggestion) else {
            return;
        };
        match self.editor.save(&path) {
            Ok(()) => self.status = format!("saved {}", path.display()),
            Err(error) => self.note(&format!("save failed: {error}")),
        }
    }

    /// Resets the selection, camera and buffers after a project is swapped in.
    fn after_load(&mut self) {
        self.bench = Bench::new(self.editor.project().clone());
        self.camera.reset();
        self.tool = Tool::Select;
        self.drag = None;
        self.last_step = None;
        self.last_scan_ms = 0.0;
        self.var_error = None;
        self.section_name_target = None;
        self.rung_text_target = None;
        self.reload_symbols();
        self.select_section(0);
    }

    /// Clears the selection and points the canvas at the first rung.
    fn reset_view(&mut self) {
        self.select_section(0);
        self.status = String::new();
    }

    /// Sets the status-bar note.
    pub(crate) fn note(&mut self, message: &str) {
        self.status = message.to_owned();
    }

    /// The current status-bar note.
    pub(crate) fn status(&self) -> &str {
        &self.status
    }

    // -- menus and dialogs --------------------------------------------------

    fn confirm_modal(&mut self, ctx: &Context) {
        let Some(pending) = self.pending else {
            return;
        };
        let mut choice = None;
        egui::Window::new("Unsaved changes")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label("This project has unsaved changes.");
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        choice = Some(Choice::Save);
                    }
                    if ui.button("Discard").clicked() {
                        choice = Some(Choice::Discard);
                    }
                    if ui.button("Cancel").clicked() {
                        choice = Some(Choice::Cancel);
                    }
                });
            });
        match choice {
            Some(Choice::Save) => {
                self.save();
                if !self.editor.is_dirty() {
                    self.pending = None;
                    self.perform(pending);
                }
            }
            Some(Choice::Discard) => {
                self.pending = None;
                self.perform(pending);
            }
            Some(Choice::Cancel) => self.pending = None,
            None => {}
        }
    }

    fn windows(&mut self, ctx: &Context) {
        if self.show_about {
            egui::Window::new("About SoftLadder")
                .collapsible(false)
                .resizable(false)
                .open(&mut self.show_about)
                .show(ctx, |ui| {
                    ui.label("SoftLadder — a clean-room Rust reimplementation of ClassicLadder.");
                    ui.label("M2 editor: ladder editing, undo/redo, live simulation bench.");
                    ui.label("Credits: ClassicLadder by Marc Le Douarain (LGPL).");
                });
        }
        if self.show_shortcuts {
            egui::Window::new("Keyboard shortcuts")
                .collapsible(false)
                .resizable(false)
                .open(&mut self.show_shortcuts)
                .show(ctx, panels::shortcuts::show);
        }
    }
}

/// The store the live indication reads from.
pub(crate) fn store(app: &EditorApp) -> &VarStore {
    app.bench.engine().store()
}

/// Reads a block sub-value (timer elapsed, counter value, register count).
pub(crate) fn block_of(
    app: &EditorApp,
    element: &PlacedElement,
    accessor: Accessor,
) -> Option<softladder_core::Value> {
    queries::block_value(element, store(app), accessor)
}

#[cfg(test)]
mod tests {
    use super::*;
    use softladder_core::{Rung, Section, Severity, Symbol, Value};

    fn project() -> Project {
        let mut project = Project::new("app tests");
        let mut main = Section::new(1, "Main");
        main.rungs.push(1);
        project.sections.push(main);
        project.rungs.push(Rung {
            elements: vec![
                PlacedElement::with_var(ElementKind::ContactNo, var("%I0"), 0, 0),
                PlacedElement::with_var(ElementKind::CoilOut, var("%Q0"), 1, 0),
            ],
            ..Rung::new(1)
        });
        project
    }

    fn var(text: &str) -> VarRef {
        text.parse().expect("test variable parses")
    }

    #[test]
    fn a_new_app_selects_the_first_section_and_rung() {
        let app = EditorApp::new(project());
        assert_eq!(app.selected_section, 0);
        assert_eq!(app.selected_rung, Some(1));
        assert_eq!(app.selection, Some((0, 0)));
        assert_eq!(app.tool, Tool::Select);
        assert!(!app.editor.is_dirty());
        assert!(!app.show_right_panel || app.right_tab == RightTab::Bench);
    }

    #[test]
    fn an_empty_project_opens_without_a_selection_or_a_panic() {
        let app = EditorApp::new(Project::new("empty"));
        assert_eq!(app.selected_rung, None);
        assert_eq!(app.selected_element(), None);
        assert!(app.editor.problems().is_empty());
        assert_eq!(app.project().sections.len(), 0);
    }

    #[test]
    fn placing_an_element_edits_the_project_and_marks_it_dirty() {
        let mut app = EditorApp::new(project());
        app.handle(Action::Pick(ElementKind::ContactNc));
        app.place_at(2, 0);
        let placed = app.element_at(1, 2, 0).expect("placed");
        assert_eq!(placed.kind, ElementKind::ContactNc);
        assert_eq!(placed.var, Some(var("%I2")));
        assert!(app.editor.is_dirty());
        assert_eq!(app.selection, Some((2, 0)));
        assert_eq!(app.tool, Tool::Select, "the tool returns to the pointer");
        assert_eq!(
            app.bench.runtime().project,
            *app.project(),
            "the bench hot-reloaded the edited program"
        );
    }

    #[test]
    fn placing_on_an_occupied_cell_replaces_it_in_one_undo_step() {
        let mut app = EditorApp::new(project());
        app.handle(Action::Pick(ElementKind::CoilOut));
        app.place_at(0, 0);
        assert_eq!(
            app.element_at(1, 0, 0).map(|e| e.kind),
            Some(ElementKind::CoilOut)
        );
        assert_eq!(app.editor.history_len(), 1, "replacement is one command");
        assert!(app.editor.undo());
        assert_eq!(
            app.element_at(1, 0, 0).map(|e| e.kind),
            Some(ElementKind::ContactNo)
        );
    }

    #[test]
    fn drawing_a_vertical_link_toggles_it() {
        let mut app = EditorApp::new(project());
        app.select_rung(1, Some((0, 0)));
        app.toggle_vertical_link();
        assert!(app
            .element_at(1, 0, 0)
            .is_some_and(|e| e.connected_with_top));
        app.toggle_vertical_link();
        assert!(!app
            .element_at(1, 0, 0)
            .is_some_and(|e| e.connected_with_top));
        assert_eq!(app.editor.history_len(), 2);
    }

    #[test]
    fn deleting_removes_the_element_and_undo_restores_it() {
        let mut app = EditorApp::new(project());
        app.select_rung(1, Some((1, 0)));
        app.delete_selection();
        assert_eq!(app.element_at(1, 1, 0), None);
        assert!(app.editor.undo());
        assert!(app.element_at(1, 1, 0).is_some());
    }

    #[test]
    fn nudging_moves_the_cursor_and_drags_the_element_along() {
        let mut app = EditorApp::new(project());
        app.select_rung(1, Some((1, 0)));
        app.nudge(0, 1);
        assert_eq!(app.selection, Some((1, 1)));
        assert_eq!(
            app.element_at(1, 1, 1).map(|e| e.kind),
            Some(ElementKind::CoilOut),
            "the coil moved with the cursor"
        );
        assert_eq!(app.element_at(1, 1, 0), None);

        // An empty cell just moves the cursor.
        app.select_rung(1, Some((5, 0)));
        app.nudge(1, 0);
        assert_eq!(app.selection, Some((6, 0)));

        // The cursor cannot leave the grid.
        app.selection = Some((0, 0));
        app.nudge(-1, -1);
        assert_eq!(app.selection, Some((0, 0)));
        app.selection = Some((layout::MAX_COL, layout::MAX_ROW));
        app.nudge(1, 1);
        assert_eq!(app.selection, Some((layout::MAX_COL, layout::MAX_ROW)));
    }

    #[test]
    fn nudging_onto_an_occupied_cell_refuses_without_moving_anything() {
        let mut app = EditorApp::new(project());
        app.select_rung(1, Some((1, 0)));
        app.nudge(-1, 0);
        assert!(app.status().contains("occupied"));
        assert!(app.element_at(1, 1, 0).is_some(), "the coil stayed put");
        assert!(app.element_at(1, 0, 0).is_some(), "the contact is intact");
    }

    #[test]
    fn a_variable_that_does_not_parse_leaves_the_element_alone() {
        let mut app = EditorApp::new(project());
        app.select_rung(1, Some((0, 0)));
        app.var_buffer = "not a variable".to_owned();
        app.apply_var();
        assert!(app.var_error.is_some(), "the error is reported");
        assert_eq!(
            app.element_at(1, 0, 0).and_then(|e| e.var),
            Some(var("%I0")),
            "the element keeps its old value"
        );
        assert_eq!(app.editor.history_len(), 0, "nothing was recorded");
    }

    #[test]
    fn a_valid_variable_updates_the_element_and_shows_the_canonical_form() {
        let mut app = EditorApp::new(project());
        app.select_rung(1, Some((0, 0)));
        app.var_buffer = "%B3".to_owned();
        app.apply_var();
        assert_eq!(app.var_error, None);
        assert_eq!(
            app.element_at(1, 0, 0)
                .and_then(|e| e.var)
                .map(|v| v.to_string()),
            Some("%M3".to_owned()),
            "ClassicLadder aliases are canonicalised"
        );
        assert_eq!(app.var_buffer, "%M3");
    }

    #[test]
    fn an_empty_variable_field_unbinds_the_element() {
        let mut app = EditorApp::new(project());
        app.select_rung(1, Some((0, 0)));
        app.var_buffer.clear();
        app.apply_var();
        assert_eq!(app.element_at(1, 0, 0).and_then(|e| e.var), None);
        assert!(
            app.editor.problems().iter().any(|d| d.code == "SL-E004"),
            "an element without a variable is reported: {:?}",
            app.editor.problems()
        );
    }

    #[test]
    fn editing_parameters_replaces_the_list() {
        let mut app = EditorApp::new(project());
        app.handle(Action::Pick(ElementKind::Timer {
            mode: softladder_core::TimerMode::On,
        }));
        app.place_at(2, 0);
        app.params_buffer = "250".to_owned();
        app.apply_params();
        assert_eq!(
            app.element_at(1, 2, 0).map(|e| e.params),
            Some(vec!["250".to_owned()])
        );

        app.params_buffer = "  10   20  ".to_owned();
        app.apply_params();
        assert_eq!(
            app.element_at(1, 2, 0).map(|e| e.params),
            Some(vec!["10".to_owned(), "20".to_owned()])
        );
    }

    #[test]
    fn undo_and_redo_drive_the_dirty_flag_and_the_bench_reload() {
        let mut app = EditorApp::new(project());
        app.handle(Action::Pick(ElementKind::ContactNo));
        app.place_at(3, 0);
        assert!(app.editor.is_dirty());

        app.handle(Action::Undo);
        assert_eq!(
            app.bench.runtime().project,
            *app.project(),
            "undo reloads the bench too"
        );
        assert_eq!(app.element_at(1, 3, 0), None);
        app.handle(Action::Redo);
        assert!(app.element_at(1, 3, 0).is_some());
    }

    #[test]
    fn editing_while_running_keeps_the_operators_positions() {
        let mut app = EditorApp::new(project());
        app.handle(Action::AutoFillBench);
        assert_eq!(app.project().simulation.switches.len(), 1);
        app.handle(Action::RunStop);
        assert_eq!(app.bench.state(), RuntimeState::Run);
        app.bench.panel_state_mut().toggle(0);
        app.bench_step();
        assert_eq!(app.bench.readings()[0].value, Value::Bit(true));

        // Redirect the coil while the bench is running.
        app.select_rung(1, Some((1, 0)));
        app.var_buffer = "%Q1".to_owned();
        app.apply_var();
        assert!(
            app.bench.panel_state().is_closed(0),
            "the switch stays where the operator left it"
        );
        assert_eq!(app.bench.state(), RuntimeState::Run, "reload keeps running");

        app.bench_step();
        assert_eq!(
            app.bench.engine().store().get(&var("%Q1")),
            Some(Value::Bit(true)),
            "the bench now runs the edited program"
        );
    }

    #[test]
    fn run_stop_and_single_scan_follow_the_runtime_state() {
        let mut app = EditorApp::new(project());
        assert_eq!(app.bench.state(), RuntimeState::Stop);
        app.handle(Action::RunStop);
        assert_eq!(app.bench.state(), RuntimeState::Run);
        app.handle(Action::RunStop);
        assert_eq!(app.bench.state(), RuntimeState::Stop);

        app.handle(Action::SingleScan);
        assert_eq!(app.bench.state(), RuntimeState::RunOneCycle);
    }

    #[test]
    fn auto_fill_builds_a_bench_for_the_program() {
        let mut app = EditorApp::new(project());
        app.handle(Action::AutoFillBench);
        let panel = &app.project().simulation;
        assert_eq!(panel.switches.len(), 1);
        assert_eq!(panel.lamps.len(), 1);
        assert_eq!(app.bench.runtime().project, *app.project());
    }

    #[test]
    fn the_canvas_tool_arms_and_disarms() {
        let mut app = EditorApp::new(project());
        app.handle(Action::Pick(ElementKind::CoilSet));
        assert_eq!(app.tool, Tool::Place(ElementKind::CoilSet));
        app.handle(Action::Cancel);
        assert_eq!(app.tool, Tool::Select);
        assert_eq!(app.selection, None);
    }

    #[test]
    fn sections_and_rungs_can_be_added_renamed_and_removed() {
        let mut app = EditorApp::new(project());
        app.add_section("Sub");
        assert_eq!(app.project().sections.len(), 2);
        assert_eq!(app.selected_section, 1);
        assert!(app.selected_rung.is_none(), "the new section is empty");

        app.insert_rung(1, 0);
        let inserted = app.selected_rung.expect("a rung was selected");
        assert_eq!(
            app.project().section(2).map(|s| s.rungs.clone()),
            Some(vec![inserted])
        );

        app.set_rung_text(inserted, "STEP", "first step");
        let target = app.project().rung(inserted).expect("rung");
        assert_eq!(target.label, "STEP");
        assert_eq!(target.comment, "first step");

        app.rename_section(1, "Subroutine");
        assert_eq!(app.project().sections[1].name, "Subroutine");

        app.delete_rung(1, inserted);
        assert!(app.project().rung(inserted).is_none());
        assert!(app.selected_rung.is_none());

        app.remove_section(1);
        assert_eq!(app.project().sections.len(), 1);
    }

    #[test]
    fn removing_the_last_section_and_rung_leaves_a_usable_editor() {
        let mut app = EditorApp::new(project());
        app.delete_rung(0, 1);
        assert_eq!(app.project().rungs.len(), 0);
        app.remove_section(0);
        assert_eq!(app.project().sections.len(), 0);
        assert_eq!(app.selected_rung, None);
        // Everything that reads the selection must keep working.
        app.delete_selection();
        app.toggle_vertical_link();
        app.nudge(1, 0);
        app.handle(Action::Undo);
        app.handle(Action::Redo);
        assert!(app.selection.is_some(), "the cursor survives undo and redo");
    }

    #[test]
    fn symbol_drafts_round_trip_and_refuse_bad_variables() {
        let symbol = Symbol {
            name: "start".to_owned(),
            var: Some(var("%I0")),
            comment: "start button".to_owned(),
            unit: None,
        };
        let draft = SymbolDraft::from_symbol(&symbol);
        assert_eq!(draft.var, "%I0");
        assert_eq!(draft.to_symbol().expect("valid"), symbol);

        let empty = SymbolDraft {
            name: "spare".to_owned(),
            var: "  ".to_owned(),
            comment: String::new(),
        };
        assert_eq!(empty.to_symbol().expect("valid").var, None);

        let broken = SymbolDraft {
            name: "oops".to_owned(),
            var: "%nonsense".to_owned(),
            comment: String::new(),
        };
        assert!(broken.to_symbol().is_err());
    }

    #[test]
    fn applying_symbols_saves_them_and_reports_bad_rows() {
        let mut app = EditorApp::new(project());
        app.symbols = vec![SymbolDraft {
            name: "start".to_owned(),
            var: "%B0".to_owned(),
            comment: "start".to_owned(),
        }];
        app.apply_symbols();
        assert_eq!(app.symbols_error, None);
        assert_eq!(app.project().symbols.len(), 1);
        assert_eq!(
            app.project().symbols[0]
                .var
                .as_ref()
                .map(ToString::to_string),
            Some("%M0".to_owned()),
            "the alias is canonicalised when it is saved"
        );

        app.symbols.push(SymbolDraft {
            name: "bad".to_owned(),
            var: "%?".to_owned(),
            comment: String::new(),
        });
        let history = app.editor.history_len();
        app.apply_symbols();
        assert!(app.symbols_error.is_some());
        assert_eq!(app.editor.history_len(), history, "nothing was applied");
        assert_eq!(app.project().symbols.len(), 1);
    }

    #[test]
    fn changing_the_scan_period_goes_through_the_editor() {
        let mut app = EditorApp::new(project());
        let scan = ScanConfig {
            period_ms: 25,
            input_period_ms: 5,
        };
        app.set_scan_config(scan);
        assert_eq!(app.project().scan, scan);
        assert_eq!(app.bench.runtime().project, *app.project());
        let history = app.editor.history_len();
        app.set_scan_config(scan);
        assert_eq!(
            app.editor.history_len(),
            history,
            "no-op edits record nothing"
        );
    }

    #[test]
    fn a_dirty_project_guards_new_and_open_until_the_user_decides() {
        let mut app = EditorApp::new(project());
        app.handle(Action::Pick(ElementKind::ContactNo));
        app.place_at(4, 0);
        assert!(app.editor.is_dirty());

        app.handle(Action::New);
        assert_eq!(app.pending, Some(Pending::New), "the modal is queued");
        assert!(app.element_at(1, 4, 0).is_some(), "nothing changed yet");

        app.pending = None;
        app.editor.mark_clean();
        app.handle(Action::New);
        assert_eq!(app.pending, None);
        assert_eq!(app.project().sections.len(), 0, "a fresh project");
        assert_eq!(app.status(), "new project");
    }

    #[test]
    fn quitting_from_a_clean_project_closes_immediately() {
        let mut app = EditorApp::new(project());
        app.guard(Pending::Quit);
        assert!(app.quit, "a clean project closes without a prompt");
        assert!(app.force_close);
        assert_eq!(app.pending, None);
    }

    #[test]
    fn no_command_panics_on_a_project_whose_rungs_are_missing() {
        let mut project = Project::new("broken");
        let mut section = Section::new(1, "Main");
        section.rungs.push(404);
        project.sections.push(section);
        let mut app = EditorApp::new(project);

        assert_eq!(app.selected_rung, None);
        assert_eq!(app.selected_element(), None);
        app.delete_selection();
        app.toggle_vertical_link();
        app.apply_var();
        app.apply_params();
        app.nudge(1, 0);
        app.select_rung(404, Some((0, 0)));
        assert_eq!(app.selected_rung, Some(404));
        assert_eq!(app.selected_element(), None);
        assert!(app.var_error.is_none());

        // A missing rung id is tolerated by the UI and *reported* by the linter
        // (`SL-E011`) instead of being skipped silently, which would hide a
        // section that executes nothing.
        let problems = app.editor.problems();
        assert!(
            problems
                .iter()
                .any(|d| d.code == "SL-E011" && d.section == Some(0)),
            "the dangling rung reference is reported: {problems:?}"
        );
        assert_eq!(app.selection, Some((0, 0)));
    }

    #[test]
    fn problem_selection_switches_to_the_offending_rung_and_cell() {
        let mut app = EditorApp::new(project());
        app.place_at(3, 0);
        app.handle(Action::Pick(ElementKind::ContactNo));
        app.place_at(3, 0);
        // The same cell can only hold one element, so build the duplicate by
        // hand through the model, exactly like an imported project would.
        let duplicate = softladder_core::Diagnostic {
            severity: Severity::Error,
            code: "SL-E009",
            section: Some(0),
            rung: Some(0),
            message: "two elements are placed on cell (col 3, row 0)".to_owned(),
        };
        let target = queries::problem_target(app.project(), &duplicate).expect("resolvable");
        assert_eq!(target.rung, 1);
        assert_eq!(target.cell, Some((3, 0)));
        app.select_rung(target.rung, target.cell);
        assert_eq!(app.selection, Some((3, 0)));
    }

    // -- headless frame tests ----------------------------------------------

    /// Runs one frame of the real interface on a headless `egui::Context`.
    fn frame(
        app: &mut EditorApp,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            events,
            ..egui::RawInput::default()
        };
        ctx.run(input, |ctx| app.draw(ctx))
    }

    fn press(pos: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        }
    }

    #[test]
    fn drawing_every_panel_state_is_panic_free() {
        let ctx = egui::Context::default();
        let mut app = EditorApp::new(project());
        for tab in [
            RightTab::Bench,
            RightTab::Watch,
            RightTab::Problems,
            RightTab::Symbols,
        ] {
            app.right_tab = tab;
            frame(&mut app, &ctx, Vec::new());
        }
        app.show_about = true;
        app.show_shortcuts = true;
        app.show_right_panel = false;
        frame(&mut app, &ctx, Vec::new());
        app.show_right_panel = true;
        app.handle(Action::Pick(ElementKind::Timer {
            mode: softladder_core::TimerMode::On,
        }));
        app.selection = Some((layout::MAX_COL, layout::MAX_ROW));
        frame(&mut app, &ctx, Vec::new());
        app.handle(Action::Cancel);
        app.var_error = Some("invalid".to_owned());
        frame(&mut app, &ctx, Vec::new());
        app.pending = Some(Pending::Quit);
        frame(&mut app, &ctx, Vec::new());
        app.pending = None;
        // A running bench, so the live indication reads a real store.
        app.handle(Action::RunStop);
        app.bench_step();
        app.show_shortcuts = true;
        frame(&mut app, &ctx, Vec::new());
    }

    #[test]
    fn drawing_an_empty_or_broken_project_is_panic_free() {
        let ctx = egui::Context::default();
        let mut empty = EditorApp::new(Project::new("empty"));
        for tab in [
            RightTab::Bench,
            RightTab::Watch,
            RightTab::Problems,
            RightTab::Symbols,
        ] {
            empty.right_tab = tab;
            frame(&mut empty, &ctx, Vec::new());
        }

        let mut broken = Project::new("broken");
        let mut section = Section::new(1, "Main");
        section.rungs.push(404);
        broken.sections.push(section);
        broken.symbols.push(Symbol {
            name: "ghost".to_owned(),
            var: None,
            comment: String::new(),
            unit: None,
        });
        let mut app = EditorApp::new(broken);
        for tab in [
            RightTab::Bench,
            RightTab::Watch,
            RightTab::Problems,
            RightTab::Symbols,
        ] {
            app.right_tab = tab;
            frame(&mut app, &ctx, Vec::new());
        }
        app.select_rung(404, Some((0, 0)));
        frame(&mut app, &ctx, Vec::new());
    }

    #[test]
    fn a_drag_on_the_canvas_pans_without_touching_the_project() {
        let ctx = egui::Context::default();
        let mut app = EditorApp::new(project());
        frame(&mut app, &ctx, Vec::new());
        let start = egui::pos2(640.0, 400.0);
        let end = egui::pos2(700.0, 430.0);
        frame(
            &mut app,
            &ctx,
            vec![
                egui::Event::PointerMoved(start),
                egui::Event::PointerButton {
                    pos: start,
                    button: egui::PointerButton::Middle,
                    pressed: true,
                    modifiers: egui::Modifiers::default(),
                },
            ],
        );
        let middle = egui::pos2(670.0, 415.0);
        frame(&mut app, &ctx, vec![egui::Event::PointerMoved(middle)]);
        frame(&mut app, &ctx, vec![egui::Event::PointerMoved(end)]);
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerButton {
                pos: end,
                button: egui::PointerButton::Middle,
                pressed: false,
                modifiers: egui::Modifiers::default(),
            }],
        );
        assert!(
            app.camera.pan != egui::Vec2::ZERO,
            "the middle button panned the canvas"
        );
        assert_eq!(app.editor.history_len(), 0, "panning is not an edit");
    }

    #[test]
    fn a_click_on_the_canvas_places_the_armed_element() {
        let ctx = egui::Context::default();
        let mut app = EditorApp::new(project());
        frame(&mut app, &ctx, Vec::new());
        app.handle(Action::Pick(ElementKind::ContactNc));
        let pos = egui::pos2(640.0, 400.0);
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(pos), press(pos, true)],
        );
        frame(&mut app, &ctx, vec![press(pos, false)]);
        let cell = app.selection.expect("the click selected a cell");
        assert_ne!(cell, (0, 0), "the click landed away from the corner");
        let placed = app.element_at(1, cell.0, cell.1).expect("element placed");
        assert_eq!(placed.kind, ElementKind::ContactNc);
        assert_eq!(app.editor.history_len(), 1);
    }

    #[test]
    fn a_click_with_the_pointer_tool_only_selects() {
        let ctx = egui::Context::default();
        let mut app = EditorApp::new(project());
        app.handle(Action::Pick(ElementKind::ContactNo));
        app.place_at(4, 2);
        assert!(app.element_at(1, 4, 2).is_some());
        assert_eq!(app.tool, Tool::Select, "placing returns to the pointer");
        let history = app.editor.history_len();

        app.selection = None;
        app.load_properties();
        frame(&mut app, &ctx, Vec::new());
        let pos = egui::pos2(600.0, 380.0);
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(pos), press(pos, true)],
        );
        frame(&mut app, &ctx, vec![press(pos, false)]);

        assert!(app.selection.is_some(), "the click selected a cell");
        assert_eq!(
            app.editor.history_len(),
            history,
            "selecting is not an edit"
        );
    }

    #[test]
    fn clicking_an_element_selects_it_and_delete_removes_it() {
        // The pointer-level *drag* cannot be driven from a headless `Context`:
        // egui only reports `drag_started` for input it classifies as a real
        // drag, and a synthetic event sequence never is. The drag itself (hit
        // testing, the source element, the drop target) is covered in
        // `canvas.rs`, and the edit it performs is `Editor::move_element`, which
        // `softladder-edit` tests directly. This covers the rest of the pointer
        // path end to end: place, select, delete.
        let ctx = egui::Context::default();
        let mut app = EditorApp::new(project());
        frame(&mut app, &ctx, Vec::new());
        app.handle(Action::Pick(ElementKind::ContactNc));

        let first = egui::pos2(640.0, 400.0);
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(first), press(first, true)],
        );
        frame(&mut app, &ctx, vec![press(first, false)]);
        let placed = app.selection.expect("a cell was selected");
        assert_eq!(
            app.element_at(1, placed.0, placed.1).map(|e| e.kind),
            Some(ElementKind::ContactNc)
        );
        assert_eq!(app.tool, Tool::Select);

        // Clicking an empty cell moves the selection to that cell: the canvas
        // tracks the selected cell, not only the selected element.
        let elsewhere = egui::pos2(
            first.x + 4.0 * crate::canvas::COL_PITCH * app.camera.zoom,
            first.y,
        );
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(elsewhere), press(elsewhere, true)],
        );
        frame(&mut app, &ctx, vec![press(elsewhere, false)]);
        let moved_selection = app.selection.expect("a cell is selected");
        assert_ne!(moved_selection, placed, "the selection followed the click");
        assert_eq!(
            app.element_at(1, moved_selection.0, moved_selection.1),
            None,
            "the newly selected cell is empty"
        );

        // Selecting the element again and pressing Delete removes it.
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(first), press(first, true)],
        );
        frame(&mut app, &ctx, vec![press(first, false)]);
        assert_eq!(app.selection, Some(placed));
        app.handle(Action::Delete);
        assert_eq!(app.element_at(1, placed.0, placed.1), None);
        assert!(app.editor.can_undo(), "the delete is undoable");
    }

    #[test]
    fn keyboard_actions_reach_the_editor_through_a_real_frame() {
        let ctx = egui::Context::default();
        let mut app = EditorApp::new(project());
        app.handle(Action::Pick(ElementKind::ContactNo));
        app.place_at(5, 0);
        let history = app.editor.history_len();
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::Key {
                key: egui::Key::Z,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers {
                    command: true,
                    ..egui::Modifiers::default()
                },
            }],
        );
        assert_eq!(app.editor.history_len(), history - 1, "Ctrl+Z undid");
        assert_eq!(app.element_at(1, 5, 0), None);
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::Key {
                key: egui::Key::Y,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers {
                    command: true,
                    ..egui::Modifiers::default()
                },
            }],
        );
        assert!(app.element_at(1, 5, 0).is_some(), "Ctrl+Y redid");
    }
}
