//! The SFC (Grafcet) document: pages, steps, transitions and their wiring.
//!
//! `docs/UX.md` §12 is the spec this module implements. A sequential section is
//! drawn as one *document* in the middle of the window — the same sheet, border
//! and paper as the ladder, with a square and coarse grid on which a step or a
//! transition occupies one cell — and the centre document stays the program, so
//! one document has two languages rather than two editors.
//!
//! The shape of the drawing follows the vendors (TIA GRAPH, CODESYS SFC, Studio
//! 5000 SFC):
//!
//! * a page is a band with its comment in the header, exactly like a ladder
//!   network, and a chart that records elements on several pages draws one band
//!   per page;
//! * a **step** is a square with its number inside, doubled when it is the
//!   initial step, filled in `tokens.energised` with a white number and its
//!   `%X<n>.V` elapsed time in a chip while it is active;
//! * a **transition** is a short bar crossed by a hairline, with its condition
//!   written beside it (tag over address, the whole expression on hover) and
//!   emphasised while it is ready to fire;
//! * an **AND** junction — a transition with more than one source or more than
//!   one target — is drawn with the IEC double bar, an OR with the single one, so
//!   the two are distinguishable at a glance;
//! * **links** are never placed by hand: they are derived from the model's
//!   `from`/`to` sets and drawn as orthogonal wiring, so the picture cannot
//!   disagree with what the engine runs.
//!
//! Everything the document draws is in *document points* and is mapped to the
//! screen by one [`ViewTransform`], the same way the ladder maps a network, so a
//! pointer position still becomes a cell through a mapping the unit tests
//! exercise.
//!
//! # Where the view state lives
//!
//! The open page, the selection, the armed tool, a drag in progress and the
//! inspector's text buffers are **view state**: nothing about them belongs in the
//! project, and losing them costs the user nothing. They live in a thread-local
//! [`View`] inside this module — the pattern the project tree already uses for
//! its expansion state — exposed through [`view`], [`select`], [`focus`],
//! [`arm`], [`reset_view`], [`handle_key`], [`delete_selection`] and [`nudge`],
//! so the panels that jump into the document (the project tree, Problems) and the
//! tests can drive it without this module reaching into `EditorApp`. Moving it
//! into `EditorApp` later is a mechanical change: the fields are exactly the ones
//! listed on [`View`].
//!
//! The document also owns its keyboard map ([`handle_key`]), because `app`'s
//! canvas shortcuts are the ladder's — cells, elements, vertical links — and
//! every action that table maps is a no-op on an SFC section.

use egui::{
    Align2, Color32, CornerRadius, CursorIcon, FontFamily, FontId, Key, Modifiers, Painter, Pos2,
    Rect, RichText, Sense, Shape, Stroke, StrokeKind, Ui, Vec2,
};
use softladder_core::{
    Accessor, Diagnostic, Project, SectionLanguage, SequentialPage, Severity, Step, Transition,
    Value, VarKind, VarRef, VarStore,
};
use softladder_edit::Command;

use crate::app::{store, EditorApp};
use crate::design::{
    empty_state, quiet_pill, section_header, Tokens, TypeScale, RADIUS_CARD, RADIUS_CONTROL,
    RADIUS_PILL, SPACE_1, SPACE_2, SPACE_3,
};
use crate::layout::{self, Camera};
use crate::palette::{SfcEntry, SfcTool};
use crate::panels::problems::{self, SfcElement};
use crate::queries;
use crate::symbols;

/// Horizontal and vertical pitch of one grid cell, in document points.
///
/// Square and coarse, as `docs/UX.md` §12 asks: a step or a transition occupies
/// exactly one cell, the way the reference's 32 × 32 pages are laid out.
const CELL: f32 = 64.0;

/// Left offset of the wiring area inside the document, in document points.
///
/// One whole cell, so the sheet's own grid lines (which start at the document
/// origin) fall exactly on the cell boundaries.
const GRID_LEFT: f32 = CELL;

/// Blank space between a page header and its first row, in document points.
const GRID_TOP_PAD: f32 = SPACE_2;

/// Nominal width of a page band, in document points.
const DOC_WIDTH: f32 = 1024.0;

/// Shortest document that still reads as a page, in document points.
const DOC_MIN_HEIGHT: f32 = 220.0;

/// Rows a page offers even when it is empty.
const MIN_ROWS: i32 = 6;

/// Columns a page offers even when it is empty.
const MIN_COLS: i32 = 14;

/// Highest row a page addresses, matching the reference's 32 × 32 pages.
const MAX_ROWS: i32 = 32;

/// Highest column a page addresses.
const MAX_COLS: i32 = 32;

/// Padding between the paper edge and the document, in screen points.
const PAPER_PAD: f32 = SPACE_3;

/// Margin between the panel edge and the paper, in screen points.
const PAPER_MARGIN: f32 = SPACE_3;

/// Height of the caption row of a page header, in document points.
const HEADER_CAPTION_H: f32 = 16.0;

/// Height of the comment row of a page header, in document points.
const HEADER_TITLE_H: f32 = 19.0;

/// Height of one wrapped comment line, in document points.
const COMMENT_LINE_H: f32 = 15.0;

/// Vertical padding inside a page header, in document points.
const HEADER_PAD: f32 = 7.0;

/// Width reserved for the state badge in a page header, in document points.
const BADGE_W: f32 = 104.0;

/// Gap between two page bands, in document points.
const BAND_GAP: f32 = SPACE_2;

/// Tallest zoom the automatic fit applies: a document is fitted, never magnified.
const FIT_MAX_ZOOM: f32 = 1.0;

/// Smallest zoom the automatic fit applies; below this a chart stops being readable.
const FIT_MIN_ZOOM: f32 = 0.8;

/// Fraction of a cell the square of a step occupies.
const STEP_SCALE: f32 = 0.56;

/// Half-width of a transition bar, as a fraction of a cell.
const BAR_SCALE: f32 = 0.34;

/// Distance between the two bars of an AND junction, in document points.
const AND_GAP: f32 = 3.5;

/// Width of a sequential palette chip.
const CHIP_W: f32 = 44.0;

/// What the SFC document has selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Selection {
    /// A step, by number.
    Step(u32),
    /// A transition, by number.
    Transition(u32),
    /// A page, by number.
    Page(u32),
}

/// A drag of one SFC element, in grid cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Drag {
    /// The element being dragged.
    pub element: Selection,
    /// The cell the drag started on.
    pub from: (i32, i32),
    /// The cell the pointer is over.
    pub over: Option<(i32, i32)>,
}

/// The SFC document's view state.
///
/// It is view state, not project state. Every field is listed here so that it is
/// obvious what would move into `EditorApp` if the document's state is ever
/// lifted out of the thread-local.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct View {
    /// Id of the section this view belongs to; a different section resets it.
    pub section: Option<u32>,
    /// The page the document is framed on, when the user picked one.
    pub page: Option<u32>,
    /// What is selected, if anything.
    pub selection: Option<Selection>,
    /// The tool the ribbon has armed.
    pub tool: Option<SfcTool>,
    /// The condition field's text in the inspector.
    pub condition: String,
    /// The step number field's text in the inspector.
    pub number: String,
    /// The page comment field's text in the inspector.
    pub comment: String,
    /// The validation message of the last inspector commit that failed.
    pub error: Option<String>,
    /// A drag in progress.
    pub drag: Option<Drag>,
}

thread_local! {
    /// The view state of the SFC document, shared by the canvas, the ribbon, the
    /// inspector and the panels that jump into the document.
    static VIEW: std::cell::RefCell<View> = std::cell::RefCell::new(View::default());
}

/// `true` when the section at `index` is written in SFC.
///
/// This is the criterion the shell uses to decide which document — and which
/// Insert group — an SFC section gets.
pub fn is_sfc(project: &Project, index: usize) -> bool {
    project
        .sections
        .get(index)
        .is_some_and(|section| section.language == SectionLanguage::Sfc)
}

/// The current view state of the SFC document.
pub fn view() -> View {
    VIEW.with(|view| view.borrow().clone())
}

/// Forgets the SFC view state.
///
/// The document resets itself when the user opens another section, and the
/// screenshot harness calls this between projects, so one document's selection
/// can never leak into another's.
pub fn reset_view() {
    VIEW.with(|view| *view.borrow_mut() = View::default());
}

/// Replaces the whole view state.
fn set_view(view: View) {
    VIEW.with(|slot| *slot.borrow_mut() = view);
}

/// The view of the selected section, reset when the document switched sections.
fn view_of(app: &EditorApp) -> View {
    let id = app
        .project()
        .sections
        .get(app.selected_section)
        .map(|section| section.id);
    VIEW.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.section != id {
            *slot = View {
                section: id,
                ..View::default()
            };
        }
        slot.clone()
    })
}

/// Selects an element, loading the inspector's buffers from the model.
pub fn select(app: &mut EditorApp, selection: Option<Selection>) {
    let mut state = view_of(app);
    state.selection = selection;
    state.error = None;
    if let Some(Selection::Page(number)) = selection {
        state.page = Some(number);
    }
    load_buffers(app, &mut state);
    set_view(state);
}

/// Opens a page of the SFC document and selects `selection` on it.
///
/// This is what the project tree and the Problems document call: one gesture,
/// from wherever the user clicked, to the chart with the offending element
/// selected — exactly like the ladder's "select the offending network and cell".
pub fn focus(app: &mut EditorApp, page: u32, selection: Option<Selection>) {
    let mut state = view_of(app);
    state.page = Some(page);
    state.selection = selection;
    state.tool = None;
    state.error = None;
    load_buffers(app, &mut state);
    set_view(state);
}

/// Opens the section at `index` in the sequential document.
///
/// This is the entry point for a caller outside the shell — the screenshot
/// harness, a script, a test — that has to bring a chart to the front: it makes
/// the section the open one, clears the ladder's selection and frames the
/// section's page.
pub fn open(app: &mut EditorApp, index: usize) {
    app.selected_section = index;
    app.selected_rung = None;
    app.selection = None;
    reset_view();
    let page = app
        .project()
        .sections
        .get(index)
        .and_then(|section| section.sequential_page.as_ref())
        .map(|page| page.number);
    if let Some(page) = page {
        focus(app, page, None);
    }
}

/// Arms the ribbon with a sequential tool, or disarms it with `None`.
pub fn arm(app: &mut EditorApp, tool: Option<SfcTool>) {
    let mut state = view_of(app);
    state.tool = tool;
    state.error = None;
    set_view(state);
    app.note(&match tool {
        Some(tool) => format!("place {}", crate::palette::sfc_short_name(tool)),
        None => "select".to_owned(),
    });
}

/// Refills the inspector's buffers from the selected element.
fn load_buffers(app: &EditorApp, state: &mut View) {
    let Ok(page) = open_page(app) else {
        state.condition.clear();
        state.number.clear();
        state.comment.clear();
        return;
    };
    state.comment = page.comment.clone();
    state.condition = match state.selection {
        Some(Selection::Transition(number)) => page
            .transition(number)
            .and_then(|transition| transition.condition.as_ref())
            .map(ToString::to_string)
            .unwrap_or_default(),
        _ => String::new(),
    };
    state.number = match state.selection {
        Some(Selection::Step(number)) => number.to_string(),
        _ => String::new(),
    };
}

/// The page of the open section, or why there is none.
fn open_page(app: &EditorApp) -> Result<SequentialPage, &'static str> {
    app.project()
        .sections
        .get(app.selected_section)
        .ok_or("no section is open")?
        .sequential_page
        .clone()
        .ok_or("this section has no page")
}

/// The id of the open section, when it exists.
fn section_id(app: &EditorApp) -> Option<u32> {
    app.project()
        .sections
        .get(app.selected_section)
        .map(|section| section.id)
}

/// Draws the SFC document and turns pointer input into editor calls.
pub fn show(app: &mut EditorApp, ui: &mut Ui) {
    let mut state = view_of(app);
    document(app, ui, &mut state);
    set_view(state);
}

/// Draws one frame of the document with `state` as its view state.
fn document(app: &mut EditorApp, ui: &mut Ui, state: &mut View) {
    let (response, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
    let panel = response.rect;
    let tokens = app.tokens;

    painter.rect_filled(panel, CornerRadius::ZERO, tokens.surface);
    let paper = paper_rect(panel);
    paint_paper(&painter, paper, &tokens);

    // The keyboard is applied once per frame, before anything is read, so an
    // armed tool or a deleted element is visible in the frame it happened in.
    if !ui.ctx().wants_keyboard_input() {
        poll_keys(app, ui);
        *state = view_of(app);
    }

    // The empty states come first: with no page there is no camera to fit and no
    // cell to hit, and a blank sheet is not an interface.
    let Some(section) = app.project().sections.get(app.selected_section) else {
        empty_document(
            app,
            ui,
            &painter,
            paper,
            &tokens,
            "No program yet",
            "Create a section in the project tree, or open a project.",
            "",
            false,
        );
        return;
    };
    let Some(page) = section.sequential_page.clone() else {
        empty_document(
            app,
            ui,
            &painter,
            paper,
            &tokens,
            "This SFC section has no page",
            "A sequential chart lives on a page: its comment, its steps and its transitions.",
            "Insert / Initial step gives the section a page to draw on.",
            true,
        );
        return;
    };

    // A selection that has disappeared (deleted, or undone away) cannot stay.
    if let Some(Selection::Step(number)) = state.selection {
        if page.step(number).is_none() {
            state.selection = None;
        }
    }
    if let Some(Selection::Transition(number)) = state.selection {
        if page.transition(number).is_none() {
            state.selection = None;
        }
    }

    let problems = app.editor.problems().to_vec();
    let section_index = app.selected_section;
    let bands = read_bands(&page, &painter, &tokens);
    if bands.is_empty() {
        return;
    }

    // The document is fitted whenever the open page changes, so a chart never
    // opens at a random zoom with its steps lost in a corner.
    let fit_id = egui::Id::new("softladder-sfc-fit");
    let key: (usize, u32, usize, i32) = (
        section_index,
        page.number,
        bands
            .iter()
            .map(|band| band.steps.len() + band.transitions.len())
            .sum(),
        page.comment.len() as i32,
    );
    let fitted: Option<(usize, u32, usize, i32)> = ui.ctx().data(|data| data.get_temp(fit_id));
    if fitted.as_ref() != Some(&key) || app.camera.is_default() {
        let base = paper.min + Vec2::splat(PAPER_PAD);
        let viewport = paper.shrink(PAPER_PAD);
        let focus = state
            .page
            .and_then(|number| bands.iter().find(|band| band.number == number))
            .map(Band::band);
        app.camera = fit_camera(document_bounds(&bands), viewport, base, focus);
        ui.ctx().data_mut(|data| data.insert_temp(fit_id, key));
    }

    let space = ui.input(|input| input.key_down(egui::Key::Space));
    handle_zoom(app, ui, &response, paper);
    handle_pan(app, &response, space);
    let transform = ViewTransform::new(paper, app.camera);

    let live = app.bench().state().is_scanning() || app.bench().cycles() > 0;
    let hover_pos = response.hover_pos();
    let target = hover_pos.and_then(|pos| hit(&bands, transform, pos));
    let hovered = target.and_then(|target| element_at(&bands, target));

    let frame = Frame {
        app,
        problems: &problems,
        section: section_index,
        live,
        target,
        selected: state.selection,
        tool: state.tool,
    };
    let clipping = painter.with_clip_rect(paper.shrink(1.0));
    for (position, band) in bands.iter().enumerate() {
        paint_band(&clipping, transform, band, position, &tokens, &frame);
    }
    if let Some(target) = target {
        paint_ghost(&clipping, transform, &bands, target, state.tool, &tokens);
    }

    handle_pointer(app, ui, &response, &bands, &page, transform, state, space);
    handle_menu(app, ui, &response, &bands, transform, state);
    expression_tooltip(&response, &page, hovered);
}

/// Applies this frame's key events through [`handle_key`].
fn poll_keys(app: &mut EditorApp, ui: &Ui) {
    let events: Vec<(Key, Modifiers)> = ui.input(|input| {
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
        handle_key(app, key, modifiers);
    }
}

/// The paper rectangle: the panel inset by a margin, which is where the document
/// lives and everything outside it is the surrounding surface.
fn paper_rect(panel: Rect) -> Rect {
    let inset = Vec2::splat(PAPER_MARGIN);
    Rect::from_min_max(
        panel.min + inset,
        (panel.max - inset).max(panel.min + inset),
    )
}

/// Fills the paper sheet and outlines it with a hairline.
fn paint_paper(painter: &Painter, paper: Rect, tokens: &Tokens) {
    painter.rect(
        paper,
        CornerRadius::same(RADIUS_CARD),
        tokens.paper,
        Stroke::new(1.0_f32, tokens.border),
        StrokeKind::Inside,
    );
}

/// One page band of the document, in document points.
struct Band {
    /// Page number this band draws.
    number: u32,
    /// The page's comment, empty for a page the section does not own.
    comment: String,
    /// Steps drawn on this page, in canonical order.
    steps: Vec<Step>,
    /// Transitions drawn on this page, in canonical order.
    transitions: Vec<Transition>,
    /// Top of the band, in document points.
    top: f32,
    /// Height of the band, in document points.
    height: f32,
    /// Grid rows the band offers.
    rows: i32,
    /// Grid columns the band offers.
    cols: i32,
    /// Number of wrapped comment lines.
    comment_lines: usize,
}

impl Band {
    /// Bottom of the band, in document points.
    fn bottom(&self) -> f32 {
        self.top + self.height
    }

    /// The band as a document rectangle.
    fn band(&self) -> Rect {
        Rect::from_min_size(Pos2::new(0.0, self.top), Vec2::new(DOC_WIDTH, self.height))
    }

    /// Height of the header, in document points.
    fn header_height(&self) -> f32 {
        header_height(!self.comment.trim().is_empty(), self.comment_lines)
    }

    /// The header as a document rectangle.
    fn header(&self) -> Rect {
        let band = self.band();
        Rect::from_min_size(band.min, Vec2::new(band.width(), self.header_height()))
    }

    /// Top of the grid, in document points.
    fn grid_top(&self) -> f32 {
        self.top + self.header_height() + GRID_TOP_PAD
    }

    /// The cell at `(x, y)` as a document rectangle.
    fn cell(&self, x: i32, y: i32) -> Rect {
        Rect::from_min_size(
            Pos2::new(
                GRID_LEFT + x as f32 * CELL,
                self.grid_top() + y as f32 * CELL,
            ),
            Vec2::splat(CELL),
        )
    }
}

/// Height of a page header with `comment_lines` wrapped comment lines.
fn header_height(has_comment: bool, comment_lines: usize) -> f32 {
    let comment = if has_comment { HEADER_TITLE_H } else { 0.0 };
    HEADER_PAD * 2.0 + HEADER_CAPTION_H + comment + comment_lines as f32 * COMMENT_LINE_H
}

/// The rectangles of one page header.
#[derive(Debug, Clone, Copy, PartialEq)]
struct HeaderRects {
    /// `Page <n>` in the caption style.
    caption: Rect,
    /// The wrapped page comment.
    comment: Rect,
    /// The state badge, right-aligned in the caption row.
    badge: Rect,
}

/// Splits a page header into the rectangles its text is drawn in.
fn header_rects(header: Rect, comment_lines: usize) -> HeaderRects {
    let left = header.left() + SPACE_3;
    let right = header.right() - SPACE_3;
    let top = header.top() + HEADER_PAD;
    let badge = Rect::from_min_size(
        Pos2::new(right - BADGE_W, top),
        Vec2::new(BADGE_W, HEADER_CAPTION_H),
    );
    let caption = Rect::from_min_max(
        Pos2::new(left, top),
        Pos2::new((badge.left() - SPACE_2).max(left), top + HEADER_CAPTION_H),
    );
    let comment = Rect::from_min_size(
        Pos2::new(left, top + HEADER_CAPTION_H),
        Vec2::new(
            (right - left).max(0.0),
            comment_lines as f32 * COMMENT_LINE_H,
        ),
    );
    HeaderRects {
        caption,
        comment,
        badge,
    }
}

/// Reads every page of `page` into the bands it is drawn in.
///
/// The model gives a section one page holding steps and transitions that each
/// record their own page number, so the bands are the distinct page numbers the
/// chart actually uses: a chart authored page by page draws page by page, and a
/// chart that records an element on a page its section does not own still shows
/// it, on its own band, which is where `SL-E011` points at it.
fn read_bands(page: &SequentialPage, painter: &Painter, tokens: &Tokens) -> Vec<Band> {
    let mut numbers: Vec<u32> = vec![page.number];
    for step in &page.steps {
        if !numbers.contains(&step.page) {
            numbers.push(step.page);
        }
    }
    for transition in &page.transitions {
        if !numbers.contains(&transition.page) {
            numbers.push(transition.page);
        }
    }
    numbers.sort_unstable();

    let comment_font = FontId::new(TypeScale::CAPTION, FontFamily::Proportional);
    let wrap = (DOC_WIDTH - SPACE_3 * 2.0).max(80.0);
    let mut bands = Vec::with_capacity(numbers.len());
    let mut top = 0.0;
    for number in &numbers {
        let steps: Vec<Step> = page
            .steps
            .iter()
            .filter(|step| step.page == *number)
            .cloned()
            .collect();
        let transitions: Vec<Transition> = page
            .transitions
            .iter()
            .filter(|transition| transition.page == *number)
            .cloned()
            .collect();
        let comment = if *number == page.number {
            page.comment.clone()
        } else {
            String::new()
        };
        let comment_lines = if comment.trim().is_empty() {
            0
        } else {
            painter
                .layout(comment.clone(), comment_font.clone(), tokens.text_dim, wrap)
                .rows
                .len()
                .clamp(1, 6)
        };
        let rows = (steps
            .iter()
            .map(|step| step.y)
            .chain(transitions.iter().map(|transition| transition.y))
            .max()
            .map_or(MIN_ROWS - 1, |row| row))
        .saturating_add(1)
        .clamp(MIN_ROWS, MAX_ROWS);
        let cols = (steps
            .iter()
            .map(|step| step.x)
            .chain(transitions.iter().map(|transition| transition.x))
            .max()
            .map_or(MIN_COLS - 1, |col| col))
        .saturating_add(1)
        .clamp(MIN_COLS, MAX_COLS);
        let height = header_height(!comment.trim().is_empty(), comment_lines)
            + GRID_TOP_PAD
            + rows as f32 * CELL
            + SPACE_2;
        bands.push(Band {
            number: *number,
            comment,
            steps,
            transitions,
            top,
            height,
            rows,
            cols,
            comment_lines,
        });
        top += height + BAND_GAP;
    }
    bands
}

/// The bounding box of every band, in document points.
fn document_bounds(bands: &[Band]) -> Rect {
    let height = bands
        .last()
        .map_or(DOC_MIN_HEIGHT, |band| band.bottom())
        .max(DOC_MIN_HEIGHT);
    Rect::from_min_size(Pos2::ZERO, Vec2::new(DOC_WIDTH, height))
}

/// The camera that frames a chart in `viewport`, with the document origin at `base`.
///
/// The rule the ladder uses: a chart that fits is shown whole, centred, at `1.0`
/// at most, and a chart taller than the window is framed on the open page at the
/// zoom below which a chart stops being readable.
fn fit_camera(content: Rect, viewport: Rect, base: Pos2, focus: Option<Rect>) -> Camera {
    if !content.is_positive()
        || !viewport.is_positive()
        || !content.width().is_finite()
        || !content.height().is_finite()
    {
        return Camera::default();
    }
    let fitted = (viewport.width() / content.width())
        .min(viewport.height() / content.height())
        .min(FIT_MAX_ZOOM);
    let zoom = layout::clamp_zoom(fitted.max(FIT_MIN_ZOOM));
    let half = viewport.height() / (2.0 * zoom);
    let centre_y = if content.height() <= 2.0 * half {
        content.top() + half
    } else {
        let wanted = focus.map_or(content.center().y, |rect| rect.center().y);
        wanted.clamp(content.top() + half, content.bottom() - half)
    };
    let pan = Vec2::new(
        viewport.left() - base.x - content.left() * zoom,
        viewport.center().y - base.y - centre_y * zoom,
    );
    Camera { zoom, pan }
}

/// The screen mapping of the document: a pan/zoom camera over document points.
#[derive(Debug, Clone, Copy)]
struct ViewTransform {
    /// Screen position of document point `(0, 0)`, the pan included.
    origin: Pos2,
    /// Zoom factor.
    zoom: f32,
}

impl ViewTransform {
    /// The view of `paper` under `camera`.
    fn new(paper: Rect, camera: Camera) -> Self {
        Self {
            origin: paper.min + Vec2::splat(PAPER_PAD) + camera.pan,
            zoom: camera.zoom,
        }
    }

    /// A document point on screen.
    fn point(&self, x: f32, y: f32) -> Pos2 {
        self.origin + Vec2::new(x, y) * self.zoom
    }

    /// A document rectangle on screen.
    fn rect(&self, rect: Rect) -> Rect {
        Rect::from_min_max(
            self.point(rect.min.x, rect.min.y),
            self.point(rect.max.x, rect.max.y),
        )
    }

    /// A font from the type scale, scaled by the zoom and never below 6 points.
    fn font(&self, size: f32, family: FontFamily) -> FontId {
        FontId::new((size * self.zoom).max(6.0), family)
    }

    /// The screen rectangle of cell `(x, y)` of `band`.
    fn cell(&self, band: &Band, x: i32, y: i32) -> Rect {
        self.rect(band.cell(x, y))
    }

    /// The cell of `band` under `pos`, if any.
    fn cell_at(&self, band: &Band, pos: Pos2) -> Option<(i32, i32)> {
        let size = CELL * self.zoom;
        if size <= 0.0 || !size.is_finite() {
            return None;
        }
        let origin = self.point(GRID_LEFT, band.grid_top());
        let x = ((pos.x - origin.x) / size).floor();
        let y = ((pos.y - origin.y) / size).floor();
        if !x.is_finite() || !y.is_finite() || x < 0.0 || y < 0.0 {
            return None;
        }
        let (x, y) = (x as i32, y as i32);
        if x >= band.cols || y >= band.rows {
            return None;
        }
        Some((x, y))
    }
}

/// What the pointer is over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    /// A cell of a band: band index, x, y.
    Cell {
        /// Index of the band in the document.
        band: usize,
        /// Column of the cell.
        x: i32,
        /// Row of the cell.
        y: i32,
    },
    /// A band itself: its header, or the sheet around its grid.
    Band {
        /// Index of the band in the document.
        band: usize,
    },
}

/// The target under `pos`, if any.
fn hit(bands: &[Band], view: ViewTransform, pos: Pos2) -> Option<Target> {
    for (index, band) in bands.iter().enumerate() {
        if let Some((x, y)) = view.cell_at(band, pos) {
            return Some(Target::Cell { band: index, x, y });
        }
    }
    bands
        .iter()
        .position(|band| view.rect(band.band()).contains(pos))
        .map(|band| Target::Band { band })
}

/// The element a target names, if it holds one.
fn element_at(bands: &[Band], target: Target) -> Option<Selection> {
    match target {
        Target::Cell { band, x, y } => {
            let band = bands.get(band)?;
            if let Some(step) = band.steps.iter().find(|step| step.x == x && step.y == y) {
                return Some(Selection::Step(step.number));
            }
            band.transitions
                .iter()
                .find(|transition| transition.x == x && transition.y == y)
                .map(|transition| Selection::Transition(transition.number))
        }
        Target::Band { band } => bands.get(band).map(|band| Selection::Page(band.number)),
    }
}

/// Read-only state shared by every band of one frame.
struct Frame<'a> {
    /// The editor the symbols and the live values come from.
    app: &'a EditorApp,
    /// Diagnostics of the whole project.
    problems: &'a [Diagnostic],
    /// Index of the open section.
    section: usize,
    /// `true` while the engine has a live picture to show.
    live: bool,
    /// What the pointer is over.
    target: Option<Target>,
    /// What is selected.
    selected: Option<Selection>,
    /// The armed tool.
    tool: Option<SfcTool>,
}

impl Frame<'_> {
    /// The running engine's variable store.
    fn store(&self) -> &VarStore {
        store(self.app)
    }

    /// `true` when the step `number` is active in the store.
    fn step_active(&self, number: u32) -> bool {
        self.live && step_active(self.store(), number)
    }

    /// `true` when every source step of `transition` is active.
    fn sources_active(&self, transition: &Transition) -> bool {
        transition
            .from
            .iter()
            .all(|number| self.step_active(*number))
    }

    /// `true` when `transition` is ready to fire.
    ///
    /// The engine fires a transition when its condition holds *and* all of its
    /// source steps are active, so that is exactly what the drawing emphasises:
    /// an unconditional transition is not lit while its source is idle.
    fn transition_true(&self, transition: &Transition) -> bool {
        if !self.live {
            return false;
        }
        let holds = transition
            .condition
            .as_ref()
            .map(|condition| {
                softladder_core::eval(condition, self.store())
                    .map(Value::as_bool)
                    .unwrap_or(false)
            })
            .unwrap_or(true);
        holds && self.sources_active(transition)
    }
}

/// The state of one page's badge.
struct Badge {
    /// Text of the badge.
    text: String,
    /// Colour of the badge.
    colour: Color32,
}

/// The badge of a page: errors first, then warnings, then `ok`.
fn page_badge(
    tokens: &Tokens,
    project: &Project,
    problems: &[Diagnostic],
    section: usize,
    page: u32,
) -> Badge {
    let mut errors = 0usize;
    let mut warnings = 0usize;
    for diagnostic in problems {
        let Some(target) = problems::sfc_target(project, diagnostic) else {
            continue;
        };
        if target.section != section || target.page != Some(page) {
            continue;
        }
        if diagnostic.severity == Severity::Error {
            errors += 1;
        } else {
            warnings += 1;
        }
    }
    if errors > 0 {
        Badge {
            text: if errors == 1 {
                "error".to_owned()
            } else {
                format!("{errors} errors")
            },
            colour: tokens.error,
        }
    } else if warnings > 0 {
        Badge {
            text: format!("{warnings} warning{}", if warnings == 1 { "" } else { "s" }),
            colour: tokens.warning,
        }
    } else {
        Badge {
            text: "ok".to_owned(),
            colour: tokens.run,
        }
    }
}

/// Draws one band: its header and its wiring area.
fn paint_band(
    painter: &Painter,
    view: ViewTransform,
    band: &Band,
    position: usize,
    tokens: &Tokens,
    frame: &Frame<'_>,
) {
    let band_rect = view.rect(band.band());
    let header = view.rect(band.header());
    let badge = page_badge(
        tokens,
        frame.app.project(),
        frame.problems,
        frame.section,
        band.number,
    );
    let selected_page = frame.selected == Some(Selection::Page(band.number));
    let hovered = matches!(frame.target, Some(Target::Band { band: index }) if index == position);

    if selected_page {
        painter.rect_filled(
            header,
            CornerRadius::same(RADIUS_CONTROL),
            tokens.accent_soft,
        );
    } else if hovered {
        painter.rect_filled(
            header,
            CornerRadius::same(RADIUS_CONTROL),
            tokens.accent.gamma_multiply(0.06),
        );
    }

    let rects = header_rects(header, band.comment_lines);
    painter.text(
        rects.caption.left_center(),
        Align2::LEFT_CENTER,
        format!("Page {}", band.number),
        view.font(TypeScale::CAPTION, FontFamily::Proportional),
        tokens.text_dim,
    );
    paint_badge(painter, view, rects.badge, &badge);
    if band.comment_lines > 0 && rects.comment.is_positive() {
        let galley = painter.layout(
            band.comment.clone(),
            view.font(TypeScale::CAPTION, FontFamily::Proportional),
            tokens.text_dim,
            rects.comment.width(),
        );
        painter.galley(rects.comment.left_top(), galley, tokens.text_dim);
    }
    painter.hline(band_rect.x_range(), band_rect.bottom(), tokens.hairline());

    let bar = Rect::from_min_size(band_rect.min, Vec2::new(3.0, band_rect.height()));
    if selected_page {
        painter.rect_filled(bar, CornerRadius::ZERO, tokens.accent);
    } else if badge.colour == tokens.error {
        painter.rect_filled(bar, CornerRadius::ZERO, tokens.error);
    }

    paint_grid(painter, view, band, tokens);
    paint_links(painter, view, band, tokens, frame);
    paint_transitions(painter, view, band, tokens, frame);
    paint_steps(painter, view, band, tokens, frame);
    paint_problem_marks(painter, view, band, tokens, frame);
    if band.steps.is_empty() && band.transitions.is_empty() {
        paint_empty_page(painter, view, band, tokens, frame);
    }
}

/// Draws the square grid of one band, aligned with its own cells.
fn paint_grid(painter: &Painter, view: ViewTransform, band: &Band, tokens: &Tokens) {
    let size = CELL * view.zoom;
    if size < 6.0 || !size.is_finite() {
        return;
    }
    let stroke = Stroke::new(1.0_f32, tokens.paper_grid);
    let top = view.point(0.0, band.grid_top()).y;
    let bottom = view.point(0.0, band.grid_top() + band.rows as f32 * CELL).y;
    let left = view.point(GRID_LEFT, 0.0).x;
    let right = view.point(GRID_LEFT + band.cols as f32 * CELL, 0.0).x;
    if !top.is_finite() || !bottom.is_finite() || !left.is_finite() || !right.is_finite() {
        return;
    }
    for column in 0..=band.cols {
        let x = view.point(GRID_LEFT + column as f32 * CELL, 0.0).x;
        painter.vline(x, top..=bottom, stroke);
    }
    for row in 0..=band.rows {
        let y = view.point(0.0, band.grid_top() + row as f32 * CELL).y;
        painter.hline(left..=right, y, stroke);
    }
}

/// Draws the wiring of one band, derived from the model's `from`/`to` sets.
fn paint_links(
    painter: &Painter,
    view: ViewTransform,
    band: &Band,
    tokens: &Tokens,
    frame: &Frame<'_>,
) {
    for transition in &band.transitions {
        let Some(bar) = transition_rect(view, band, transition) else {
            continue;
        };
        let ready = frame.transition_true(transition);
        for number in transition.from.iter().chain(transition.to.iter()) {
            let Some(step) = band.steps.iter().find(|step| step.number == *number) else {
                continue;
            };
            let square = step_rect(view.cell(band, step.x, step.y), view.zoom);
            let live = if transition.from.contains(number) {
                frame.step_active(*number)
            } else {
                ready
            };
            paint_path(painter, &link_path(square, bar), tokens, live);
        }
    }
}

/// Draws one orthogonally routed wire.
fn paint_path(painter: &Painter, path: &[Pos2], tokens: &Tokens, live: bool) {
    if path.len() < 2 {
        return;
    }
    let style = symbols::Style::idle(tokens).with_live(tokens, live);
    for pair in path.windows(2) {
        if let [from, to] = pair {
            painter.line_segment([*from, *to], style.wire);
        }
    }
}

/// Draws the steps of one band.
fn paint_steps(
    painter: &Painter,
    view: ViewTransform,
    band: &Band,
    tokens: &Tokens,
    frame: &Frame<'_>,
) {
    for step in &band.steps {
        let cell = view.cell(band, step.x, step.y);
        let rect = step_rect(cell, view.zoom);
        if !rect.is_positive() {
            continue;
        }
        let active = frame.step_active(step.number);
        paint_step(painter, view, rect, step, tokens, active);
        let scale = view.zoom.max(0.5);
        // The initial step is marked with a chip, the way GRAPH writes `Init`.
        if step.is_initial {
            let anchor = Pos2::new(rect.center().x, rect.top() - 10.0 * scale);
            symbols::value_chip(painter, anchor, tokens, "init", false);
        }
        // The elapsed time of an active step, in place.
        if active {
            if let Some(value) = step_time(frame.store(), step.number) {
                let anchor = Pos2::new(rect.center().x, rect.bottom() + 12.0 * scale);
                // An opaque plate under the chip: the wire that feeds the next
                // transition runs down this column, and a readout is a label, not
                // another translucent tint over the drawing.
                let galley = painter.layout_no_wrap(
                    value.clone(),
                    FontId::new(TypeScale::CAPTION, FontFamily::Monospace),
                    tokens.energised,
                );
                painter.rect_filled(
                    Rect::from_center_size(anchor, galley.size() + Vec2::new(6.0, 2.0)),
                    CornerRadius::same(RADIUS_PILL),
                    tokens.paper,
                );
                symbols::value_chip(painter, anchor, tokens, &value, true);
            }
        }
        outline(
            painter,
            view,
            rect,
            frame.selected == Some(Selection::Step(step.number)),
            tokens,
        );
    }
}

/// Draws one step: a square with its number, doubled when it is initial.
fn paint_step(
    painter: &Painter,
    view: ViewTransform,
    rect: Rect,
    step: &Step,
    tokens: &Tokens,
    active: bool,
) {
    let style = symbols::Style::idle(tokens).with_live(tokens, active);
    let fill = if active {
        tokens.energised
    } else {
        tokens.paper
    };
    painter.rect(
        rect,
        CornerRadius::same(RADIUS_PILL),
        fill,
        style.symbol,
        StrokeKind::Middle,
    );
    if step.is_initial {
        // The doubled border of the initial step: a second outline just inside.
        let inner = rect.shrink(3.0 * view.zoom.max(0.4));
        if inner.is_positive() {
            painter.rect_stroke(
                inner,
                CornerRadius::same(RADIUS_PILL),
                style.symbol,
                StrokeKind::Middle,
            );
        }
    }
    let colour = if active { Color32::WHITE } else { tokens.text };
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        step.number.to_string(),
        FontId::new(
            (rect.height() * 0.42).clamp(7.0, 18.0),
            FontFamily::Proportional,
        ),
        colour,
    );
}

/// Draws the transitions of one band.
fn paint_transitions(
    painter: &Painter,
    view: ViewTransform,
    band: &Band,
    tokens: &Tokens,
    frame: &Frame<'_>,
) {
    for transition in &band.transitions {
        let Some(bar) = transition_rect(view, band, transition) else {
            continue;
        };
        let ready = frame.transition_true(transition);
        let style = symbols::Style::idle(tokens).with_live(tokens, ready);
        let centre = bar.center();
        let scale = view.zoom.max(0.5);
        for offset in bar_offsets(is_and(transition)) {
            let y = centre.y + offset * scale;
            painter.hline(
                (centre.x - bar.width() / 2.0)..=(centre.x + bar.width() / 2.0),
                y,
                style.symbol,
            );
        }
        // The hairline crossing the bar: the IEC mark of a transition.
        let half = (CELL * view.zoom * 0.16).max(2.0);
        painter.vline(
            centre.x,
            (centre.y - half)..=(centre.y + half),
            style.symbol,
        );
        paint_condition(painter, view, bar, transition, tokens, frame);
        outline(
            painter,
            view,
            Rect::from_center_size(
                centre,
                Vec2::new(bar.width(), (CELL * view.zoom * 0.4).max(bar.height())),
            ),
            frame.selected == Some(Selection::Transition(transition.number)),
            tokens,
        );
    }
}

/// Draws the condition of a transition beside its bar.
fn paint_condition(
    painter: &Painter,
    view: ViewTransform,
    bar: Rect,
    transition: &Transition,
    tokens: &Tokens,
    frame: &Frame<'_>,
) {
    let (name, text) = condition_label(frame.app.project(), transition);
    let scale = view.zoom.max(0.5);
    let left = bar.right() + 6.0 * scale;
    let colour = if transition.condition.is_none() {
        tokens.text_dim
    } else {
        tokens.text
    };
    match name {
        Some(name) => {
            painter.text(
                Pos2::new(left, bar.center().y - 7.0 * scale),
                Align2::LEFT_CENTER,
                name,
                view.font(TypeScale::CAPTION, FontFamily::Proportional),
                colour,
            );
            painter.text(
                Pos2::new(left, bar.center().y + 7.0 * scale),
                Align2::LEFT_CENTER,
                text,
                view.font(TypeScale::CAPTION, FontFamily::Monospace),
                tokens.text_dim,
            );
        }
        None => {
            painter.text(
                Pos2::new(left, bar.center().y),
                Align2::LEFT_CENTER,
                text,
                view.font(TypeScale::CAPTION, FontFamily::Monospace),
                colour,
            );
        }
    }
}

/// The tag name and text drawn for a transition's condition.
///
/// A condition that is a single variable reads like a ladder element: its tag
/// name over its address. Anything else is shown as the expression itself, in
/// monospace; an unconditional transition is spelled out, because "fires
/// whenever its sources are active" is not obvious from an empty bar.
fn condition_label(project: &Project, transition: &Transition) -> (Option<String>, String) {
    match transition.condition.as_ref() {
        None => (None, "always".to_owned()),
        Some(softladder_core::Expr::Var(var)) => {
            let name = queries::symbol_for(project, var).map(|symbol| symbol.name.clone());
            (name, var.to_string())
        }
        Some(other) => (None, other.to_string()),
    }
}

/// `true` when a transition is an AND junction: several sources or several targets.
///
/// This is where the vendors' drawings earn their keep: a transition that
/// requires several steps, or activates several, is an AND, and the IEC notation
/// writes it with a double bar, so the two junctions are distinguishable at a
/// glance rather than by reading the sets.
fn is_and(transition: &Transition) -> bool {
    transition.from.len() > 1 || transition.to.len() > 1
}

/// The junction a transition forms with the rest of the page, if any.
///
/// A transition with several sources or several targets is an **AND** junction.
/// One that shares a source or a target with another transition is an **OR**
/// junction: either branch can fire, or either branch can activate the shared
/// step. A plain transition in a sequence is neither, and the inspector says so
/// by leaving the badge off rather than mislabelling it.
fn junction_of(page: &SequentialPage, transition: &Transition) -> Option<&'static str> {
    if is_and(transition) {
        return Some("AND");
    }
    let shares = page.transitions.iter().any(|other| {
        other.number != transition.number
            && (other
                .from
                .iter()
                .any(|number| transition.from.contains(number))
                || other.to.iter().any(|number| transition.to.contains(number)))
    });
    shares.then_some("OR")
}

/// The vertical offsets of the bars of a transition, in document points.
fn bar_offsets(and: bool) -> &'static [f32] {
    if and {
        &[-AND_GAP, AND_GAP]
    } else {
        &[0.0]
    }
}

/// The screen rectangle of a transition's bar.
fn transition_rect(view: ViewTransform, band: &Band, transition: &Transition) -> Option<Rect> {
    let cell = view.cell(band, transition.x, transition.y);
    if !cell.is_positive() {
        return None;
    }
    Some(Rect::from_center_size(
        cell.center(),
        Vec2::new(
            cell.width() * BAR_SCALE * 2.0,
            (CELL * view.zoom * 0.12).max(1.0),
        ),
    ))
}

/// The square of a step inside its cell.
fn step_rect(cell: Rect, zoom: f32) -> Rect {
    // `f32::min` returns the other operand for a NaN, so the zoom is checked
    // before it is used rather than after.
    if !cell.is_positive() || !zoom.is_finite() || zoom <= 0.0 {
        return Rect::NOTHING;
    }
    let size = (CELL * STEP_SCALE * zoom)
        .min(cell.width())
        .min(cell.height());
    if !size.is_finite() || size <= 0.0 {
        return Rect::NOTHING;
    }
    Rect::from_center_size(cell.center(), Vec2::splat(size))
}

/// The orthogonal wiring between two element rectangles.
///
/// A link between two elements of the same column is a straight vertical run;
/// anywhere else the wire leaves the source at its side, runs across at the
/// target's row and enters the target from its side, so the drawing stays on the
/// sheet grid and never doubles back. Links are derived from the model, never
/// placed by hand, so this is the only place their shape is decided.
fn link_path(from: Rect, to: Rect) -> Vec<Pos2> {
    if !from.is_positive() || !to.is_positive() {
        return Vec::new();
    }
    let source = from.center();
    let target = to.center();
    if (source.x - target.x).abs() < 1.0 {
        let x = source.x;
        return if source.y <= target.y {
            vec![Pos2::new(x, from.bottom()), Pos2::new(x, to.top())]
        } else {
            vec![Pos2::new(x, from.top()), Pos2::new(x, to.bottom())]
        };
    }
    let exit = Pos2::new(
        if target.x > source.x {
            from.right()
        } else {
            from.left()
        },
        source.y,
    );
    let enter = Pos2::new(
        if target.x > source.x {
            to.left()
        } else {
            to.right()
        },
        target.y,
    );
    let corner = Pos2::new(exit.x, enter.y);
    if (corner.y - exit.y).abs() < 0.5 {
        vec![exit, enter]
    } else {
        vec![exit, corner, enter]
    }
}

/// Draws the selection outline of one element.
fn outline(painter: &Painter, view: ViewTransform, rect: Rect, selected: bool, tokens: &Tokens) {
    if !selected || !rect.is_positive() {
        return;
    }
    let ring = rect.expand(3.0 * view.zoom.max(0.4));
    painter.rect_stroke(
        ring,
        CornerRadius::same(RADIUS_PILL),
        Stroke::new(2.0_f32, tokens.accent),
        StrokeKind::Outside,
    );
}

/// Marks the steps and transitions a diagnostic of this page names.
fn paint_problem_marks(
    painter: &Painter,
    view: ViewTransform,
    band: &Band,
    tokens: &Tokens,
    frame: &Frame<'_>,
) {
    for diagnostic in frame.problems {
        let Some(target) = problems::sfc_target(frame.app.project(), diagnostic) else {
            continue;
        };
        if target.section != frame.section || target.page != Some(band.number) {
            continue;
        }
        let colour = tokens.severity(diagnostic.severity);
        let rect = match target.element {
            Some(SfcElement::Step(number)) => band
                .steps
                .iter()
                .find(|step| step.number == number)
                .map(|step| step_rect(view.cell(band, step.x, step.y), view.zoom)),
            Some(SfcElement::Transition(number)) => band
                .transitions
                .iter()
                .find(|transition| transition.number == number)
                .and_then(|transition| transition_rect(view, band, transition))
                .map(|rect| rect.expand(4.0)),
            None => None,
        };
        if let Some(rect) = rect.filter(Rect::is_positive) {
            painter.rect_stroke(
                rect,
                CornerRadius::same(RADIUS_PILL),
                Stroke::new(1.0_f32, colour),
                StrokeKind::Outside,
            );
        }
    }
}

/// Draws the placeholder of a page that holds no step and no transition.
fn paint_empty_page(
    painter: &Painter,
    view: ViewTransform,
    band: &Band,
    tokens: &Tokens,
    frame: &Frame<'_>,
) {
    let area = Rect::from_min_max(
        view.point(GRID_LEFT, band.grid_top()),
        view.point(
            GRID_LEFT + CELL * 6.0,
            band.grid_top() + CELL * band.rows.min(3) as f32,
        ),
    );
    if !area.is_positive() {
        return;
    }
    painter.rect_filled(
        area,
        CornerRadius::same(RADIUS_CONTROL),
        tokens.accent.gamma_multiply(0.03),
    );
    let path = [
        area.left_top(),
        area.right_top(),
        area.right_bottom(),
        area.left_bottom(),
        area.left_top(),
    ];
    painter.extend(Shape::dashed_line(
        &path,
        Stroke::new(1.0_f32, tokens.border),
        6.0,
        6.0,
    ));
    let text = match frame.tool {
        Some(tool) => format!(
            "Click a cell to place {}",
            crate::palette::sfc_short_name(tool)
        ),
        None => "This page has no steps — arm a tool and click a cell".to_owned(),
    };
    painter.text(
        area.center(),
        Align2::CENTER_CENTER,
        text,
        view.font(TypeScale::BODY, FontFamily::Proportional),
        tokens.text_dim,
    );
}

/// Draws the ghost of the armed tool over the target under the pointer.
fn paint_ghost(
    painter: &Painter,
    view: ViewTransform,
    bands: &[Band],
    target: Target,
    tool: Option<SfcTool>,
    tokens: &Tokens,
) {
    let Some(tool) = tool else {
        return;
    };
    let Target::Cell { band, x, y } = target else {
        return;
    };
    let Some(band) = bands.get(band) else {
        return;
    };
    let cell = view.cell(band, x, y);
    if !cell.is_positive() {
        return;
    }
    let outline = cell.shrink(2.0);
    painter.rect_filled(
        outline,
        CornerRadius::same(RADIUS_PILL),
        tokens.accent.gamma_multiply(0.07),
    );
    let path = [
        outline.left_top(),
        outline.right_top(),
        outline.right_bottom(),
        outline.left_bottom(),
        outline.left_top(),
    ];
    painter.extend(Shape::dashed_line(
        &path,
        Stroke::new(1.5_f32, tokens.accent),
        6.0,
        4.0,
    ));
    let glyph = match tool {
        SfcTool::InitialStep | SfcTool::Step => step_rect(cell, view.zoom).shrink(1.0),
        SfcTool::Comment => cell.shrink(CELL * view.zoom * 0.2),
        _ => Rect::from_center_size(
            cell.center(),
            Vec2::new(cell.width() * 0.62, cell.height() * 0.2),
        ),
    };
    tool_glyph(painter, glyph, tokens, tool);
}

/// Applies a click, a drag and the context menu of the document.
#[allow(clippy::too_many_arguments)]
fn handle_pointer(
    app: &mut EditorApp,
    ui: &mut Ui,
    response: &egui::Response,
    bands: &[Band],
    page: &SequentialPage,
    transform: ViewTransform,
    state: &mut View,
    space: bool,
) {
    if space {
        return;
    }
    let Some(section) = section_id(app) else {
        return;
    };
    let primary = egui::PointerButton::Primary;
    let drag_id = egui::Id::new("softladder-sfc-drag");

    if response.drag_started_by(primary) {
        let origin = response
            .ctx
            .input(|input| input.pointer.press_origin())
            .or_else(|| response.interact_pointer_pos());
        let element = origin
            .and_then(|pos| hit(bands, transform, pos))
            .and_then(|target| element_at(bands, target))
            .or(state.selection);
        if let Some(element) = element {
            let from = match element {
                Selection::Step(number) => page.step(number).map(|step| (step.x, step.y)),
                Selection::Transition(number) => page
                    .transition(number)
                    .map(|transition| (transition.x, transition.y)),
                Selection::Page(_) => None,
            };
            if let Some(from) = from {
                state.drag = Some(Drag {
                    element,
                    from,
                    over: Some(from),
                });
                ui.ctx()
                    .data_mut(|data| data.insert_temp(drag_id, Some(element)));
            }
        }
    }
    if response.dragged_by(primary) {
        let source: Option<Selection> = ui.ctx().data(|data| data.get_temp(drag_id)).flatten();
        if source.is_some() {
            if let (Some(drag), Some(pointer)) =
                (state.drag.as_mut(), response.interact_pointer_pos())
            {
                if let Some(Target::Cell { x, y, .. }) = hit(bands, transform, pointer) {
                    drag.over = Some((x, y));
                }
            }
        }
    }
    if response.drag_stopped_by(primary) {
        ui.ctx()
            .data_mut(|data| data.insert_temp(drag_id, None::<Selection>));
        if let Some(drag) = state.drag.take() {
            match drag.over {
                Some(to) if to != drag.from => {
                    move_element(app, section, drag.element, to, state);
                }
                Some(_) => {}
                None => {}
            }
        }
    }

    if response.clicked() {
        let Some(pos) = response.interact_pointer_pos() else {
            return;
        };
        let Some(target) = hit(bands, transform, pos) else {
            return;
        };
        let element = element_at(bands, target);
        match state.tool {
            None => match element {
                Some(element) => select_at(app, state, element),
                None => {
                    // The header selects the page; the sheet around the grid
                    // clears the element selection.
                    match target {
                        Target::Band { band } => {
                            if let Some(number) = bands.get(band).map(|band| band.number) {
                                select_at(app, state, Selection::Page(number));
                            }
                        }
                        Target::Cell { .. } => {
                            state.selection = None;
                            state.error = None;
                            load_buffers(app, state);
                        }
                    }
                }
            },
            Some(tool) => {
                if let Target::Cell { band, x, y } = target {
                    place_at(app, section, bands, state, tool, band, (x, y), element);
                }
            }
        }
    }
    ui.ctx().set_cursor_icon(match state.tool {
        Some(_) => CursorIcon::Crosshair,
        None => CursorIcon::PointingHand,
    });
}

/// Selects `element`, keeping the document's page and buffers in step.
fn select_at(app: &mut EditorApp, state: &mut View, element: Selection) {
    state.selection = Some(element);
    state.error = None;
    if let Selection::Page(number) = element {
        state.page = Some(number);
    }
    load_buffers(app, state);
}

/// Moves the dragged element to `to`.
fn move_element(
    app: &mut EditorApp,
    section: u32,
    element: Selection,
    to: (i32, i32),
    state: &mut View,
) {
    let result = match element {
        Selection::Step(number) => app.editor.move_step(section, number, to.0, to.1),
        Selection::Transition(number) => app.editor.move_transition(section, number, to.0, to.1),
        Selection::Page(_) => return,
    };
    match result {
        Ok(()) => {
            app.after_edit();
            state.selection = Some(element);
        }
        Err(error) => app.note(&error.to_string()),
    }
}

/// Turns a click of an armed tool into the command it places.
#[allow(clippy::too_many_arguments)]
fn place_at(
    app: &mut EditorApp,
    section: u32,
    bands: &[Band],
    state: &mut View,
    tool: SfcTool,
    band: usize,
    cell: (i32, i32),
    element: Option<Selection>,
) {
    let (x, y) = cell;
    let band = bands.get(band);
    let placed = match tool {
        SfcTool::InitialStep | SfcTool::Step => app
            .editor
            .insert_step(section, x, y, tool == SfcTool::InitialStep)
            .map(|number| (Selection::Step(number), "step placed")),
        SfcTool::Transition => app
            .editor
            .insert_transition(section, x, y)
            .map(|number| (Selection::Transition(number), "transition placed")),
        SfcTool::AndDivergence => {
            let (above, below) = match band {
                Some(band) => (
                    column_group(band, x, y, true),
                    column_group(band, x, y, false),
                ),
                None => (Vec::new(), Vec::new()),
            };
            app.editor
                .insert_transition_linked(section, x, y, &above, &below)
                .map(|number| (Selection::Transition(number), "AND divergence placed"))
        }
        SfcTool::OrDivergence => {
            let (above, below) = match band {
                Some(band) => (
                    nearest_step(band, x, y, true).into_iter().collect(),
                    nearest_step(band, x, y, false).into_iter().collect(),
                ),
                None => (Vec::new(), Vec::new()),
            };
            app.editor
                .insert_transition_linked(section, x, y, &above, &below)
                .map(|number| (Selection::Transition(number), "OR divergence placed"))
        }
        SfcTool::Link => {
            link_at(app, section, state, element);
            return;
        }
        SfcTool::Comment => {
            let number = band.map_or(0, |band| band.number);
            select_at(app, state, Selection::Page(number));
            state.tool = None;
            app.note("edit the page comment in the inspector");
            return;
        }
    };
    match placed {
        Ok((selection, note)) => {
            app.after_edit();
            state.selection = Some(selection);
            state.tool = None;
            state.error = None;
            load_buffers(app, state);
            app.note(note);
        }
        Err(error) => app.note(&error.to_string()),
    }
}

/// Toggles one link between the selected element and the clicked one.
///
/// Links are the model's `from`/`to` sets, so "drawing a wire" is a membership
/// change and undoing the command undraws it: the picture can never disagree
/// with what the engine will run.
fn link_at(app: &mut EditorApp, section: u32, state: &mut View, element: Option<Selection>) {
    let Some(element) = element else {
        app.note("click a step or a transition to link");
        return;
    };
    let (result, note) = match (state.selection, element) {
        (Some(Selection::Step(step)), Selection::Transition(transition)) => {
            let linked = !names(app, section, transition, step, true);
            (
                app.editor
                    .link_transition_from(section, transition, step, linked),
                if linked {
                    "linked the step to the transition"
                } else {
                    "unlinked the step"
                },
            )
        }
        (Some(Selection::Transition(transition)), Selection::Step(step)) => {
            let linked = !names(app, section, transition, step, false);
            (
                app.editor
                    .link_transition_to(section, transition, step, linked),
                if linked {
                    "linked the transition to the step"
                } else {
                    "unlinked the step"
                },
            )
        }
        (Some(_), _) => {
            select_at(app, state, element);
            app.note("a link joins a step and a transition");
            return;
        }
        (None, _) => {
            select_at(app, state, element);
            app.note("now click the element to link it with");
            return;
        }
    };
    match result {
        Ok(()) => {
            app.after_edit();
            app.note(note);
        }
        Err(error) => app.note(&error.to_string()),
    }
}

/// `true` when `transition` already names `step` in its `from` (or `to`) set.
fn names(app: &EditorApp, section: u32, transition: u32, step: u32, from: bool) -> bool {
    app.project()
        .section(section)
        .and_then(|section| section.sequential_page.as_ref())
        .and_then(|page| page.transition(transition))
        .map(|transition| {
            if from {
                transition.from.contains(&step)
            } else {
                transition.to.contains(&step)
            }
        })
        .unwrap_or(false)
}

/// The right-click menu of a cell.
fn handle_menu(
    app: &mut EditorApp,
    ui: &mut Ui,
    response: &egui::Response,
    bands: &[Band],
    transform: ViewTransform,
    state: &mut View,
) {
    let id = egui::Id::new("softladder-sfc-menu");
    let stored: Option<Target> = ui.ctx().data(|data| data.get_temp(id)).flatten();
    let click = response.secondary_clicked();
    let target = if click {
        response
            .interact_pointer_pos()
            .and_then(|pos| hit(bands, transform, pos))
    } else {
        stored
    };
    if click {
        ui.ctx().data_mut(|data| data.insert_temp(id, target));
        if let Some(target) = target {
            if let Some(element) = element_at(bands, target) {
                select_at(app, state, element);
            }
        }
    }
    let Some(target) = target else {
        return;
    };
    let opened = response.context_menu(|ui| context_menu(app, ui, bands, state, target));
    if opened.is_none() {
        ui.ctx()
            .data_mut(|data| data.insert_temp(id, None::<Target>));
    }
}

/// The entries of the cell context menu.
fn context_menu(
    app: &mut EditorApp,
    ui: &mut Ui,
    bands: &[Band],
    state: &mut View,
    target: Target,
) {
    let Some(section) = section_id(app) else {
        return;
    };
    let element = element_at(bands, target);
    let heading = match (target, element) {
        (Target::Cell { x, y, .. }, Some(Selection::Step(number))) => {
            format!("step {number} · x {x} y {y}")
        }
        (Target::Cell { x, y, .. }, Some(Selection::Transition(number))) => {
            format!("transition {number} · x {x} y {y}")
        }
        (Target::Cell { x, y, .. }, _) => format!("empty cell · x {x} y {y}"),
        (Target::Band { band }, _) => {
            format!("page {}", bands.get(band).map_or(0, |band| band.number))
        }
    };
    ui.label(RichText::new(heading).size(TypeScale::CAPTION));
    ui.separator();

    let steps = bands
        .iter()
        .flat_map(|band| band.steps.iter())
        .find(|step| Some(Selection::Step(step.number)) == element)
        .cloned();
    let transitions = bands
        .iter()
        .flat_map(|band| band.transitions.iter())
        .find(|transition| Some(Selection::Transition(transition.number)) == element)
        .cloned();

    if let Some(step) = &steps {
        if ui
            .button(if step.is_initial {
                "Clear the initial flag"
            } else {
                "Make this the initial step"
            })
            .on_hover_text("The initial steps are the ones active when the section starts")
            .clicked()
        {
            match app
                .editor
                .set_step_initial(section, step.number, !step.is_initial)
            {
                Ok(()) => app.after_edit(),
                Err(error) => app.note(&error.to_string()),
            }
            ui.close_menu();
        }
        if ui.button("Delete the step").on_hover_text("Del").clicked() {
            select_at(app, state, Selection::Step(step.number));
            delete_selection(app);
            ui.close_menu();
        }
    }
    if let Some(transition) = &transitions {
        if ui
            .button("Clear the condition")
            .on_hover_text("An unconditional transition fires whenever its sources are active")
            .clicked()
        {
            match app
                .editor
                .set_transition_condition(section, transition.number, "")
            {
                Ok(()) => {
                    app.after_edit();
                    load_buffers(app, state);
                }
                Err(error) => app.note(&error.to_string()),
            }
            ui.close_menu();
        }
        if ui
            .button("Delete the transition")
            .on_hover_text("Del")
            .clicked()
        {
            select_at(app, state, Selection::Transition(transition.number));
            delete_selection(app);
            ui.close_menu();
        }
    }
    if let Target::Cell { x, y, .. } = target {
        if element.is_none() {
            for tool in [
                SfcTool::InitialStep,
                SfcTool::Step,
                SfcTool::Transition,
                SfcTool::AndDivergence,
            ] {
                let entry = tool.entry();
                if ui
                    .button(format!("Place {}", entry.label.to_lowercase()))
                    .clicked()
                {
                    place_with(app, state, tool, (x, y));
                    ui.close_menu();
                }
            }
        }
    }
    if let Target::Band { band } = target {
        if let Some(number) = bands.get(band).map(|band| band.number) {
            if ui
                .button("Edit the page comment")
                .on_hover_text("The comment is edited in the inspector")
                .clicked()
            {
                select_at(app, state, Selection::Page(number));
                ui.close_menu();
            }
        }
    }
}

/// Places one element of `tool` at `cell`, as the palette would.
fn place_with(app: &mut EditorApp, state: &mut View, tool: SfcTool, cell: (i32, i32)) {
    let Some(section) = section_id(app) else {
        return;
    };
    let initial = tool == SfcTool::InitialStep;
    let result = match tool {
        SfcTool::Transition | SfcTool::AndDivergence | SfcTool::OrDivergence => {
            app.editor.insert_transition(section, cell.0, cell.1)
        }
        _ => app.editor.insert_step(section, cell.0, cell.1, initial),
    };
    match result {
        Ok(number) => {
            app.after_edit();
            state.selection = Some(match tool {
                SfcTool::InitialStep | SfcTool::Step => Selection::Step(number),
                _ => Selection::Transition(number),
            });
            state.tool = None;
            load_buffers(app, state);
        }
        Err(error) => app.note(&error.to_string()),
    }
}

/// Shows the whole expression of a transition on hover.
fn expression_tooltip(
    response: &egui::Response,
    page: &SequentialPage,
    hovered: Option<Selection>,
) {
    let Some(Selection::Transition(number)) = hovered else {
        return;
    };
    let Some(condition) = page
        .transition(number)
        .and_then(|transition| transition.condition.as_ref())
    else {
        return;
    };
    response.clone().on_hover_text(condition.to_string());
}

/// Pans the camera with the middle button or with space held down.
fn handle_pan(app: &mut EditorApp, response: &egui::Response, space: bool) {
    let middle = response.dragged_by(egui::PointerButton::Middle);
    let space_drag = space && response.dragged_by(egui::PointerButton::Primary);
    if middle || space_drag {
        app.camera.pan_by(response.drag_delta());
    }
}

/// Zooms around the pointer with the wheel.
fn handle_zoom(app: &mut EditorApp, ui: &Ui, response: &egui::Response, paper: Rect) {
    if !response.hovered() {
        return;
    }
    let scroll = ui.input(|input| input.raw_scroll_delta.y);
    if scroll == 0.0 {
        return;
    }
    let old_zoom = app.camera.zoom;
    let new_zoom = layout::clamp_zoom(old_zoom * (scroll * 0.0015).exp());
    if (new_zoom - old_zoom).abs() < f32::EPSILON {
        return;
    }
    if let Some(pointer) = response.hover_pos() {
        let base = paper.min + Vec2::splat(PAPER_PAD);
        let origin = base + app.camera.pan;
        let document = (pointer - origin) / old_zoom;
        app.camera.pan = pointer - document * new_zoom - base;
    }
    app.camera.set_zoom(new_zoom);
}

/// Draws the empty state of the document on the paper.
#[allow(clippy::too_many_arguments)]
fn empty_document(
    app: &mut EditorApp,
    ui: &mut Ui,
    painter: &Painter,
    paper: Rect,
    tokens: &Tokens,
    title: &str,
    body: &str,
    hint: &str,
    add_page: bool,
) {
    painter.rect_stroke(
        paper.shrink(SPACE_3),
        CornerRadius::same(RADIUS_CARD),
        Stroke::new(1.0_f32, tokens.border),
        StrokeKind::Inside,
    );
    let top = paper.top() + paper.height() * 0.26;
    let area = Rect::from_min_size(
        Pos2::new(paper.left(), top),
        Vec2::new(paper.width(), paper.height() * 0.6),
    );
    let section = section_id(app);
    let mut add = false;
    ui.scope_builder(egui::UiBuilder::new().max_rect(area), |ui| {
        empty_state(ui, tokens, title, body, hint);
        if add_page {
            ui.vertical_centered(|ui| {
                add = ui.button("Add a page").clicked();
            });
        }
    });
    if add {
        if let Some(section) = section {
            match app.editor.add_page(section) {
                Ok(number) => {
                    app.after_edit();
                    let mut state = view_of(app);
                    state.page = Some(number);
                    state.selection = Some(Selection::Page(number));
                    set_view(state);
                    app.note("page added");
                }
                Err(error) => app.note(&error.to_string()),
            }
        }
    }
}

/// Draws the state badge of a page header.
fn paint_badge(painter: &Painter, view: ViewTransform, rect: Rect, badge: &Badge) {
    if !rect.is_positive() {
        return;
    }
    let galley = painter.layout_no_wrap(
        badge.text.clone(),
        view.font(TypeScale::CAPTION, FontFamily::Proportional),
        badge.colour,
    );
    let size = Vec2::new(
        (galley.size().x + 12.0).min(rect.width()),
        (galley.size().y + 4.0).min(rect.height().max(galley.size().y + 4.0)),
    );
    let pill = Rect::from_min_size(
        Pos2::new(rect.right() - size.x, rect.center().y - size.y / 2.0),
        size,
    );
    painter.rect_filled(
        pill,
        CornerRadius::same(RADIUS_PILL),
        badge.colour.gamma_multiply(0.14),
    );
    painter.galley(pill.center() - galley.size() / 2.0, galley, badge.colour);
}

/// The steps of the parallel group above (or below) `(x, y)`, up to the next
/// transition of the same column.
///
/// This is what the AND divergence tool wires: the group of steps a junction
/// actually joins, rather than every step of the column.
fn column_group(band: &Band, x: i32, y: i32, above: bool) -> Vec<u32> {
    let mut found: Vec<u32> = Vec::new();
    let mut row = y;
    loop {
        row += if above { -1 } else { 1 };
        if row < 0 || row >= band.rows {
            break;
        }
        if band
            .transitions
            .iter()
            .any(|transition| transition.x == x && transition.y == row)
        {
            break;
        }
        if let Some(step) = band.steps.iter().find(|step| step.x == x && step.y == row) {
            found.push(step.number);
        }
    }
    found
}

/// The nearest step above (or below) `(x, y)`, in the same column when there is
/// one and anywhere on the page otherwise.
///
/// This is what the OR divergence tool branches from: a branch leaves the step
/// nearest the junction, which is the one a person would point at.
fn nearest_step(band: &Band, x: i32, y: i32, above: bool) -> Option<u32> {
    let mut best: Option<&Step> = None;
    for step in &band.steps {
        let correct_side = if above { step.y < y } else { step.y > y };
        if !correct_side {
            continue;
        }
        let better = match best {
            None => true,
            Some(best) => {
                let candidate_distance = (step.y - y).abs();
                let best_distance = (best.y - y).abs();
                candidate_distance < best_distance
                    || (candidate_distance == best_distance
                        && (step.x - x).abs() < (best.x - x).abs())
            }
        };
        if better {
            best = Some(step);
        }
    }
    best.map(|step| step.number)
}

/// Draws the sequential palette band, inside the ribbon's Insert group.
///
/// `docs/UX.md` §12: the Insert group changes with the document, so an SFC
/// section offers the chart's own vocabulary. The band is the ladder palette's
/// shape — a scrollable row of icon chips carrying the shortcut in the tooltip —
/// so the ribbon does not jump when the document switches language.
pub fn palette(app: &mut EditorApp, ui: &mut Ui) {
    let tokens = app.tokens;
    let state = view_of(app);
    let mut pick: Option<SfcTool> = None;
    let mut clear = false;
    egui::ScrollArea::horizontal()
        .id_salt("sfc-palette-band")
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.vertical(|ui| {
                ui.add_space(SPACE_1);
                ui.horizontal(|ui| {
                    ui.add_space(SPACE_1);
                    ui.label(
                        RichText::new("SEQUENTIAL")
                            .size(TypeScale::CAPTION)
                            .color(tokens.text_dim)
                            .strong(),
                    );
                });
                egui::Frame::new()
                    .fill(tokens.panel)
                    .stroke(tokens.hairline())
                    .corner_radius(CornerRadius::same(RADIUS_CONTROL))
                    .inner_margin(egui::Margin::same(SPACE_1 as i8))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing = egui::vec2(SPACE_1, SPACE_1);
                            // The pointer tool first, as on the ladder.
                            if sfc_chip(ui, &tokens, None, "Select", state.tool.is_none(), CHIP_W)
                                .on_hover_text("Pointer tool: click to select, drag to move")
                                .clicked()
                            {
                                clear = true;
                            }
                            for entry in crate::palette::sfc_entries() {
                                let response = sfc_chip(
                                    ui,
                                    &tokens,
                                    Some(entry.tool),
                                    entry.caption,
                                    state.tool == Some(entry.tool),
                                    CHIP_W,
                                )
                                .on_hover_text(format!(
                                    "{}  ({})\n{}",
                                    entry.tooltip, entry.letter, entry.label
                                ));
                                if response.clicked() {
                                    pick = Some(entry.tool);
                                }
                            }
                        });
                    });
            });
        });
    if clear {
        arm(app, None);
    } else if let Some(tool) = pick {
        arm(app, Some(tool));
    }
}

/// Draws one sequential palette chip.
fn sfc_chip(
    ui: &mut Ui,
    tokens: &Tokens,
    tool: Option<SfcTool>,
    caption: &str,
    armed: bool,
    width: f32,
) -> egui::Response {
    let font = FontId::new(TypeScale::CAPTION, FontFamily::Proportional);
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 40.0), Sense::click());
    let border = if armed || response.hovered() {
        Stroke::new(1.0_f32, tokens.accent)
    } else {
        Stroke::new(1.0_f32, tokens.border)
    };
    let fill = if armed {
        tokens.accent_soft
    } else if response.hovered() {
        tokens.surface
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect(
        rect,
        CornerRadius::same(RADIUS_CONTROL),
        fill,
        border,
        StrokeKind::Inside,
    );
    let plate = Rect::from_center_size(
        Pos2::new(rect.center().x, rect.top() + SPACE_1 + 8.5),
        Vec2::new(rect.width() - SPACE_1, 17.0 + SPACE_1),
    );
    ui.painter().rect(
        plate,
        CornerRadius::same(RADIUS_PILL),
        tokens.surface,
        Stroke::NONE,
        StrokeKind::Inside,
    );
    let ink = if armed { tokens.accent } else { tokens.text };
    let glyph_tokens = Tokens {
        wire_idle: ink,
        text: ink,
        ..*tokens
    };
    let glyph = Rect::from_center_size(
        Pos2::new(rect.center().x, rect.top() + SPACE_1 + 8.5),
        Vec2::new((rect.width() - SPACE_2).min(30.0), 17.0),
    );
    match tool {
        Some(tool) => tool_glyph(ui.painter(), glyph, &glyph_tokens, tool),
        None => crate::panels::icons::draw(
            ui.painter(),
            glyph,
            &glyph_tokens,
            crate::panels::icons::Icon::Pointer,
        ),
    }
    let galley = ui
        .painter()
        .layout(caption.to_owned(), font, ink, rect.width() - SPACE_1);
    ui.painter().galley(
        Pos2::new(
            rect.center().x - galley.size().x / 2.0,
            glyph.bottom() + SPACE_1,
        ),
        galley,
        ink,
    );
    if armed {
        ui.painter().hline(
            (rect.left() + SPACE_1)..=(rect.right() - SPACE_1),
            rect.bottom() - 1.0,
            Stroke::new(2.0_f32, tokens.accent),
        );
    }
    response
}

/// Draws the icon of a sequential tool, for the palette chips.
pub fn tool_glyph(painter: &Painter, rect: Rect, tokens: &Tokens, tool: SfcTool) {
    if !rect.is_positive() {
        return;
    }
    let stroke = Stroke::new(1.4_f32, tokens.wire_idle);
    let centre = rect.center();
    let size = rect.height().min(rect.width()) * 0.72;
    let square = Rect::from_center_size(centre, Vec2::splat(size));
    match tool {
        SfcTool::InitialStep | SfcTool::Step => {
            painter.rect_stroke(
                square,
                CornerRadius::same(RADIUS_PILL),
                stroke,
                StrokeKind::Middle,
            );
            if tool == SfcTool::InitialStep {
                painter.rect_stroke(
                    square.shrink(2.0),
                    CornerRadius::same(RADIUS_PILL),
                    stroke,
                    StrokeKind::Middle,
                );
            }
        }
        SfcTool::Transition => {
            painter.hline(
                (centre.x - size / 2.0)..=(centre.x + size / 2.0),
                centre.y,
                stroke,
            );
            painter.vline(
                centre.x,
                (centre.y - size / 2.0)..=(centre.y + size / 2.0),
                stroke,
            );
        }
        SfcTool::Link => {
            painter.vline(
                centre.x - size / 2.0,
                (centre.y - size / 2.0)..=(centre.y + size / 2.0),
                stroke,
            );
            painter.vline(
                centre.x + size / 2.0,
                (centre.y - size / 2.0)..=(centre.y + size / 2.0),
                stroke,
            );
            painter.hline(
                (centre.x - size / 2.0)..=(centre.x + size / 2.0),
                centre.y,
                stroke,
            );
        }
        SfcTool::AndDivergence | SfcTool::OrDivergence => {
            let bars: &[f32] = if tool == SfcTool::AndDivergence {
                &[-2.5, 2.5]
            } else {
                &[0.0]
            };
            for offset in bars {
                painter.hline(
                    (centre.x - size / 2.0)..=(centre.x + size / 2.0),
                    centre.y + offset,
                    stroke,
                );
            }
            painter.vline(
                centre.x,
                (centre.y - size / 2.0)..=(centre.y + size / 2.0),
                stroke,
            );
            if tool == SfcTool::OrDivergence {
                for direction in [-1.0f32, 1.0] {
                    painter.line_segment(
                        [
                            Pos2::new(centre.x, centre.y - 2.0),
                            Pos2::new(centre.x + direction * size / 3.0, centre.y - size / 2.0),
                        ],
                        stroke,
                    );
                }
            }
        }
        SfcTool::Comment => {
            painter.rect_stroke(
                square,
                CornerRadius::same(RADIUS_PILL),
                stroke,
                StrokeKind::Middle,
            );
            painter.hline(
                (square.left() + 2.0)..=(square.right() - 2.0),
                square.center().y - 2.0,
                stroke,
            );
            painter.hline(
                (square.left() + 2.0)..=square.center().x,
                square.center().y + 2.0,
                stroke,
            );
        }
    }
}

/// Handles one keyboard event of the SFC document.
///
/// Returns `true` when the key was consumed. The document owns its own keyboard
/// map — the letters arm its tools, `Del` deletes the selected element and the
/// arrows nudge it — because the ladder's map is about cells and elements and
/// every action it maps is a no-op on a sequential section.
pub fn handle_key(app: &mut EditorApp, key: Key, modifiers: Modifiers) -> bool {
    if modifiers.command || modifiers.alt {
        return false;
    }
    match key {
        Key::Delete | Key::Backspace => {
            delete_selection(app);
            true
        }
        Key::ArrowLeft => {
            nudge(app, -1, 0);
            true
        }
        Key::ArrowRight => {
            nudge(app, 1, 0);
            true
        }
        Key::ArrowUp => {
            nudge(app, 0, -1);
            true
        }
        Key::ArrowDown => {
            nudge(app, 0, 1);
            true
        }
        Key::Escape => {
            arm(app, None);
            select(app, None);
            true
        }
        other => match crate::palette::sfc_tool_for_key(other) {
            Some(tool) => {
                arm(app, Some(tool));
                true
            }
            None => false,
        },
    }
}

/// Deletes the selected step or transition.
pub fn delete_selection(app: &mut EditorApp) {
    let Some(section) = section_id(app) else {
        return;
    };
    let mut state = view_of(app);
    let result = match state.selection {
        Some(Selection::Step(number)) => app.editor.remove_step(section, number).map(|()| {
            state.selection = None;
            "step deleted"
        }),
        Some(Selection::Transition(number)) => {
            app.editor.remove_transition(section, number).map(|()| {
                state.selection = None;
                "transition deleted"
            })
        }
        Some(Selection::Page(_)) => {
            app.note("select a step or a transition to delete");
            return;
        }
        None => {
            app.note("nothing is selected");
            return;
        }
    };
    match result {
        Ok(note) => {
            app.after_edit();
            load_buffers(app, &mut state);
            set_view(state);
            app.note(note);
        }
        Err(error) => {
            set_view(state);
            app.note(&error.to_string());
        }
    }
}

/// Moves the selected step or transition by whole cells.
pub fn nudge(app: &mut EditorApp, dx: i32, dy: i32) {
    let Some(section) = section_id(app) else {
        return;
    };
    let state = view_of(app);
    let result = match state.selection {
        Some(Selection::Step(number)) => match cell_of_step(app, number) {
            Some((x, y)) => app.editor.move_step(
                section,
                number,
                (x + dx).clamp(0, MAX_COLS - 1),
                (y + dy).clamp(0, MAX_ROWS - 1),
            ),
            None => return,
        },
        Some(Selection::Transition(number)) => match cell_of_transition(app, number) {
            Some((x, y)) => app.editor.move_transition(
                section,
                number,
                (x + dx).clamp(0, MAX_COLS - 1),
                (y + dy).clamp(0, MAX_ROWS - 1),
            ),
            None => return,
        },
        _ => {
            app.note("select a step or a transition to move");
            return;
        }
    };
    match result {
        Ok(()) => {
            app.after_edit();
            set_view(state);
        }
        Err(error) => {
            set_view(state);
            app.note(&error.to_string());
        }
    }
}

/// The cell of a step of the open section.
fn cell_of_step(app: &EditorApp, number: u32) -> Option<(i32, i32)> {
    open_page(app)
        .ok()?
        .step(number)
        .map(|step| (step.x, step.y))
}

/// The cell of a transition of the open section.
fn cell_of_transition(app: &EditorApp, number: u32) -> Option<(i32, i32)> {
    open_page(app)
        .ok()?
        .transition(number)
        .map(|transition| (transition.x, transition.y))
}

/// The live activity of a step, read from the engine's store.
fn step_active(store: &VarStore, number: u32) -> bool {
    store
        .get(&VarRef::new(VarKind::Step, number).with_accessor(Accessor::Activity))
        .map(Value::as_bool)
        .unwrap_or(false)
}

/// The elapsed time of a step, as the chip draws it.
fn step_time(store: &VarStore, number: u32) -> Option<String> {
    let value = store.get(&VarRef::new(VarKind::Step, number).with_accessor(Accessor::Value))?;
    let milliseconds = match value {
        Value::Word(word) => i64::from(word),
        Value::DWord(word) => word,
        Value::Bit(bit) => i64::from(bit),
        Value::Real(real) if real.is_finite() => real as i64,
        Value::Real(_) => return None,
    };
    Some(format!("{milliseconds} ms"))
}

/// Draws the inspector for the SFC document.
///
/// With a step selected: its number, its initial flag, its page and cell and,
/// while the bench runs, its activity and elapsed time. With a transition: its
/// condition, with the same validation and tag picker as the ladder, and the
/// steps it deactivates and activates. With a page: its comment.
pub fn inspector(app: &mut EditorApp, ui: &mut Ui) {
    let tokens = app.tokens;
    let mut state = view_of(app);
    section_header(ui, &tokens, "Sequential");
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let Ok(page) = open_page(app) else {
                ui.add_space(SPACE_3);
                ui.label(
                    RichText::new("This section has no page yet.")
                        .size(TypeScale::BODY)
                        .color(tokens.text_dim),
                );
                if let Some(section) = section_id(app) {
                    if ui.button("Add a page").clicked() {
                        match app.editor.add_page(section) {
                            Ok(number) => {
                                app.after_edit();
                                state.page = Some(number);
                                state.selection = Some(Selection::Page(number));
                            }
                            Err(error) => app.note(&error.to_string()),
                        }
                    }
                }
                return;
            };
            page_selector(app, ui, &page, &mut state);
            ui.separator();
            match state.selection {
                Some(Selection::Step(number)) => step_inspector(app, ui, &page, number, &mut state),
                Some(Selection::Transition(number)) => {
                    transition_inspector(app, ui, &page, number, &mut state)
                }
                _ => page_inspector(app, ui, &page, &mut state),
            }
        });
    set_view(state);
}

/// The row of buttons that jumps between the pages of the section.
fn page_selector(app: &EditorApp, ui: &mut Ui, page: &SequentialPage, state: &mut View) {
    let tokens = app.tokens;
    let mut numbers: Vec<u32> = vec![page.number];
    for step in &page.steps {
        if !numbers.contains(&step.page) {
            numbers.push(step.page);
        }
    }
    for transition in &page.transitions {
        if !numbers.contains(&transition.page) {
            numbers.push(transition.page);
        }
    }
    numbers.sort_unstable();
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new("Page")
                .size(TypeScale::CAPTION)
                .color(tokens.text_dim),
        );
        for number in numbers {
            if ui
                .selectable_label(state.page == Some(number), number.to_string())
                .on_hover_text(format!("Show page {number}"))
                .clicked()
            {
                state.page = Some(number);
                state.selection = Some(Selection::Page(number));
            }
        }
    });
}

/// The properties of the selected step.
fn step_inspector(
    app: &mut EditorApp,
    ui: &mut Ui,
    page: &SequentialPage,
    number: u32,
    state: &mut View,
) {
    let tokens = app.tokens;
    let Some(step) = page.step(number).cloned() else {
        state.selection = None;
        property(ui, &tokens, "Step", "—");
        return;
    };
    let section = section_id(app);
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 16.0), Sense::hover());
        tool_glyph(
            ui.painter(),
            rect,
            &tokens,
            if step.is_initial {
                SfcTool::InitialStep
            } else {
                SfcTool::Step
            },
        );
        ui.label(
            RichText::new(format!("Step {}", step.number))
                .size(TypeScale::EMPHASIS)
                .color(tokens.text)
                .strong(),
        );
    });
    property(ui, &tokens, "Page", step.page.to_string());
    property(ui, &tokens, "Cell", format!("x {} · y {}", step.x, step.y));

    ui.add_space(SPACE_2);
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("Number")
                .size(TypeScale::CAPTION)
                .color(tokens.text_dim),
        );
        let response = ui.add(
            egui::TextEdit::singleline(&mut state.number)
                .desired_width(60.0)
                .font(egui::TextStyle::Monospace),
        );
        if commit(&response, ui) {
            commit_number(app, state, step.number);
        }
    });
    if let Some(error) = state.error.clone() {
        field_error(ui, &tokens, &error);
    }

    let mut initial = step.is_initial;
    if ui
        .checkbox(&mut initial, "Active at start-up")
        .on_hover_text("The initial steps are the ones active when the section starts")
        .changed()
    {
        if let Some(section) = section {
            match app.editor.set_step_initial(section, step.number, initial) {
                Ok(()) => {
                    state.error = None;
                    app.after_edit();
                }
                Err(error) => state.error = Some(error.to_string()),
            }
        }
    }

    ui.add_space(SPACE_2);
    let live = app.bench().state().is_scanning() || app.bench().cycles() > 0;
    if live {
        let store = store(app);
        property(
            ui,
            &tokens,
            "Activity",
            if step_active(store, step.number) {
                "active"
            } else {
                "idle"
            },
        );
        property(
            ui,
            &tokens,
            "Timer",
            step_time(store, step.number).unwrap_or_else(|| "—".to_owned()),
        );
    } else {
        ui.label(
            RichText::new("Run the bench to see the activity and the timer.")
                .size(TypeScale::CAPTION)
                .color(tokens.text_dim),
        );
    }

    ui.add_space(SPACE_2);
    if ui
        .button("Delete the step")
        .on_hover_text("The transitions that named it are pruned with it (Ctrl+Z undoes it)")
        .clicked()
    {
        delete_selection(app);
    }
}

/// Commits the step number field, reporting a refusal next to it.
fn commit_number(app: &mut EditorApp, state: &mut View, current: u32) {
    let Some(section) = section_id(app) else {
        return;
    };
    let text = state.number.trim().to_owned();
    let Ok(number) = text.parse::<u32>() else {
        state.error = Some(format!("`{text}` is not a whole number"));
        state.number = current.to_string();
        return;
    };
    if number == current {
        state.error = None;
        return;
    }
    match app.editor.set_step_number(section, current, number) {
        Ok(()) => {
            state.selection = Some(Selection::Step(number));
            state.error = None;
            app.after_edit();
        }
        Err(error) => {
            state.error = Some(error.to_string());
            state.number = current.to_string();
        }
    }
}

/// The properties of the selected transition.
fn transition_inspector(
    app: &mut EditorApp,
    ui: &mut Ui,
    page: &SequentialPage,
    number: u32,
    state: &mut View,
) {
    let tokens = app.tokens;
    let Some(transition) = page.transition(number).cloned() else {
        state.selection = None;
        property(ui, &tokens, "Transition", "—");
        return;
    };
    let junction = junction_of(page, &transition);
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 16.0), Sense::hover());
        tool_glyph(
            ui.painter(),
            rect,
            &tokens,
            match junction {
                Some("AND") => SfcTool::AndDivergence,
                Some(_) => SfcTool::OrDivergence,
                None => SfcTool::Transition,
            },
        );
        ui.label(
            RichText::new(format!("Transition {}", transition.number))
                .size(TypeScale::EMPHASIS)
                .color(tokens.text)
                .strong(),
        );
        if let Some(junction) = junction {
            quiet_pill(
                ui,
                if junction == "AND" {
                    tokens.warning
                } else {
                    tokens.accent
                },
                junction,
            );
        }
    });
    property(ui, &tokens, "Page", transition.page.to_string());
    property(
        ui,
        &tokens,
        "Cell",
        format!("x {} · y {}", transition.x, transition.y),
    );

    ui.add_space(SPACE_2);
    ui.label(
        RichText::new("Condition")
            .size(TypeScale::CAPTION)
            .color(tokens.text_dim),
    );
    ui.horizontal(|ui| {
        let response = ui.add(
            egui::TextEdit::singleline(&mut state.condition)
                .desired_width(f32::INFINITY)
                .hint_text("always")
                .font(egui::TextStyle::Monospace),
        );
        if commit(&response, ui) {
            commit_condition(app, state, transition.number);
        }
    });
    let mut pick: Option<String> = None;
    ui.horizontal_wrapped(|ui| {
        ui.menu_button("Tag…", |ui| {
            let mut vars = queries::used_vars(app.project());
            for step in &page.steps {
                let activity = VarRef::new(VarKind::Step, step.number);
                if !vars.contains(&activity) {
                    vars.push(activity);
                }
            }
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
        if ui
            .button("Clear")
            .on_hover_text("An unconditional transition fires whenever its sources are active")
            .clicked()
        {
            state.condition.clear();
            commit_condition(app, state, transition.number);
        }
        if let Some(var) = pick {
            state.condition = var;
            commit_condition(app, state, transition.number);
        }
    });
    if let Some(error) = state.error.clone() {
        field_error(ui, &tokens, &error);
    }

    ui.add_space(SPACE_2);
    ui.label(
        RichText::new("Deactivates (all must be active)")
            .size(TypeScale::CAPTION)
            .color(tokens.text_dim),
    );
    step_toggles(
        app,
        ui,
        page,
        &transition.from,
        transition.number,
        true,
        state,
    );
    ui.add_space(SPACE_1);
    ui.label(
        RichText::new("Activates")
            .size(TypeScale::CAPTION)
            .color(tokens.text_dim),
    );
    step_toggles(
        app,
        ui,
        page,
        &transition.to,
        transition.number,
        false,
        state,
    );

    ui.add_space(SPACE_2);
    if ui
        .button("Delete the transition")
        .on_hover_text("Ctrl+Z undoes it")
        .clicked()
    {
        delete_selection(app);
    }
}

/// A check box per step of the page, adding or removing it from a transition set.
fn step_toggles(
    app: &mut EditorApp,
    ui: &mut Ui,
    page: &SequentialPage,
    members: &[u32],
    transition: u32,
    from: bool,
    state: &mut View,
) {
    if page.steps.is_empty() {
        ui.label(RichText::new("This page has no steps yet").weak());
        return;
    }
    let Some(section) = section_id(app) else {
        return;
    };
    let mut wanted: Option<Vec<u32>> = None;
    ui.horizontal_wrapped(|ui| {
        let mut set = members.to_vec();
        for step in &page.steps {
            let mut linked = set.contains(&step.number);
            let label = if step.is_initial {
                format!("{} (init)", step.number)
            } else {
                step.number.to_string()
            };
            if ui
                .checkbox(&mut linked, label)
                .on_hover_text("Link the step to this transition")
                .changed()
            {
                set.retain(|entry| *entry != step.number);
                if linked {
                    set.push(step.number);
                }
                wanted = Some(set.clone());
            }
        }
    });
    if let Some(set) = wanted {
        let result = if from {
            app.editor.set_transition_from(section, transition, &set)
        } else {
            app.editor.set_transition_to(section, transition, &set)
        };
        match result {
            Ok(()) => {
                state.error = None;
                app.after_edit();
            }
            Err(error) => state.error = Some(error.to_string()),
        }
    }
}

/// Commits the condition field through the editor, reporting a refusal in place.
fn commit_condition(app: &mut EditorApp, state: &mut View, transition: u32) {
    let Some(section) = section_id(app) else {
        return;
    };
    let text = state.condition.clone();
    match app
        .editor
        .set_transition_condition(section, transition, &text)
    {
        Ok(()) => {
            state.error = None;
            app.after_edit();
        }
        Err(error) => state.error = Some(error.to_string()),
    }
}

/// The properties of the open page.
fn page_inspector(app: &mut EditorApp, ui: &mut Ui, page: &SequentialPage, state: &mut View) {
    let tokens = app.tokens;
    let number = state.page.unwrap_or(page.number);
    ui.label(
        RichText::new(format!("Page {number}"))
            .size(TypeScale::EMPHASIS)
            .color(tokens.text)
            .strong(),
    );
    property(ui, &tokens, "Steps", page.steps.len().to_string());
    property(
        ui,
        &tokens,
        "Transitions",
        page.transitions.len().to_string(),
    );
    let initial = page.steps.iter().filter(|step| step.is_initial).count();
    property(ui, &tokens, "Initial steps", initial.to_string());

    ui.add_space(SPACE_2);
    ui.label(
        RichText::new("Comment")
            .size(TypeScale::CAPTION)
            .color(tokens.text_dim),
    );
    let response = ui.add(
        egui::TextEdit::multiline(&mut state.comment)
            .desired_width(f32::INFINITY)
            .desired_rows(2),
    );
    if commit(&response, ui) {
        if let Some(section) = section_id(app) {
            let comment = state.comment.clone();
            match app.editor.set_page_comment(section, &comment) {
                Ok(()) => {
                    state.error = None;
                    app.after_edit();
                }
                Err(error) => state.error = Some(error.to_string()),
            }
        }
    }
    if let Some(error) = state.error.clone() {
        field_error(ui, &tokens, &error);
    }

    ui.add_space(SPACE_2);
    ui.horizontal_wrapped(|ui| {
        if ui
            .button("Remove the page")
            .on_hover_text("Takes the steps and transitions with it (Ctrl+Z undoes it)")
            .clicked()
        {
            if let Some(section) = section_id(app) {
                match app.editor.remove_page(section) {
                    Ok(()) => {
                        state.selection = None;
                        state.page = None;
                        app.after_edit();
                    }
                    Err(error) => state.error = Some(error.to_string()),
                }
            }
        }
        if ui
            .button("Place an initial step")
            .on_hover_text("Arm the tool, then click a cell")
            .clicked()
        {
            arm(app, Some(SfcTool::InitialStep));
        }
    });
}

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

/// The validation message of a field, drawn next to it rather than in a modal.
fn field_error(ui: &mut Ui, tokens: &Tokens, message: &str) {
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new(message)
                .size(TypeScale::CAPTION)
                .color(tokens.error),
        );
    });
}

/// Whether a text field asked to commit: focus left it, or `Enter` was pressed.
fn commit(response: &egui::Response, ui: &Ui) -> bool {
    response.lost_focus()
        || (response.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)))
}

/// The sequential palette entries, as the Insert menu lists them.
pub fn entries() -> &'static [SfcEntry] {
    crate::palette::sfc_entries()
}

/// A command that inserts a step, for callers that build charts themselves.
///
/// The screenshot harness and the tests use it to place a chart through the same
/// command path the document uses.
pub fn insert_step_command(section: u32, step: Step) -> Command {
    Command::InsertStep { section, step }
}

#[cfg(test)]
mod tests {
    use super::*;
    use softladder_core::{Expr, Project, Rung, Section};

    /// The window the headless frames are laid out in.
    const TEST_SIZE: Vec2 = Vec2::new(1200.0, 800.0);

    /// A page with two stacked steps and the transition between them.
    fn page() -> SequentialPage {
        let mut page = SequentialPage::new(3, "start-up and stop");
        page.steps.push(Step {
            number: 0,
            is_initial: true,
            x: 0,
            y: 0,
            page: 3,
        });
        page.steps.push(Step {
            number: 1,
            is_initial: false,
            x: 0,
            y: 2,
            page: 3,
        });
        page.transitions.push(Transition {
            number: 0,
            condition: Some("%I0".parse::<Expr>().expect("a condition parses")),
            from: vec![0],
            to: vec![1],
            page: 3,
            x: 0,
            y: 1,
        });
        page
    }

    /// An editor with the SFC section selected, as clicking it in the tree does.
    fn app() -> EditorApp {
        let mut app = EditorApp::new(project());
        app.select_section(1);
        app
    }

    /// A project whose SFC section owns [`page`].
    fn project() -> Project {
        let mut project = Project::new("sfc document");
        let mut main = Section::new(1, "Main");
        main.rungs.push(1);
        project.sections.push(main);
        project.rungs.push(Rung::new(1));
        project.sections.push(Section::sfc(2, "Sequence", page()));
        project
    }

    /// A band wrapping `page`'s elements, without a painter.
    fn band_of(page: &SequentialPage) -> Band {
        let rows = page
            .steps
            .iter()
            .map(|step| step.y)
            .chain(page.transitions.iter().map(|t| t.y))
            .max()
            .map_or(MIN_ROWS - 1, |row| row)
            .saturating_add(1)
            .clamp(MIN_ROWS, MAX_ROWS);
        Band {
            number: page.number,
            comment: page.comment.clone(),
            steps: page.steps.clone(),
            transitions: page.transitions.clone(),
            top: 0.0,
            height: 400.0,
            rows,
            cols: MIN_COLS,
            comment_lines: 1,
        }
    }

    /// A transform at zoom `1.0` with the document origin at `(10, 20)`.
    fn transform() -> ViewTransform {
        ViewTransform {
            origin: Pos2::new(10.0, 20.0),
            zoom: 1.0,
        }
    }

    /// Runs one headless frame of the document, and one of the inspector.
    fn frame(app: &mut EditorApp) {
        let ctx = egui::Context::default();
        let input = || egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, TEST_SIZE)),
            ..egui::RawInput::default()
        };
        for _ in 0..2 {
            let _ = ctx.run(input(), |ctx| {
                egui::SidePanel::right("inspector").show(ctx, |ui| inspector(app, ui));
                egui::CentralPanel::default().show(ctx, |ui| show(app, ui));
            });
        }
    }

    #[test]
    fn the_cell_mapping_round_trips() {
        let band = band_of(&page());
        let view = transform();
        for (x, y) in [(0_i32, 0_i32), (1, 0), (0, 5), (13, 5)] {
            let cell = view.cell(&band, x, y);
            assert_eq!(view.cell_at(&band, cell.center()), Some((x, y)));
            assert_eq!(
                view.cell_at(&band, cell.min + Vec2::splat(0.5)),
                Some((x, y))
            );
        }
        // Outside the grid: before the origin, and past the last column or row.
        let origin = view.point(GRID_LEFT, band.grid_top());
        assert_eq!(view.cell_at(&band, origin - Vec2::splat(1.0)), None);
        assert_eq!(
            view.cell_at(&band, view.cell(&band, band.cols, 0).center()),
            None
        );
        assert_eq!(
            view.cell_at(&band, view.cell(&band, 0, band.rows).center()),
            None
        );
        // A degenerate transform addresses nothing rather than panicking.
        let broken = ViewTransform {
            origin: Pos2::new(f32::NAN, 0.0),
            zoom: 0.0,
        };
        assert_eq!(broken.cell_at(&band, Pos2::ZERO), None);
    }

    #[test]
    fn links_are_wired_orthogonally() {
        let above = Rect::from_min_size(Pos2::new(100.0, 100.0), Vec2::splat(36.0));
        let below = Rect::from_min_size(Pos2::new(100.0, 200.0), Vec2::splat(36.0));
        // Same column: a straight vertical run from edge to edge.
        let path = link_path(above, below);
        assert_eq!(path.len(), 2);
        assert_eq!(path[0], Pos2::new(above.center().x, above.bottom()));
        assert_eq!(path[1], Pos2::new(below.center().x, below.top()));

        // Different columns: out of the side, across at the target's row, in.
        let right = Rect::from_min_size(Pos2::new(300.0, 200.0), Vec2::splat(36.0));
        let path = link_path(above, right);
        assert_eq!(path.len(), 3, "one elbow");
        assert_eq!(path[0], Pos2::new(above.right(), above.center().y));
        assert_eq!(path[1], Pos2::new(above.right(), right.center().y));
        assert_eq!(path[2], Pos2::new(right.left(), right.center().y));
        // The segments are axis-parallel, which is what keeps the drawing on
        // the sheet grid.
        for pair in path.windows(2) {
            let delta = pair[1] - pair[0];
            assert!(
                delta.x.abs() < 1e-3 || delta.y.abs() < 1e-3,
                "a wire segment is not axis-parallel: {delta:?}"
            );
        }

        // The same row is a single straight run.
        let beside = Rect::from_min_size(Pos2::new(300.0, 100.0), Vec2::splat(36.0));
        assert_eq!(link_path(above, beside).len(), 2);
        // A degenerate rectangle has no wire at all.
        assert!(link_path(Rect::NOTHING, right).is_empty());
        assert!(link_path(above, Rect::NOTHING).is_empty());
    }

    #[test]
    fn and_junctions_are_drawn_with_the_double_bar() {
        let mut transition = Transition::new(0, 0);
        assert!(!is_and(&transition), "one source and one target is an OR");
        assert_eq!(bar_offsets(false), &[0.0]);

        transition.from = vec![0, 1];
        assert!(is_and(&transition), "two sources are an AND convergence");
        assert_eq!(bar_offsets(true).len(), 2);
        assert!(bar_offsets(true)[0] < bar_offsets(true)[1]);

        let mut transition = Transition::new(1, 0);
        transition.to = vec![0, 1, 2];
        assert!(is_and(&transition), "three targets are an AND divergence");
    }

    #[test]
    fn a_junction_is_named_only_when_it_is_one() {
        let page = page();
        // A plain transition in a sequence is neither an AND nor an OR.
        assert_eq!(junction_of(&page, &page.transitions[0]), None);

        let mut two_sources = page.clone();
        two_sources.transitions[0].from = vec![0, 1];
        assert_eq!(
            junction_of(&two_sources, &two_sources.transitions[0]),
            Some("AND")
        );

        let mut branches = page.clone();
        branches.transitions.push(Transition {
            number: 1,
            condition: None,
            from: vec![0],
            to: vec![5],
            page: 0,
            x: 2,
            y: 1,
        });
        assert_eq!(
            junction_of(&branches, &branches.transitions[0]),
            Some("OR"),
            "two transitions leaving the same step are an OR divergence"
        );
        let mut merge = page.clone();
        merge.transitions.push(Transition {
            number: 1,
            condition: None,
            from: vec![3],
            to: vec![1],
            page: 0,
            x: 2,
            y: 1,
        });
        assert_eq!(
            junction_of(&merge, &merge.transitions[0]),
            Some("OR"),
            "two transitions activating the same step are an OR convergence"
        );
    }

    #[test]
    fn step_squares_and_transition_bars_stay_inside_their_cell() {
        let cell = Rect::from_min_size(Pos2::ZERO, Vec2::splat(CELL));
        let square = step_rect(cell, 1.0);
        assert!(cell.contains(square.min) && cell.contains(square.max));
        assert!(square.width() < cell.width(), "the cell shows its grid");
        assert_eq!(square.center(), cell.center());
        // A degenerate cell draws nothing rather than panicking.
        assert_eq!(step_rect(Rect::NOTHING, 1.0), Rect::NOTHING);
        assert_eq!(step_rect(cell, 0.0), Rect::NOTHING);
        assert_eq!(step_rect(cell, f32::NAN), Rect::NOTHING);

        let band = band_of(&page());
        let view = transform();
        let bar = transition_rect(view, &band, &band.transitions[0]).expect("a bar");
        assert!(bar.width() > 0.0 && bar.height() > 0.0);
        assert!(bar.center().x > 0.0 && bar.center().y > 0.0);
    }

    #[test]
    fn the_document_stacks_one_band_per_page() {
        let mut page = self::page();
        page.steps.push(Step {
            number: 2,
            is_initial: false,
            x: 1,
            y: 0,
            page: 9,
        });
        let mut project = Project::new("two pages");
        project.sections.push(Section::sfc(2, "Sequence", page));
        let app = EditorApp::new(project);
        // A painter is needed for the comment wrap, so use a real frame.
        let ctx = egui::Context::default();
        let mut bands = Vec::new();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                bands = read_bands(
                    &app.project().sections[0]
                        .sequential_page
                        .clone()
                        .expect("page"),
                    ui.painter(),
                    &Tokens::light(),
                );
            });
        });
        assert_eq!(
            bands.len(),
            2,
            "one band per page number: {:?}",
            bands.iter().map(|b| b.number).collect::<Vec<_>>()
        );
        assert_eq!(bands[0].number, 3);
        assert_eq!(bands[0].comment, "start-up and stop");
        assert_eq!(
            bands[0].steps.len(),
            2,
            "page 9's step stays on its own band"
        );
        assert_eq!(bands[1].number, 9);
        assert!(bands[1].comment.is_empty());
        assert_eq!(bands[1].steps.len(), 1);
        assert!(
            bands[0].bottom() <= bands[1].top,
            "the bands do not overlap"
        );
        assert!(bands[0].rows >= MIN_ROWS && bands[0].cols >= MIN_COLS);
        assert_eq!(
            bands[1].cols, MIN_COLS,
            "a page with one step still offers a grid to click"
        );
    }

    #[test]
    fn the_divergence_tools_pick_the_group_and_the_nearest_step() {
        let mut band = band_of(&page());
        // A second and third step of the parallel group above and below the
        // transition at (0, 1).
        band.steps.push(Step {
            number: 2,
            is_initial: false,
            x: 0,
            y: 3,
            page: 3,
        });
        assert_eq!(column_group(&band, 0, 1, true), vec![0]);
        assert_eq!(column_group(&band, 0, 1, false), vec![1, 2]);
        // A transition stops the group: the one at row 1 bounds the walk down.
        assert_eq!(column_group(&band, 0, 0, false), Vec::<u32>::new());
        assert_eq!(column_group(&band, 5, 1, true), Vec::<u32>::new());

        assert_eq!(nearest_step(&band, 0, 1, true), Some(0));
        assert_eq!(nearest_step(&band, 0, 1, false), Some(1));
        // A branch placed to the side still leaves the nearest step.
        band.steps.push(Step {
            number: 3,
            is_initial: false,
            x: 4,
            y: 1,
            page: 3,
        });
        assert_eq!(nearest_step(&band, 5, 2, true), Some(3));
        assert_eq!(nearest_step(&band, 5, 0, true), None, "nothing is above");
    }

    #[test]
    fn a_condition_reads_as_a_tag_over_its_address() {
        let mut project = Project::new("labels");
        project.symbols.push(softladder_core::Symbol {
            name: "start".to_owned(),
            var: Some("%I0".parse().expect("a variable parses")),
            comment: String::new(),
            unit: None,
        });
        let mut transition = Transition::new(0, 0);
        assert_eq!(
            condition_label(&project, &transition),
            (None, "always".to_owned())
        );
        transition.condition = Some("%I0".parse().expect("a condition parses"));
        assert_eq!(
            condition_label(&project, &transition),
            (Some("start".to_owned()), "%I0".to_owned())
        );
        transition.condition = Some("%I0 AND %I1".parse().expect("a condition parses"));
        let (name, text) = condition_label(&project, &transition);
        assert_eq!(name, None, "an expression has no single tag");
        assert!(
            text.contains("AND"),
            "the expression itself is drawn: {text}"
        );
    }

    #[test]
    fn the_document_draws_every_state_without_panicking() {
        reset_view();
        let mut app = app();
        frame(&mut app);
        // An armed tool, a selection, a drag and a page selection.
        arm(&mut app, Some(SfcTool::InitialStep));
        frame(&mut app);
        select(&mut app, Some(Selection::Step(0)));
        frame(&mut app);
        select(&mut app, Some(Selection::Transition(0)));
        frame(&mut app);
        select(&mut app, Some(Selection::Page(3)));
        frame(&mut app);
        let mut state = view_of(&app);
        state.drag = Some(Drag {
            element: Selection::Step(0),
            from: (0, 0),
            over: Some((1, 1)),
        });
        set_view(state);
        frame(&mut app);
        // Running, with the initial step active and a live timer.
        app.handle(crate::shortcuts::Action::RunStop);
        for _ in 0..5 {
            app.single_scan();
        }
        select(&mut app, Some(Selection::Step(0)));
        frame(&mut app);
        // Both themes, and a section with no page at all.
        app.set_theme(crate::design::Theme::Dark);
        frame(&mut app);
        let mut no_page = Project::new("no page");
        let mut section = Section::new(1, "Sequence");
        section.language = SectionLanguage::Sfc;
        no_page.sections.push(section);
        let mut app = EditorApp::new(no_page);
        frame(&mut app);
        // And an empty project.
        let mut app = EditorApp::new(Project::new("empty"));
        frame(&mut app);
        reset_view();
    }

    #[test]
    fn a_hostile_chart_draws_without_panicking() {
        reset_view();
        let mut page = SequentialPage::new(4, "hostile");
        // A step recorded on another page, a dangling reference, a negative
        // coordinate and a step far past the sheet.
        page.steps.push(Step {
            number: 0,
            is_initial: true,
            x: -3,
            y: -2,
            page: 9,
        });
        page.steps.push(Step {
            number: 1,
            is_initial: false,
            x: 400,
            y: 400,
            page: 4,
        });
        page.transitions.push(Transition {
            number: 0,
            condition: None,
            from: vec![7, 8],
            to: vec![9],
            page: 4,
            x: 0,
            y: 0,
        });
        let mut project = Project::new("hostile");
        project.sections.push(Section::sfc(2, "Broken", page));
        let mut app = EditorApp::new(project);
        assert!(
            !app.editor().problems().is_empty(),
            "the chart is reported as broken"
        );
        frame(&mut app);
        select(&mut app, Some(Selection::Step(0)));
        frame(&mut app);
        select(&mut app, Some(Selection::Transition(0)));
        frame(&mut app);
        // The document survives a chart that is deleted from under it.
        let mut state = view_of(&app);
        state.selection = Some(Selection::Step(99));
        set_view(state);
        frame(&mut app);
        assert_eq!(view().selection, None, "the dead selection is dropped");
        reset_view();
    }

    #[test]
    fn selecting_deleting_and_nudging_drive_the_model() {
        reset_view();
        let mut app = app();
        focus(&mut app, 3, Some(Selection::Step(1)));
        assert_eq!(view().selection, Some(Selection::Step(1)));
        assert_eq!(view().page, Some(3));

        nudge(&mut app, 1, 0);
        let moved = open_page(&app)
            .expect("a page")
            .step(1)
            .cloned()
            .expect("the step");
        assert_eq!((moved.x, moved.y), (1, 2));

        // A nudge that would walk off the grid is refused, not clamped silently
        // into another element's cell.
        nudge(&mut app, -1, 0);
        assert_eq!(
            open_page(&app)
                .expect("a page")
                .step(1)
                .map(|step| (step.x, step.y)),
            Some((0, 2))
        );

        delete_selection(&mut app);
        assert!(open_page(&app).expect("a page").step(1).is_none());
        assert_eq!(view().selection, None);
        assert!(app.editor.undo());
        assert!(open_page(&app).expect("a page").step(1).is_some());
        reset_view();
    }

    #[test]
    fn the_keyboard_map_arms_tools_and_edits_the_chart() {
        reset_view();
        let mut app = app();
        assert!(handle_key(&mut app, Key::S, Modifiers::default()));
        assert_eq!(view().tool, Some(SfcTool::Step));
        assert!(handle_key(&mut app, Key::I, Modifiers::default()));
        assert_eq!(view().tool, Some(SfcTool::InitialStep));
        assert!(handle_key(&mut app, Key::Escape, Modifiers::default()));
        assert_eq!(view().tool, None);
        assert!(!handle_key(&mut app, Key::F5, Modifiers::default()));

        select(&mut app, Some(Selection::Step(1)));
        assert!(handle_key(&mut app, Key::ArrowDown, Modifiers::default()));
        assert_eq!(
            open_page(&app).expect("a page").step(1).map(|s| (s.x, s.y)),
            Some((0, 3))
        );
        assert!(handle_key(&mut app, Key::ArrowUp, Modifiers::default()));
        assert_eq!(
            open_page(&app).expect("a page").step(1).map(|s| (s.x, s.y)),
            Some((0, 2))
        );
        // The transition at (0, 1) is in the way: the nudge is refused rather
        // than silently overwriting it.
        assert!(handle_key(&mut app, Key::ArrowUp, Modifiers::default()));
        assert_eq!(
            open_page(&app).expect("a page").step(1).map(|s| (s.x, s.y)),
            Some((0, 2))
        );
        assert!(handle_key(&mut app, Key::Delete, Modifiers::default()));
        assert!(open_page(&app).expect("a page").step(1).is_none());
        // A modified key is the platform's, not the document's.
        let command = Modifiers {
            command: true,
            ..Modifiers::default()
        };
        assert!(!handle_key(&mut app, Key::S, command));
        reset_view();
    }

    #[test]
    fn a_placed_chart_is_drawn_from_the_model_it_was_built_with() {
        reset_view();
        let mut app = app();
        let section = app.project().sections[1].id;
        // Placing through the same commands the document uses.
        app.editor
            .apply(insert_step_command(
                section,
                Step {
                    number: 5,
                    is_initial: false,
                    x: 2,
                    y: 4,
                    page: 3,
                },
            ))
            .expect("inserts");
        app.editor
            .insert_transition_linked(section, 2, 3, &[1], &[5])
            .expect("inserts a transition");
        // The link is derived, and the drawing follows the model.
        let page = open_page(&app).expect("a page");
        let transition = page.transition(1).expect("the transition");
        assert_eq!(transition.from, vec![1]);
        assert_eq!(transition.to, vec![5]);
        assert!(
            !is_and(transition),
            "one source and one target is an OR junction"
        );
        frame(&mut app);
        reset_view();
    }

    #[test]
    fn step_times_render_in_milliseconds() {
        let mut store = VarStore::with_default_sizes();
        assert_eq!(step_time(&store, 0), Some("0 ms".to_owned()));
        store
            .set(
                &VarRef::new(VarKind::Step, 3).with_accessor(Accessor::Value),
                Value::Word(420),
            )
            .expect("the store accepts the write");
        assert_eq!(step_time(&store, 3), Some("420 ms".to_owned()));
        assert!(!step_active(&store, 3));
        store
            .set(
                &VarRef::new(VarKind::Step, 3).with_accessor(Accessor::Activity),
                Value::Bit(true),
            )
            .expect("the store accepts the write");
        assert!(step_active(&store, 3));
        assert_eq!(step_time(&VarStore::new(), 0), None);
    }

    #[test]
    fn the_placeholder_of_an_empty_page_is_a_real_state() {
        reset_view();
        let mut project = Project::new("empty page");
        project
            .sections
            .push(Section::sfc(1, "Sequence", SequentialPage::new(0, "")));
        let mut app = EditorApp::new(project);
        frame(&mut app);
        arm(&mut app, Some(SfcTool::OrDivergence));
        frame(&mut app);
        reset_view();
    }

    #[test]
    fn the_palette_entries_are_all_reachable() {
        for entry in entries() {
            assert!(!entry.caption.is_empty());
            assert!(!entry.label.is_empty());
            assert!(!entry.tooltip.is_empty());
            assert_ne!(entry.letter, 'V', "V does nothing in the SFC document");
            assert_eq!(
                crate::palette::sfc_tool_for_key(entry.key),
                Some(entry.tool)
            );
            assert_eq!(entry.tool.entry().tool, entry.tool);
        }
        assert_eq!(entries().len(), SfcTool::ALL.len());
    }
}
