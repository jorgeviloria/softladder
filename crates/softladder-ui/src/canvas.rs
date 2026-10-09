//! The ladder document: a paper sheet, one network band per rung, IEC symbols
//! and the live indication.
//!
//! Every rung of the open section is drawn as one *network*, the way TIA Portal,
//! GX Works and Studio 5000 draw them: a numbered header with the rung title, its
//! comment and a state badge, and under it a wiring area with the power rail, the
//! cells of [`crate::layout`] and the IEC glyphs of [`crate::symbols`].
//!
//! All geometry is in *document points* (points at zoom `1.0`) and is mapped to
//! the screen by one [`Layout`] per network, so a pointer position still becomes
//! a cell through the mapping the layout tests exercise. All live values come
//! from the bench's engine store, never from wall-clock time, which is what keeps
//! the picture from flickering.

use egui::{
    Align2, Color32, CornerRadius, CursorIcon, FontFamily, FontId, Painter, Pos2, Rect, Sense,
    Shape, Stroke, StrokeKind, Ui, Vec2,
};
use softladder_core::{
    Accessor, Diagnostic, ElementKind, PlacedElement, Project, Rung, Severity, Value, VarStore,
};
use softladder_edit::Command;

use crate::app::{store, CentreTab, EditorApp, Tool};
use crate::design::{
    empty_state, Tokens, TypeScale, RADIUS_CARD, RADIUS_CONTROL, RADIUS_PILL, SPACE_2, SPACE_3,
};
use crate::layout::{self, Camera, Layout};
use crate::palette;
use crate::queries::{self, CellPower};
use crate::symbols::{self, Style};

/// Horizontal pitch of one grid column, in document points.
///
/// Wider than [`layout::BASE_CELL_W`] because a cell carries its tag name: a
/// vendor ladder is read by name, so a column has to be wide enough for one.
/// The pointer is still resolved through [`layout`]'s mapping, over the same
/// cells the canvas draws with — a screen test that drives a drag has to use
/// this pitch times the camera zoom.
pub(crate) const COL_PITCH: f32 = 96.0;

/// Height of the glyph area of one cell, in document points.
const GLYPH_H: f32 = 48.0;

/// Height of the label gutter above a cell's glyph, in document points.
///
/// The gutter holds the two label lines (tag name over address), so a parallel
/// branch's rows can sit next to each other without their labels colliding.
const LABEL_GUTTER: f32 = 32.0;

/// Vertical pitch of one grid row: the glyph plus its label gutter.
pub(crate) const ROW_PITCH: f32 = GLYPH_H + LABEL_GUTTER;

/// Band above a multi-row block where its instance label sits, in document
/// points. A block gives its first row to this label and starts its box below.
const BLOCK_LABEL_BAND: f32 = 17.0;

/// Extra room a network leaves under a block, whose box is one row pitch tall
/// per row the engine reads.
const BLOCK_OVERHANG: f32 = 20.0;

/// Blank space between a network header and its first row, in document points.
///
/// The labels of the first row end just above their cell, so they need a little
/// air under the header rather than sitting on the comment.
const GRID_TOP_PAD: f32 = 8.0;

/// Distance between the power rail and column zero, in document points.
const RAIL_GAP: f32 = 20.0;

/// Left offset of the wiring area inside the document, in document points.
const GRID_LEFT: f32 = 34.0;

/// Nominal width of a section document, in document points.
///
/// A network band spans the whole document, the way a TIA network box does, so
/// the header text and the comment have room to breathe.
const DOC_WIDTH: f32 = 1040.0;

/// Shortest document that still reads as a page, in document points.
const DOC_MIN_HEIGHT: f32 = 220.0;

/// Padding between the paper edge and the document, in screen points.
const PAPER_PAD: f32 = SPACE_3;

/// Margin between the panel edge and the paper, in screen points.
const PAPER_MARGIN: f32 = SPACE_3;

/// Height of the caption row of a network header, in document points.
const HEADER_CAPTION_H: f32 = 16.0;

/// Height of the title row of a network header, in document points.
const HEADER_TITLE_H: f32 = 19.0;

/// Height of one wrapped comment line, in document points.
const COMMENT_LINE_H: f32 = 15.0;

/// Vertical padding inside a network header, in document points.
const HEADER_PAD: f32 = 7.0;

/// Width reserved for the state badge in a network header, in document points.
const BADGE_W: f32 = 104.0;

/// Gap between two network bands, in document points.
const NETWORK_GAP: f32 = SPACE_2;

/// Height of the placeholder drawn for a network with no elements.
const EMPTY_NETWORK_H: f32 = 124.0;

/// Columns the wiring area offers, whatever the rung uses.
///
/// The band spans the whole document, so a click anywhere on the sheet lands on
/// a cell; the editor refuses a cell the model cannot hold.
const DOC_COLS: u8 = ((DOC_WIDTH - GRID_LEFT) / COL_PITCH) as u8;

/// Tallest zoom the automatic fit applies: a document is fitted, never magnified.
const FIT_MAX_ZOOM: f32 = 1.0;

/// Smallest zoom the automatic fit applies.
///
/// Fitting a long section into a small window would otherwise shrink the ladder
/// until nothing is readable; below this the document is framed on the selected
/// network instead and panned like every vendor tool.
const FIT_MIN_ZOOM: f32 = 0.8;

/// Draws the ladder document and turns pointer input into editor calls.
pub fn show(app: &mut EditorApp, ui: &mut Ui) {
    let (response, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
    let panel = response.rect;
    let tokens = app.tokens;

    painter.rect_filled(panel, CornerRadius::ZERO, tokens.surface);
    let paper = paper_rect(panel);
    paint_paper(&painter, paper, &tokens);

    // The empty states come first: with nothing to draw there is no camera to
    // fit and no cell to hit, and a blank sheet is not an interface.
    if app.project().sections.is_empty() {
        empty_paper(
            app,
            ui,
            &painter,
            paper,
            &tokens,
            EmptyState {
                title: "No program yet",
                body: "Create a section in the project tree, or open a project.",
                hint: "",
                add_rung: false,
            },
        );
        return;
    }
    // A rung that has disappeared (deleted, or undone away) cannot stay open.
    if app.selected_rung.is_some() && app.selected_rung_ref().is_none() {
        app.selected_rung = None;
        app.selection = None;
    }
    let networks = read_networks(app, &painter, &tokens);
    if networks.is_empty() {
        empty_paper(
            app,
            ui,
            &painter,
            paper,
            &tokens,
            EmptyState {
                title: "This section has no rungs",
                body: "Add a rung to start programming.",
                hint: "Use Insert / Rung, or the + next to Rungs in the project tree.",
                add_rung: true,
            },
        );
        return;
    }

    // The document is fitted whenever the open section changes, so a project
    // never opens at a random zoom with its ladder lost in a corner.
    let fit_id = egui::Id::new("softladder-canvas-fit");
    let key: (usize, Vec<u32>) = (
        app.selected_section,
        networks.iter().map(|network| network.rung.id).collect(),
    );
    let fitted: Option<(usize, Vec<u32>)> = ui.ctx().data(|data| data.get_temp(fit_id));
    if fitted.as_ref() != Some(&key) || app.camera.is_default() {
        let base = paper.min + Vec2::splat(PAPER_PAD);
        let viewport = paper.shrink(PAPER_PAD);
        let focus = app
            .selected_rung
            .and_then(|id| networks.iter().find(|network| network.rung.id == id))
            .map(|network| network.band());
        app.camera = fit_camera(document_bounds(&networks), viewport, base, focus);
        ui.ctx().data_mut(|data| data.insert_temp(fit_id, key));
    }

    let space = ui.input(|input| input.key_down(egui::Key::Space));
    handle_zoom(app, ui, &response, paper);
    handle_pan(app, &response, space);
    let view = View::new(paper, app.camera);

    // Everything below is read-only state for one frame, gathered before any
    // edit so painting can borrow the project and the store.
    let problems = app.editor.problems().to_vec();
    let powered: Vec<Vec<CellPower>> = networks
        .iter()
        .map(|network| queries::power_map(&network.rung, store(app)))
        .collect();
    // A live picture is drawn once the engine is scanning or has scanned: the
    // values come from the store, never from the clock, so nothing flickers.
    let live = app.bench().state().is_scanning() || app.bench().cycles() > 0;
    let scanned = app.bench().cycles() > 0;
    let hover_pos = response.hover_pos();
    let hover = hover_pos.and_then(|pos| cell_hit(&networks, view, pos));
    let selected = networks
        .iter()
        .position(|network| Some(network.rung.id) == app.selected_rung);
    let armed = match app.tool {
        Tool::Place(kind) => Some(kind),
        Tool::Select => None,
    };
    // The ghost follows the pointer, and falls back to the keyboard cursor so an
    // armed tool is always visible.
    let ghost = armed.and(hover.or_else(|| {
        selected
            .zip(app.selection)
            .map(|(net, (col, row))| (net, col, row))
    }));

    let frame = Frame {
        app,
        powered: &powered,
        problems: &problems,
        live,
        scanned,
        hover,
        hover_pos,
        selected,
        selected_cell: app.selection,
        ghost,
        armed,
    };
    let clipping = painter.with_clip_rect(paper.shrink(1.0));
    paint_grid(&clipping, paper, view, &networks, &tokens);
    for (index, network) in networks.iter().enumerate() {
        paint_network(&clipping, view, network, index, &tokens, &frame);
    }

    handle_pointer(app, ui, &response, &networks, view, space, hover);
    handle_menu(app, ui, &response, &networks, view, hover);
    inline_editor(app, ui, &networks, view);
    expression_tooltip(&response, &networks, hover);
}

/// The paper rectangle: the panel inset by a margin, which is where the
/// document lives and everything outside it is the surrounding surface.
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

/// Draws the engineering grid on the paper, aligned with the document origin.
fn paint_grid(painter: &Painter, paper: Rect, view: View, networks: &[Network], tokens: &Tokens) {
    let cell = Vec2::new(COL_PITCH * view.zoom, ROW_PITCH * view.zoom);
    if cell.x < 7.0 || cell.y < 7.0 || !cell.x.is_finite() || !cell.y.is_finite() {
        return;
    }
    let stroke = Stroke::new(1.0_f32, tokens.paper_grid);
    let mut x = view.origin.x;
    let mut guard = 0;
    while x <= paper.right() && guard < 512 {
        if x >= paper.left() {
            painter.vline(x, paper.y_range(), stroke);
        }
        x += cell.x;
        guard += 1;
    }
    let bottom = networks
        .last()
        .map_or(view.origin.y, |network| view.point(0.0, network.bottom()).y);
    let mut y = view.origin.y;
    let mut guard = 0;
    while y <= paper.bottom() && guard < 512 {
        if y >= paper.top() && y <= bottom + cell.y {
            painter.hline(paper.x_range(), y, stroke);
        }
        y += cell.y;
        guard += 1;
    }
}

/// One network band of the open section, in document points.
struct Network {
    /// The rung drawn in this band.
    rung: Rung,
    /// One-based number shown in the header.
    number: usize,
    /// Top of the band, in document points.
    top: f32,
    /// Height of the band, in document points.
    height: f32,
    /// Grid rows, counting every row a function block occupies.
    rows: u8,
    /// Grid columns.
    cols: u8,
    /// Number of wrapped comment lines.
    comment_lines: usize,
}

impl Network {
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
        header_height(!self.rung.label.trim().is_empty(), self.comment_lines)
    }

    /// The header as a document rectangle.
    fn header(&self) -> Rect {
        let band = self.band();
        Rect::from_min_size(band.min, Vec2::new(band.width(), self.header_height()))
    }

    /// Top of the wiring area, in document points.
    ///
    /// The first row's tag name and address are drawn *above* its glyph, in the
    /// label gutter, so the wiring area starts a gutter below the header or the
    /// labels would collide with the comment.
    fn grid_top(&self) -> f32 {
        self.top + self.header_height() + GRID_TOP_PAD + LABEL_GUTTER
    }
}

/// The rectangles of one network header.
#[derive(Debug, Clone, Copy, PartialEq)]
struct HeaderRects {
    /// `Network <n>` in the caption style.
    caption: Rect,
    /// The rung label, empty when the rung has none.
    title: Rect,
    /// The wrapped comment.
    comment: Rect,
    /// The state badge, right-aligned in the caption row.
    badge: Rect,
}

/// Height of a network header with `comment_lines` wrapped comment lines.
fn header_height(has_title: bool, comment_lines: usize) -> f32 {
    let title = if has_title { HEADER_TITLE_H } else { 0.0 };
    HEADER_PAD * 2.0 + HEADER_CAPTION_H + title + comment_lines as f32 * COMMENT_LINE_H
}

/// Splits a network header into the rectangles its text is drawn in.
///
/// The caption row carries `Network <n>` on the left and the state badge on the
/// right; the title is under it and the comment under the title, both wrapped to
/// the header's width.
fn header_rects(header: Rect, has_title: bool, comment_lines: usize) -> HeaderRects {
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
    let mut y = top + HEADER_CAPTION_H;
    let title = if has_title {
        Rect::from_min_size(
            Pos2::new(left, y),
            Vec2::new((right - left).max(0.0), HEADER_TITLE_H),
        )
    } else {
        Rect::NOTHING
    };
    if has_title {
        y += HEADER_TITLE_H;
    }
    let comment = Rect::from_min_size(
        Pos2::new(left, y),
        Vec2::new(
            (right - left).max(0.0),
            comment_lines as f32 * COMMENT_LINE_H,
        ),
    );
    HeaderRects {
        caption,
        title,
        comment,
        badge,
    }
}

/// Reads every rung of the open section into the bands it is drawn in.
fn read_networks(app: &EditorApp, painter: &Painter, tokens: &Tokens) -> Vec<Network> {
    let ids = queries::section_rungs(app.project(), app.selected_section);
    let comment_font = FontId::new(TypeScale::CAPTION, FontFamily::Proportional);
    let wrap = (DOC_WIDTH - SPACE_3 * 2.0).max(80.0);
    let mut networks = Vec::with_capacity(ids.len());
    let mut top = 0.0;
    for (index, id) in ids.iter().enumerate() {
        let Some(rung) = app.project().rung(*id) else {
            continue;
        };
        let comment_lines = if rung.comment.trim().is_empty() {
            0
        } else {
            painter
                .layout(
                    rung.comment.clone(),
                    comment_font.clone(),
                    tokens.text_dim,
                    wrap,
                )
                .rows
                .len()
                .clamp(1, 6)
        };
        // The grid is what the user can point at, so it always offers room to
        // place an element: one empty row, and the columns the sheet has.
        let rows = queries::rung_rows(rung)
            .max(2)
            .min(layout::MAX_ROW.saturating_add(1));
        let cols = queries::rung_cols(rung)
            .saturating_add(2)
            .max(DOC_COLS)
            .min(layout::MAX_COL.saturating_add(1));
        let has_block = rung.elements.iter().any(|element| is_block(element.kind));
        let height = header_height(!rung.label.trim().is_empty(), comment_lines)
            + GRID_TOP_PAD
            + if rung.elements.is_empty() {
                EMPTY_NETWORK_H
            } else {
                f32::from(rows) * ROW_PITCH + SPACE_2
            }
            + if has_block { BLOCK_OVERHANG } else { 0.0 };
        networks.push(Network {
            rung: rung.clone(),
            number: index + 1,
            top,
            height,
            rows,
            cols,
            comment_lines,
        });
        top += height + NETWORK_GAP;
    }
    networks
}

/// The bounding box of every network, in document points.
fn document_bounds(networks: &[Network]) -> Rect {
    let height = networks
        .last()
        .map_or(DOC_MIN_HEIGHT, |network| network.bottom())
        .max(DOC_MIN_HEIGHT);
    Rect::from_min_size(Pos2::ZERO, Vec2::new(DOC_WIDTH, height))
}

/// The camera that frames a section in `viewport`, with the document origin at
/// `base`.
///
/// A section that fits is shown whole, centred, at `1.0` at most: a document is
/// fitted to the window, not blown up until its symbols fill the screen. A
/// section taller than the window is framed on `focus` (the selected network)
/// at [`FIT_MIN_ZOOM`], the zoom below which a ladder stops being readable, and
/// panned from there — which is what every vendor tool does with a long block.
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
        // A document that fits starts at the top of the page, the way a
        // document does, and the sheet below it stays empty.
        content.top() + half
    } else {
        let wanted = focus.map_or(content.center().y, |rect| rect.center().y);
        wanted.clamp(content.top() + half, content.bottom() - half)
    };
    // The document is left-aligned against the sheet: a narrow window shows the
    // rail and the first columns rather than the middle of the network.
    let pan = Vec2::new(
        viewport.left() - base.x - content.left() * zoom,
        viewport.center().y - base.y - centre_y * zoom,
    );
    Camera { zoom, pan }
}

/// The screen mapping of the document: a pan/zoom camera over document points.
#[derive(Debug, Clone, Copy)]
struct View {
    /// Screen position of document point `(0, 0)`, the pan included.
    origin: Pos2,
    /// Zoom factor.
    zoom: f32,
}

impl View {
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

    /// The cell layout of a network whose wiring area starts at `top`.
    fn layout(&self, top: f32) -> Layout {
        Layout::with_cell(
            self.point(GRID_LEFT, top),
            Vec2::new(COL_PITCH * self.zoom, ROW_PITCH * self.zoom),
        )
    }
}

/// Read-only state shared by every network of one frame.
struct Frame<'a> {
    /// The editor the project, the symbols and the live values come from.
    app: &'a EditorApp,
    /// Power flow per network, by network index.
    powered: &'a [Vec<CellPower>],
    /// Diagnostics of the whole project.
    problems: &'a [Diagnostic],
    /// `true` while the engine has a live picture to show.
    live: bool,
    /// `true` once the engine has scanned at least once.
    ///
    /// An engine that has never run has every function block at its initial
    /// state (a counter with preset zero reads *done*), which would light the
    /// whole drawing up before the first scan.
    scanned: bool,
    /// The cell the pointer is over: network index, column, row.
    hover: Option<(usize, u8, u8)>,
    /// Where the pointer is, for the header highlights.
    hover_pos: Option<Pos2>,
    /// Index of the network the selection belongs to.
    selected: Option<usize>,
    /// The selected cell, in its network's grid.
    selected_cell: Option<(u8, u8)>,
    /// Where the ghost of an armed palette tool is drawn.
    ghost: Option<(usize, u8, u8)>,
    /// The element kind the palette has armed, if any.
    armed: Option<ElementKind>,
}

impl Frame<'_> {
    /// The kind the armed palette tool would place.
    fn armed_kind(&self) -> Option<ElementKind> {
        self.armed
    }

    /// `true` when every element shows its address as well as its tag.
    fn show_addresses(&self) -> bool {
        self.app.show_addresses
    }

    /// The project the symbols are resolved from.
    fn project(&self) -> &Project {
        self.app.project()
    }

    /// The running engine's variable store.
    fn store(&self) -> &VarStore {
        store(self.app)
    }
}

/// The state of one network's badge.
struct Badge {
    /// Text of the badge.
    text: String,
    /// Colour of the badge.
    colour: Color32,
}

/// The segement of a vertical link: from the cell above down into this one.
///
/// The link joins the *left sides* of the two cells (`docs/SEMANTICS.md` §2), so
/// the stub is drawn at the left edge of the lower cell, where the power that
/// merges arrives.
fn vertical_stub(upper: Rect, lower: Rect) -> Option<[Pos2; 2]> {
    if !upper.is_positive() || !lower.is_positive() {
        return None;
    }
    let x = lower.left();
    Some([
        Pos2::new(x, upper.center().y),
        Pos2::new(x, lower.center().y),
    ])
}

/// The ghost outline of the armed tool on a cell: one point inside the cell, so
/// the real cell stays visible under it.
fn ghost_rect(cell: Rect) -> Rect {
    cell.shrink(1.0)
}

/// The glyph area of a cell: everything below its label gutter.
fn glyph_rect(cell: Rect, zoom: f32) -> Rect {
    let top = (cell.top() + LABEL_GUTTER * zoom).min(cell.bottom());
    Rect::from_min_max(Pos2::new(cell.left(), top), cell.max)
}

/// The rectangle a function block's box is drawn in.
///
/// A single-cell block keeps its own cell, grown around the wire so it stays
/// connected to the row; a block that reads several rows is given the rectangle
/// that puts its pins on the wires of those rows.
fn block_rect(grid: Layout, col: u8, row: u8, span: u8, zoom: f32) -> Rect {
    let cells = span_cell(grid, col, row, span);
    let first_wire = glyph_rect(layout::cell_rect(col, row, grid), zoom)
        .center()
        .y;
    if span <= 1 {
        let top = cells.top() + BLOCK_LABEL_BAND * zoom;
        let bottom = 2.0 * first_wire - top;
        return Rect::from_min_max(
            Pos2::new(cells.left(), top),
            Pos2::new(cells.right(), bottom.max(top + 1.0)),
        );
    }
    symbols::aligned_block_rect(
        cells.left(),
        cells.width(),
        first_wire,
        span,
        ROW_PITCH * zoom,
    )
}

/// The screen rectangle of the cells a block of `span` rows occupies.
fn span_cell(grid: Layout, col: u8, row: u8, span: u8) -> Rect {
    let last = row
        .saturating_add(span.saturating_sub(1))
        .min(layout::MAX_ROW);
    let first_cell = layout::cell_rect(col, row, grid);
    let last_cell = layout::cell_rect(col, last, grid);
    Rect::from_min_max(first_cell.min, last_cell.max)
}

/// The cell under `pos` inside a network's own grid, if any.
fn cell_hit(networks: &[Network], view: View, pos: Pos2) -> Option<(usize, u8, u8)> {
    for (index, network) in networks.iter().enumerate() {
        let grid = view.layout(network.grid_top());
        if let Some((col, row)) = layout::cell_at(pos, grid) {
            if col < network.cols && row < network.rows {
                return Some((index, col, row));
            }
        }
    }
    None
}

/// The *element* cell under `pos`, counting the rows a block occupies, so a
/// click on any row of a counter selects and drags the counter itself.
fn element_hit(networks: &[Network], view: View, pos: Pos2) -> Option<(usize, u8, u8)> {
    let (index, col, row) = cell_hit(networks, view, pos)?;
    let network = networks.get(index)?;
    let element = queries::element_at_cell(&network.rung, col, row)?;
    Some((index, element.col, element.row))
}

/// The network band under `pos`, if any.
fn band_hit(networks: &[Network], view: View, pos: Pos2) -> Option<usize> {
    networks
        .iter()
        .position(|network| view.rect(network.band()).contains(pos))
}

/// The badge of a network: errors first, then warnings, then `ok`.
fn network_badge(tokens: &Tokens, project: &Project, problems: &[Diagnostic], rung: u32) -> Badge {
    let mut errors = 0usize;
    let mut warnings = 0usize;
    for diagnostic in problems {
        if queries::problem_target(project, diagnostic).map(|target| target.rung) != Some(rung) {
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

/// Draws one network: its header band and its wiring area.
fn paint_network(
    painter: &Painter,
    view: View,
    network: &Network,
    index: usize,
    tokens: &Tokens,
    frame: &Frame<'_>,
) {
    let band = view.rect(network.band());
    let header = view.rect(network.header());
    let has_title = !network.rung.label.trim().is_empty();
    let badge = network_badge(tokens, frame.project(), frame.problems, network.rung.id);
    let selected = frame.selected == Some(index);
    let hovered = frame.hover_pos.is_some_and(|pos| header.contains(pos));

    if selected {
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

    let rects = header_rects(header, has_title, network.comment_lines);
    painter.text(
        rects.caption.left_center(),
        Align2::LEFT_CENTER,
        format!("Network {}", network.number),
        view.font(TypeScale::CAPTION, FontFamily::Proportional),
        tokens.text_dim,
    );
    paint_badge(painter, view, rects.badge, &badge);
    if has_title {
        painter.text(
            rects.title.left_center(),
            Align2::LEFT_CENTER,
            &network.rung.label,
            view.font(TypeScale::EMPHASIS, FontFamily::Proportional),
            tokens.text,
        );
    }
    if network.comment_lines > 0 && rects.comment.is_positive() {
        let galley = painter.layout(
            network.rung.comment.clone(),
            view.font(TypeScale::CAPTION, FontFamily::Proportional),
            tokens.text_dim,
            rects.comment.width(),
        );
        painter.galley(rects.comment.left_top(), galley, tokens.text_dim);
    }
    painter.hline(band.x_range(), band.bottom(), tokens.hairline());

    // The left edge of the band: the selection marker, or the error marker.
    let bar = Rect::from_min_size(band.min, Vec2::new(3.0, band.height()));
    if selected {
        painter.rect_filled(bar, CornerRadius::ZERO, tokens.accent);
    } else if badge.colour == tokens.error {
        painter.rect_filled(bar, CornerRadius::ZERO, tokens.error);
    }

    let grid = view.layout(network.grid_top());
    paint_wires(painter, view, network, index, grid, tokens, frame);
    for element in &network.rung.elements {
        if element.col >= network.cols || element.row >= network.rows {
            continue;
        }
        let span = queries::block_span(element.kind);
        let cell = span_cell(grid, element.col, element.row, span);
        let is_block = is_block(element.kind);
        let glyph = if is_block {
            block_rect(grid, element.col, element.row, span, view.zoom)
        } else {
            glyph_rect(cell, view.zoom)
        };
        // A block names itself in one line above its box; everything else hangs
        // its tag and address in the cell's gutter.
        let label = if is_block {
            Rect::from_min_size(
                Pos2::new(cell.left(), cell.top() + BLOCK_LABEL_BAND * view.zoom),
                cell.size(),
            )
        } else {
            glyph
        };
        let state = queries::cell_state(powered_for(frame, index), element.col, element.row);
        paint_element(painter, view, tokens, frame, element, (glyph, label), state);
    }
    paint_problem_cells(painter, view, network, grid, tokens, frame);
    if network.rung.elements.is_empty() {
        paint_empty_network(painter, view, network, grid, tokens, frame);
    }
    // The ghost of the armed palette tool, drawn where a click would place it.
    if frame.ghost.is_some_and(|(ghost, _, _)| ghost == index) {
        if let (Some(kind), Some((_, col, row))) = (frame.armed, frame.ghost) {
            paint_ghost(painter, view, tokens, kind, grid, col, row);
        }
    }
    // The hover highlight of the cell under the pointer, and of a block as a
    // whole, with the selection outline above it.
    if let Some((hovered_network, col, row)) = frame.hover {
        if hovered_network == index {
            let span = queries::element_at_cell(&network.rung, col, row)
                .map_or(1, |element| queries::block_span(element.kind));
            let cell = span_cell(grid, col, row, span);
            painter.rect_filled(
                cell,
                CornerRadius::same(RADIUS_PILL),
                tokens.accent.gamma_multiply(0.08),
            );
            painter.rect_stroke(
                cell,
                CornerRadius::same(RADIUS_PILL),
                tokens.focus(),
                StrokeKind::Inside,
            );
        }
    }
    if selected {
        if let Some((col, row)) = frame.selected_cell {
            if let Some(element) = queries::element_at_cell(&network.rung, col, row) {
                let span = queries::block_span(element.kind);
                let cell = glyph_rect(span_cell(grid, element.col, element.row, span), view.zoom);
                painter.rect_stroke(
                    cell,
                    CornerRadius::same(RADIUS_PILL),
                    Stroke::new(2.0_f32, tokens.accent),
                    StrokeKind::Outside,
                );
            }
        }
    }
}

/// Draws the faded preview of the element an armed tool would place.
fn paint_ghost(
    painter: &Painter,
    view: View,
    tokens: &Tokens,
    kind: ElementKind,
    grid: Layout,
    col: u8,
    row: u8,
) {
    let span = queries::block_span(kind);
    let cell = span_cell(grid, col, row, span);
    let outline = ghost_rect(cell);
    if !outline.is_positive() {
        return;
    }
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
    let glyph = glyph_rect(cell, view.zoom);
    if !glyph.is_positive() {
        return;
    }
    let style = Style {
        wire: Stroke::new(2.0_f32, tokens.accent.gamma_multiply(0.6)),
        symbol: Stroke::new(2.0_f32, tokens.accent.gamma_multiply(0.8)),
        fill: tokens.accent.gamma_multiply(0.10),
        text: tokens.accent,
    };
    match kind {
        ElementKind::ContactNo => symbols::contact_no(painter, glyph, style),
        ElementKind::ContactNc => symbols::contact_nc(painter, glyph, style),
        ElementKind::ContactRising => symbols::contact_rising(painter, glyph, style),
        ElementKind::ContactFalling => symbols::contact_falling(painter, glyph, style),
        ElementKind::Connection => symbols::wire(painter, glyph, style),
        ElementKind::Timer { .. } | ElementKind::Counter { .. } | ElementKind::Register { .. } => {
            symbols::block(painter, glyph, style, describe(kind), &block_pins(kind))
        }
        ElementKind::Compare | ElementKind::Operate => {
            symbols::expression_box(painter, glyph, style, "%MW0 = 0")
        }
        other => {
            if let Some(coil) = symbols::coil_of(other) {
                symbols::coil(painter, glyph, style, coil);
            }
        }
    }
}

/// The power flow of a network, or an empty slice when it has none.
fn powered_for<'a>(frame: &'a Frame<'_>, index: usize) -> &'a [CellPower] {
    frame.powered.get(index).map_or(&[], Vec::as_slice)
}

/// Draws the rail, the wires of every live row and the vertical branch stubs.
fn paint_wires(
    painter: &Painter,
    view: View,
    network: &Network,
    index: usize,
    grid: Layout,
    tokens: &Tokens,
    frame: &Frame<'_>,
) {
    let power = powered_for(frame, index);
    let implicit = network.rung.wire_mode != softladder_core::model::WireMode::Explicit;
    let zoom = view.zoom;

    // The power rail: a bus left of column zero with one stub per row that
    // touches it, so an open first contact still shows the rail is live.
    let rail_x = grid.origin.x - RAIL_GAP * zoom;
    let mut rail: Option<(f32, f32)> = None;
    let mut rail_live = false;
    for row in 0..network.rows {
        if queries::element_at_cell(&network.rung, 0, row).is_none() {
            continue;
        }
        let y = glyph_rect(layout::cell_rect(0, row, grid), zoom).center().y;
        let state = queries::cell_state(power, 0, row);
        let live = frame.live && state.fed;
        rail_live |= live;
        painter.hline(
            rail_x..=grid.origin.x,
            y,
            Style::idle(tokens).with_live(tokens, live).wire,
        );
        rail = Some(match rail {
            Some((top, bottom)) => (top.min(y), bottom.max(y)),
            None => (y, y),
        });
    }
    if let Some((top, bottom)) = rail {
        let style = Style::idle(tokens).with_live(tokens, rail_live);
        painter.vline(rail_x, top..=bottom, Stroke::new(3.0_f32, style.wire.color));
    }

    // The horizontal wires. A cell that holds an element draws its own wire;
    // an empty one conducts when the row is live and the wiring is implicit.
    for row in 0..network.rows {
        let mut first: Option<u8> = None;
        let mut last: Option<u8> = None;
        for element in &network.rung.elements {
            let start = usize::from(element.row);
            let end = start + usize::from(queries::block_span(element.kind));
            if usize::from(row) >= start && usize::from(row) < end {
                first = Some(first.map_or(element.col, |col| col.min(element.col)));
                last = Some(last.map_or(element.col, |col| col.max(element.col)));
            }
        }
        let (Some(first), Some(last)) = (first, last) else {
            continue;
        };
        for col in first..=last {
            let cell = layout::cell_rect(col, row, grid);
            let glyph = glyph_rect(cell, zoom);
            let state = queries::cell_state(power, col, row);
            let style = Style::idle(tokens).with_live(tokens, frame.live && state.fed);
            match queries::element_at_cell(&network.rung, col, row) {
                None => {
                    if implicit {
                        symbols::wire(painter, glyph, style);
                    }
                }
                Some(element) if element.row < row => {
                    // A row of a function block reads its input here: run the
                    // wire into the box, which owns the labelled pins.
                    let span = queries::block_span(element.kind);
                    let block = glyph_rect(span_cell(grid, element.col, element.row, span), zoom);
                    let body = symbols::block_body(block);
                    painter.hline(glyph.left()..=body.left(), glyph.center().y, style.wire);
                }
                Some(_) => {}
            }
        }
    }

    // The vertical links: a real stub from the row above into this cell, which
    // is what makes a parallel branch and its merge point read as a branch.
    for element in &network.rung.elements {
        if !element.connected_with_top || element.row == 0 {
            continue;
        }
        let upper = glyph_rect(layout::cell_rect(element.col, element.row - 1, grid), zoom);
        let lower = glyph_rect(layout::cell_rect(element.col, element.row, grid), zoom);
        let Some([from, to]) = vertical_stub(upper, lower) else {
            continue;
        };
        let state = queries::cell_state(power, element.col, element.row);
        let above = queries::cell_state(power, element.col, element.row - 1);
        let live = frame.live && (state.fed || state.live || above.live);
        painter.line_segment([from, to], Style::idle(tokens).with_live(tokens, live).wire);
    }
}

/// Draws one element's glyph, its label and its live value.
fn paint_element(
    painter: &Painter,
    view: View,
    tokens: &Tokens,
    frame: &Frame<'_>,
    element: &PlacedElement,
    shapes: (Rect, Rect),
    state: CellPower,
) {
    let (glyph, label_anchor) = shapes;
    if !glyph.is_positive() {
        return;
    }
    let live = frame.live && state.live;
    let style = Style::idle(tokens).with_live(tokens, live);
    match element.kind {
        ElementKind::ContactNo => symbols::contact_no(painter, glyph, style),
        ElementKind::ContactNc => symbols::contact_nc(painter, glyph, style),
        ElementKind::ContactRising => symbols::contact_rising(painter, glyph, style),
        ElementKind::ContactFalling => symbols::contact_falling(painter, glyph, style),
        ElementKind::Connection => symbols::wire(painter, glyph, style),
        ElementKind::Timer { .. } | ElementKind::Counter { .. } | ElementKind::Register { .. } => {
            // Before the first scan a block's stored output is meaningless (a
            // counter with preset zero already reads *done*), so its box only
            // goes live once the engine has run.
            let style = Style::idle(tokens).with_live(tokens, live && frame.scanned);
            symbols::block(
                painter,
                glyph,
                style,
                describe(element.kind),
                &block_pins(element.kind),
            );
            if frame.live {
                if let Some(value) = block_value_text(frame, element) {
                    symbols::block_value(painter, glyph, style, &value);
                }
            }
        }
        ElementKind::Compare | ElementKind::Operate => {
            let text = elide(
                painter,
                &element.params.join(" "),
                FontId::new(
                    (glyph.height() * 0.24).clamp(6.0, 11.0),
                    FontFamily::Monospace,
                ),
                tokens.text_dim,
                glyph.width() * 0.86,
            );
            symbols::expression_box(painter, glyph, style.with_text(tokens.text), &text);
        }
        other => {
            if let Some(coil) = symbols::coil_of(other) {
                symbols::coil(painter, glyph, style, coil);
            }
        }
    }

    let (name, address) = label_texts(frame.project(), element);
    let label = if is_block(element.kind) {
        // A block names itself in one line: its tag, or the instance, which is
        // the name a user gives a timer or a counter.
        let text = match block_parameter_text(element) {
            Some(parameter) => {
                let name = name.unwrap_or(address.as_str());
                format!("{name} · {parameter}")
            }
            None => name.unwrap_or(address.as_str()).to_owned(),
        };
        symbols::element_label_rect(painter, label_anchor, tokens, Some(&text), "", false)
    } else if frame.show_addresses() {
        // The View toggle drops the tag names and shows the address alone.
        symbols::element_label_rect(painter, label_anchor, tokens, None, &address, true)
    } else {
        // The tag name over its address; an element without a tag shows the
        // address alone.
        symbols::element_label_rect(
            painter,
            label_anchor,
            tokens,
            name,
            &address,
            name.is_some(),
        )
    };
    if frame.live {
        if let Some(value) = chip_text(element, frame.store()) {
            paint_value_chip(painter, view, tokens, label, &value, live);
        }
    }
}

/// Draws the value chip of a bit element just after its label.
fn paint_value_chip(
    painter: &Painter,
    view: View,
    tokens: &Tokens,
    label: Rect,
    value: &str,
    live: bool,
) {
    if !label.is_positive() {
        return;
    }
    let font = view.font(TypeScale::CAPTION, FontFamily::Monospace);
    let colour = if live {
        tokens.energised
    } else {
        tokens.text_dim
    };
    let size = painter
        .layout_no_wrap(value.to_owned(), font, colour)
        .size()
        + Vec2::new(6.0, 2.0);
    let anchor = Pos2::new(
        label.right() + 3.0 + size.x / 2.0,
        label.bottom() - size.y / 2.0,
    );
    symbols::value_chip(painter, anchor, tokens, value, live);
}

/// The live value of a contact or a coil, as `1` or `0`.
///
/// A function block carries its reading inside its own box and an expression
/// block has no bit of its own, so neither gets a chip.
fn chip_text(element: &PlacedElement, store: &VarStore) -> Option<String> {
    if !matches!(
        element.kind,
        ElementKind::ContactNo
            | ElementKind::ContactNc
            | ElementKind::ContactRising
            | ElementKind::ContactFalling
            | ElementKind::CoilOut
            | ElementKind::CoilOutNeg
            | ElementKind::CoilSet
            | ElementKind::CoilReset
    ) {
        return None;
    }
    let value = element.var.as_ref().and_then(|var| store.get(var))?;
    Some(if value.as_bool() {
        "1".to_owned()
    } else {
        "0".to_owned()
    })
}

/// The live value of a function block: `ET/PT`, `CV/PV` or `count/capacity`.
///
/// Until the engine has configured the block, its stored preset is still zero,
/// so the preset comes from the element's parameter instead — which is what the
/// engine will load on its first scan, expressed in the same units.
fn block_value_text(frame: &Frame<'_>, element: &PlacedElement) -> Option<String> {
    let read = |accessor: Accessor| {
        queries::value_text(crate::app::block_of(frame.app, element, accessor))
    };
    let stored = |accessor: Accessor| crate::app::block_of(frame.app, element, accessor);
    let number = |value: Option<Value>| queries::value_text(value);
    match element.kind {
        ElementKind::Timer { .. } => {
            let preset = stored(Accessor::Preset)
                .filter(|value| value.as_i64() != 0)
                .or_else(|| parameter_units(element));
            Some(format!("{}/{}", read(Accessor::Value), number(preset)))
        }
        ElementKind::Counter { .. } => {
            let preset = stored(Accessor::Preset)
                .filter(|value| value.as_i64() != 0)
                .or_else(|| {
                    element
                        .params
                        .first()
                        .and_then(|parameter| parameter.trim().parse::<i32>().ok())
                        .map(Value::Word)
                });
            Some(format!("{}/{}", read(Accessor::Value), number(preset)))
        }
        ElementKind::Register { .. } => {
            let capacity = element
                .params
                .first()
                .cloned()
                .unwrap_or_else(|| "—".to_owned());
            Some(format!("{}/{}", read(Accessor::Count), capacity))
        }
        _ => None,
    }
}

/// A timer parameter as a preset in the time-base units the engine counts in.
///
/// Mirrors the engine's rule for a literal parameter: a plain number is a
/// duration in milliseconds on the 100 ms base, and an `s`/`m` suffix picks the
/// second or minute base. A parameter that names a variable is left to the
/// store, whose preset is correct as soon as the engine has scanned.
fn parameter_units(element: &PlacedElement) -> Option<Value> {
    let parameter = element.params.first()?.trim().to_owned();
    // A suffixed literal is already a duration in that unit, so `3s` is three
    // units on the one-second base; a plain number is milliseconds on the
    // 100 ms base, which is how the engine reads a literal preset.
    let units = if let Some(rest) = parameter.strip_suffix(['s', 'S']) {
        rest.trim().parse::<i64>().ok()?
    } else if let Some(rest) = parameter.strip_suffix(['m', 'M']) {
        rest.trim().parse::<i64>().ok()?
    } else {
        parameter.parse::<i64>().ok()? / 100
    };
    let units = units.max(i64::from(units > 0));
    Some(Value::Word(
        units.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
    ))
}

/// The tag name and address drawn above an element.
///
/// A block shows its instance (`%TM0`), not the sub-value its variable may
/// carry, because the instance is what the user named.
fn label_texts<'a>(project: &'a Project, element: &PlacedElement) -> (Option<&'a str>, String) {
    let name = element
        .var
        .as_ref()
        .and_then(|var| queries::symbol_for(project, var))
        .map(|symbol| symbol.name.as_str());
    let address = element.var.as_ref().map_or_else(String::new, |var| {
        if is_block(element.kind) {
            instance_text(var)
        } else {
            var.to_string()
        }
    });
    (name, address)
}

/// The instance of a timer, counter or register: `%TM0`, `%C1`, `%R2`.
///
/// The canonical spelling of a bare `%TM0` prints its implied `.Q`, which is a
/// sub-value, not the instance the user named, so the instance is rebuilt here.
fn instance_text(var: &softladder_core::VarRef) -> String {
    match &var.index_expr {
        Some(index) => format!("%{}[{index}]", var.kind.mnemonic()),
        None => format!("%{}{}", var.kind.mnemonic(), var.index),
    }
}

/// `true` for the element kinds drawn as a function block box.
fn is_block(kind: ElementKind) -> bool {
    matches!(
        kind,
        ElementKind::Timer { .. } | ElementKind::Counter { .. } | ElementKind::Register { .. }
    )
}

/// The pins of a function block: its **inputs** on the left, its readouts on the
/// right.
///
/// The inputs are not listed here — they are [`ElementKind::input_pins`], the
/// same table the engine takes its block span from. That is the point: a pin is
/// only drawn where the engine actually reads a wire, so the drawing can never
/// advertise a connection that does nothing. A timer therefore shows a single
/// input (`IN`); its preset is a *parameter* (`%TM0.P`), drawn as a parameter by
/// [`block_parameter_text`] rather than as a pin.
fn block_pins(kind: ElementKind) -> Vec<(&'static str, i32)> {
    let mut pins: Vec<(&'static str, i32)> =
        kind.input_pins().iter().map(|label| (*label, -1)).collect();
    pins.extend(block_readouts(kind).iter().map(|label| (*label, 1)));
    pins
}

/// The labels drawn on the right of a block: the flow out and its readouts.
fn block_readouts(kind: ElementKind) -> &'static [&'static str] {
    match kind {
        ElementKind::Timer { .. } => &["Q", "ET"],
        ElementKind::Counter { .. } => &["Q", "CV"],
        ElementKind::Register { .. } => &["E", "F"],
        _ => &[],
    }
}

/// A block's parameter, as it is drawn beside the instance name.
///
/// The preset of a timer or a counter and the capacity of a register are
/// parameters, not wires: they are set in the inspector (or typed as `3s`,
/// `%MW10`), so the canvas labels them `PT 3s`, `PV 5`, `N 8`.
fn block_parameter_text(element: &PlacedElement) -> Option<String> {
    let parameter = element.params.first()?.trim();
    if parameter.is_empty() {
        return None;
    }
    let label = match element.kind {
        ElementKind::Timer { .. } => "PT",
        ElementKind::Counter { .. } => "PV",
        ElementKind::Register { .. } => "N",
        _ => return None,
    };
    Some(format!("{label} {parameter}"))
}

/// Shortens `text` with an ellipsis until it fits `width`.
fn elide(painter: &Painter, text: &str, font: FontId, colour: Color32, width: f32) -> String {
    if text.is_empty() || width <= 0.0 {
        return String::new();
    }
    if painter
        .layout_no_wrap(text.to_owned(), font.clone(), colour)
        .size()
        .x
        <= width
    {
        return text.to_owned();
    }
    let mut characters: Vec<char> = text.chars().collect();
    while !characters.is_empty() {
        characters.pop();
        let candidate: String = characters.iter().collect::<String>() + "…";
        if painter
            .layout_no_wrap(candidate.clone(), font.clone(), colour)
            .size()
            .x
            <= width
        {
            return candidate;
        }
    }
    "…".to_owned()
}

/// Draws the state badge of a network header: a pill sized to its text and
/// right-aligned in the space reserved for it.
fn paint_badge(painter: &Painter, view: View, rect: Rect, badge: &Badge) {
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

/// Marks the cells a diagnostic names with a red outline.
fn paint_problem_cells(
    painter: &Painter,
    view: View,
    network: &Network,
    grid: Layout,
    tokens: &Tokens,
    frame: &Frame<'_>,
) {
    for diagnostic in frame.problems {
        let Some(target) = queries::problem_target(frame.project(), diagnostic) else {
            continue;
        };
        if target.rung != network.rung.id {
            continue;
        }
        let Some((col, row)) = target.cell else {
            continue;
        };
        let cell = glyph_rect(layout::cell_rect(col, row, grid), view.zoom);
        if cell.is_positive() {
            painter.rect_stroke(
                cell,
                CornerRadius::same(RADIUS_PILL),
                Stroke::new(1.0_f32, tokens.error),
                StrokeKind::Outside,
            );
        }
    }
}

/// Draws the placeholder of a network that holds no element.
fn paint_empty_network(
    painter: &Painter,
    view: View,
    network: &Network,
    grid: Layout,
    tokens: &Tokens,
    frame: &Frame<'_>,
) {
    let area = Rect::from_min_max(
        grid.origin,
        view.point(
            GRID_LEFT + COL_PITCH * 5.0,
            network.grid_top() + ROW_PITCH * 2.0,
        ),
    );
    let area = Rect::from_min_max(area.min, area.max.min(painter.clip_rect().max));
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
    let text = match frame.armed_kind() {
        Some(kind) => format!("Click a cell to place {}", palette::short_name(kind)),
        None => "Empty network — pick an element to place".to_owned(),
    };
    painter.text(
        area.center(),
        Align2::CENTER_CENTER,
        text,
        view.font(TypeScale::BODY, FontFamily::Proportional),
        tokens.text_dim,
    );
}

/// The words of an empty state.
struct EmptyState<'a> {
    /// Heading of the state.
    title: &'a str,
    /// One line under the heading.
    body: &'a str,
    /// A hint about the shortcut that also works.
    hint: &'a str,
    /// `true` when the state offers an "Add a rung" button.
    add_rung: bool,
}

/// Draws the empty state of a section or a project on the paper.
fn empty_paper(
    app: &mut EditorApp,
    ui: &mut Ui,
    painter: &Painter,
    paper: Rect,
    tokens: &Tokens,
    words: EmptyState<'_>,
) {
    let EmptyState {
        title,
        body,
        hint,
        add_rung,
    } = words;
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
    let section = app.selected_section;
    let mut add = false;
    ui.scope_builder(egui::UiBuilder::new().max_rect(area), |ui| {
        empty_state(ui, tokens, title, body, hint);
        if add_rung {
            ui.vertical_centered(|ui| {
                add = ui.button("Add a rung").clicked();
            });
        }
    });
    if add {
        app.insert_rung(section, 0);
    }
}

/// Applies a click, a drag and a double click on the document.
#[allow(clippy::too_many_arguments)]
fn handle_pointer(
    app: &mut EditorApp,
    ui: &mut Ui,
    response: &egui::Response,
    networks: &[Network],
    view: View,
    space: bool,
    hover: Option<(usize, u8, u8)>,
) {
    if space {
        return;
    }
    let primary = egui::PointerButton::Primary;
    let drag_id = egui::Id::new("softladder-canvas-drag");
    if response.drag_started_by(primary) {
        // `interact_pointer_pos` is where the pointer is *now*, which is already
        // one cell away once egui decides a drag has started; the element being
        // dragged is the one under the press origin.
        let origin = response
            .ctx
            .input(|input| input.pointer.press_origin())
            .or_else(|| response.interact_pointer_pos());
        // The element under the press origin is the one being dragged. Synthetic
        // input and some platforms deliver no press origin at all, so the
        // selected element is the fallback — which is also what a user expects:
        // select it, then drag it.
        let hit = origin
            .and_then(|pos| element_hit(networks, view, pos))
            .or_else(|| {
                let (index, _, _) = cell_hit(networks, view, response.interact_pointer_pos()?)?;
                let (col, row) = app.selection?;
                let element = queries::element_at_cell(&networks.get(index)?.rung, col, row)?;
                Some((index, element.col, element.row))
            });
        if let Some((index, col, row)) = hit {
            app.drag = Some(crate::app::Drag {
                from: (col, row),
                over: Some((col, row)),
            });
            ui.ctx()
                .data_mut(|data| data.insert_temp(drag_id, Some(index)));
        }
    }
    if response.dragged_by(primary) {
        let source: Option<usize> = ui.ctx().data(|data| data.get_temp(drag_id)).flatten();
        if let (Some(drag), Some(pointer)) = (app.drag.as_mut(), response.interact_pointer_pos()) {
            if let Some((index, col, row)) = cell_hit(networks, view, pointer) {
                if Some(index) == source {
                    drag.over = Some((col, row));
                }
            }
        }
    }
    if response.drag_stopped_by(primary) {
        let source: Option<usize> = ui.ctx().data(|data| data.get_temp(drag_id)).flatten();
        ui.ctx()
            .data_mut(|data| data.insert_temp(drag_id, None::<usize>));
        if let Some(drag) = app.drag.take() {
            let Some(rung) = source
                .and_then(|index| networks.get(index))
                .map(|network| network.rung.id)
            else {
                return;
            };
            match drag.over {
                Some(to) if to != drag.from => match app.editor.move_element(rung, drag.from, to) {
                    Ok(()) => {
                        app.after_edit();
                        app.selection = Some(to);
                        app.load_properties();
                    }
                    Err(error) => app.note(&error.to_string()),
                },
                Some(to) => {
                    app.selection = Some(to);
                    app.load_properties();
                }
                None => {}
            }
        }
    }
    if response.double_clicked() {
        start_text_edit(app, ui, networks, view, response.interact_pointer_pos());
    }
    if response.clicked() {
        let Some(pos) = response.interact_pointer_pos() else {
            return;
        };
        if let Some((index, col, row)) = cell_hit(networks, view, pos) {
            let Some(rung) = networks.get(index).map(|network| network.rung.id) else {
                return;
            };
            match app.tool {
                Tool::Place(_) => {
                    if app.selected_rung != Some(rung) {
                        app.select_rung(rung, Some((col, row)));
                    }
                    app.place_at(col, row);
                }
                Tool::Select => {
                    let cell = element_hit(networks, view, pos)
                        .map(|(_, col, row)| (col, row))
                        .unwrap_or((col, row));
                    app.select_rung(rung, Some(cell));
                }
            }
        } else if let Some(index) = band_hit(networks, view, pos) {
            if let Some(rung) = networks.get(index).map(|network| network.rung.id) {
                app.select_rung(rung, None);
            }
        }
    }
    if let Some((_, col, row)) = hover {
        let cursor = match app.tool {
            Tool::Place(_) => CursorIcon::Crosshair,
            Tool::Select => {
                let _ = (col, row);
                CursorIcon::PointingHand
            }
        };
        ui.ctx().set_cursor_icon(cursor);
    }
}

/// Starts editing a rung's title or comment in place, on a double click.
fn start_text_edit(
    app: &mut EditorApp,
    ui: &mut Ui,
    networks: &[Network],
    view: View,
    pos: Option<Pos2>,
) {
    let Some(pos) = pos else {
        return;
    };
    for network in networks {
        let rects = header_rects(
            view.rect(network.header()),
            !network.rung.label.trim().is_empty(),
            network.comment_lines,
        );
        let field = if rects.comment.is_positive() && rects.comment.contains(pos) {
            EditField::Comment
        } else if rects.title.is_positive() && rects.title.contains(pos) {
            EditField::Title
        } else {
            continue;
        };
        app.rung_text_target = Some(network.rung.id);
        app.rung_label_buffer = network.rung.label.clone();
        app.rung_comment_buffer = network.rung.comment.clone();
        ui.ctx()
            .data_mut(|data| data.insert_temp(edit_id(), Some((network.rung.id, field))));
        return;
    }
}

/// Which line of a network header is being edited on the canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditField {
    /// The rung label.
    Title,
    /// The rung comment.
    Comment,
}

/// The id the canvas stores its in-place editor state under.
fn edit_id() -> egui::Id {
    egui::Id::new("softladder-canvas-text-edit")
}

/// Draws the in-place title/comment editor, if the canvas opened one.
fn inline_editor(app: &mut EditorApp, ui: &mut Ui, networks: &[Network], view: View) {
    let editing: Option<(u32, EditField)> =
        ui.ctx().data(|data| data.get_temp(edit_id())).flatten();
    let Some((rung, field)) = editing else {
        return;
    };
    let Some(network) = networks.iter().find(|network| network.rung.id == rung) else {
        return;
    };
    let rects = header_rects(
        view.rect(network.header()),
        !network.rung.label.trim().is_empty(),
        network.comment_lines,
    );
    let rect = match field {
        EditField::Title => rects.title,
        EditField::Comment => rects.comment,
    };
    if rect.width() < 24.0 || !rect.is_positive() {
        return;
    }
    let response = match field {
        EditField::Title => ui.put(rect, egui::TextEdit::singleline(&mut app.rung_label_buffer)),
        EditField::Comment => ui.put(
            rect,
            egui::TextEdit::singleline(&mut app.rung_comment_buffer),
        ),
    };
    let done = response.lost_focus() || ui.input(|input| input.key_pressed(egui::Key::Enter));
    if done {
        let label = app.rung_label_buffer.clone();
        let comment = app.rung_comment_buffer.clone();
        app.set_rung_text(rung, &label, &comment);
        ui.ctx()
            .data_mut(|data| data.insert_temp(edit_id(), None::<(u32, EditField)>));
    }
}

/// Shows the full expression of a compare or operate block on hover.
fn expression_tooltip(
    response: &egui::Response,
    networks: &[Network],
    hover: Option<(usize, u8, u8)>,
) {
    let Some((index, col, row)) = hover else {
        return;
    };
    let Some(network) = networks.get(index) else {
        return;
    };
    let Some(element) = queries::element_at_cell(&network.rung, col, row) else {
        return;
    };
    if !matches!(element.kind, ElementKind::Compare | ElementKind::Operate) {
        return;
    }
    let expression = element.params.join(" ");
    if expression.is_empty() {
        return;
    }
    response.clone().on_hover_text(expression);
}

/// The right-click menu of a cell.
fn handle_menu(
    app: &mut EditorApp,
    ui: &mut Ui,
    response: &egui::Response,
    networks: &[Network],
    view: View,
    hover: Option<(usize, u8, u8)>,
) {
    let id = egui::Id::new("softladder-canvas-menu");
    let stored: Option<(usize, u8, u8)> = ui.ctx().data(|data| data.get_temp(id)).flatten();
    let cell = if response.secondary_clicked() {
        hover
    } else {
        stored
    };
    if response.secondary_clicked() {
        ui.ctx().data_mut(|data| data.insert_temp(id, cell));
        // Right-clicking a cell selects it, so the commands and the properties
        // strip agree about what "this cell" means.
        if let Some((index, col, row)) = cell {
            if let Some(network) = networks.get(index) {
                let target = queries::element_at_cell(&network.rung, col, row)
                    .map(|element| (element.col, element.row))
                    .unwrap_or((col, row));
                let rung = network.rung.id;
                app.select_rung(rung, Some(target));
            }
        }
    }
    let Some(cell) = cell else {
        return;
    };
    let opened = response.context_menu(|ui| context_menu(app, ui, networks, view, cell));
    if opened.is_none() {
        ui.ctx()
            .data_mut(|data| data.insert_temp(id, None::<(usize, u8, u8)>));
    }
}

/// The entries of the cell context menu.
///
/// Everything the editor can do is here; a command the model has no shape for is
/// shown disabled with the reason on its tooltip rather than hidden.
fn context_menu(
    app: &mut EditorApp,
    ui: &mut Ui,
    networks: &[Network],
    view: View,
    cell: (usize, u8, u8),
) {
    let _ = view;
    let (index, col, row) = cell;
    let Some(network) = networks.get(index) else {
        return;
    };
    let rung = network.rung.id;
    let element = queries::element_at_cell(&network.rung, col, row).cloned();

    ui.label(
        egui::RichText::new(match &element {
            Some(element) => format!("{} · col {col} row {row}", describe(element.kind)),
            None => format!("empty cell · col {col} row {row}"),
        })
        .size(TypeScale::CAPTION),
    );
    ui.separator();

    for label in ["Insert contact before", "Insert contact after"] {
        if ui
            .add_enabled(false, egui::Button::new(label))
            .on_disabled_hover_text(
                "The editor has no insert-and-shift command: place the contact on a free cell",
            )
            .clicked()
        {}
    }

    let branch_row = row.saturating_add(1);
    let branch_free = element.is_none()
        && branch_row <= layout::MAX_ROW
        && queries::element_at_cell(&network.rung, col, branch_row).is_none();
    if ui
        .add_enabled(branch_free, egui::Button::new("Insert branch"))
        .on_hover_text("Draw a parallel branch into the row below: a connection linked upwards")
        .on_disabled_hover_text("The cell below is occupied, or is past the last row")
        .clicked()
    {
        let mut connection = palette::element(ElementKind::Connection, col, branch_row);
        connection.connected_with_top = true;
        match app.editor.apply(Command::ReplaceElement {
            rung,
            element: connection,
        }) {
            Ok(()) => {
                app.after_edit();
                app.selection = Some((col, branch_row));
                app.load_properties();
                app.note("branch inserted");
            }
            Err(error) => app.note(&error.to_string()),
        }
        ui.close_menu();
    }

    if ui
        .add_enabled(element.is_some(), egui::Button::new("Toggle vertical link"))
        .on_hover_text("Link this cell to the one above (V)")
        .on_disabled_hover_text("There is no element on this cell")
        .clicked()
    {
        if let Some(element) = &element {
            app.selection = Some((element.col, element.row));
            app.load_properties();
        }
        app.handle(crate::shortcuts::Action::ToggleVerticalLink);
        ui.close_menu();
    }
    if ui
        .add_enabled(element.is_some(), egui::Button::new("Delete"))
        .on_hover_text("Delete the element (Del)")
        .on_disabled_hover_text("There is no element on this cell")
        .clicked()
    {
        if let Some(element) = &element {
            app.selection = Some((element.col, element.row));
            app.load_properties();
        }
        app.handle(crate::shortcuts::Action::Delete);
        ui.close_menu();
    }

    ui.separator();
    for label in ["Cut", "Copy", "Paste"] {
        if ui
            .add_enabled(false, egui::Button::new(label))
            .on_disabled_hover_text("The editor has no clipboard yet")
            .clicked()
        {}
    }

    ui.separator();
    let tag = element
        .as_ref()
        .and_then(|element| element.var.as_ref())
        .and_then(|var| queries::symbol_for(app.project(), var))
        .map(|symbol| symbol.name.clone());
    let go_to_tag = tag
        .as_ref()
        .and_then(|name| app.project().symbols.iter().position(|s| &s.name == name));
    if ui
        .add_enabled(go_to_tag.is_some(), egui::Button::new("Go to tag"))
        .on_hover_text(match &tag {
            Some(name) => format!("Show `{name}` in the PLC tags table"),
            None => String::new(),
        })
        .on_disabled_hover_text("This element is not bound to a tag")
        .clicked()
    {
        if let Some(index) = go_to_tag {
            app.selected_tag = Some(index);
            app.centre_tab = CentreTab::Tags;
        }
        ui.close_menu();
    }
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
fn handle_zoom(app: &mut EditorApp, ui: &egui::Ui, response: &egui::Response, paper: Rect) {
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

/// Names the element kind for the palette tooltip and the properties strip.
pub fn describe(kind: ElementKind) -> &'static str {
    match kind {
        ElementKind::Timer {
            mode: softladder_core::TimerMode::On,
        } => "TON",
        ElementKind::Timer {
            mode: softladder_core::TimerMode::Off,
        } => "TOF",
        ElementKind::Timer {
            mode: softladder_core::TimerMode::Pulse,
        } => "TP",
        ElementKind::Counter {
            kind: softladder_core::CounterKind::Up,
        } => "CTU",
        ElementKind::Counter {
            kind: softladder_core::CounterKind::Down,
        } => "CTD",
        ElementKind::Counter {
            kind: softladder_core::CounterKind::UpDown,
        } => "CTUD",
        ElementKind::Register {
            mode: softladder_core::RegisterMode::Fifo,
        } => "FIFO",
        ElementKind::Register {
            mode: softladder_core::RegisterMode::Lifo,
        } => "LIFO",
        other => palette::short_name(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{pos2, vec2};
    use softladder_core::{
        CounterKind, PlacedElement, Project, RegisterMode, Rung, Section, TimerMode, VarKind,
        VarRef,
    };

    /// A project with one section, one rung and the elements the test asks for.
    fn project_with(rung: Rung) -> Project {
        let mut project = Project::new("canvas tests");
        let mut section = Section::new(1, "Main");
        section.rungs.push(rung.id);
        project.sections.push(section);
        project.rungs.push(rung);
        project
    }

    fn var(text: &str) -> softladder_core::VarRef {
        text.parse().expect("test variable parses")
    }

    /// Runs `check` with a painter from a real (headless) frame.
    fn with_painter<R>(check: impl FnOnce(&Painter) -> R) -> R {
        let ctx = egui::Context::default();
        let mut painter = None;
        let _ = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                painter = Some(ui.painter().clone());
            });
        });
        let painter = painter.expect("the frame ran");
        check(&painter)
    }

    #[test]
    fn fitting_frames_a_document_that_fits_and_never_magnifies() {
        let content = Rect::from_min_size(Pos2::ZERO, vec2(400.0, 200.0));
        let viewport = Rect::from_min_size(pos2(50.0, 50.0), vec2(800.0, 400.0));
        let camera = fit_camera(content, viewport, viewport.min, None);
        assert_eq!(camera.zoom, 1.0, "a small document is not blown up");
        // The document starts at the top left of the viewport.
        let top_left = viewport.min + camera.pan + content.min.to_vec2() * camera.zoom;
        assert!((top_left.y - viewport.top()).abs() < 0.01, "top aligned");
        assert!((top_left.x - viewport.left()).abs() < 0.01, "left aligned");

        // A document that does not fit width-wise is scaled down, but never
        // below the zoom a ladder stays readable at.
        let wide = Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 200.0));
        let camera = fit_camera(wide, viewport, viewport.min, None);
        assert!(
            (camera.zoom - FIT_MIN_ZOOM).abs() < 0.01,
            "zoom {}",
            camera.zoom
        );
        assert!(wide.width() * camera.zoom <= viewport.width() + 0.01);
    }

    #[test]
    fn fitting_a_long_section_frames_the_selected_network() {
        let content = Rect::from_min_size(Pos2::ZERO, vec2(600.0, 4000.0));
        let viewport = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 400.0));
        let focus = Rect::from_min_size(pos2(0.0, 3000.0), vec2(600.0, 400.0));
        let camera = fit_camera(content, viewport, viewport.min, Some(focus));
        assert_eq!(camera.zoom, FIT_MIN_ZOOM, "the ladder stays readable");
        // The focused network is inside the window, and the window stays inside
        // the document.
        let visible_top = (Pos2::ZERO - viewport.min - camera.pan) / camera.zoom;
        let visible_bottom = visible_top.y + viewport.height() / camera.zoom;
        assert!(visible_bottom >= focus.top(), "the focus is above the fold");
        assert!(
            visible_top.y <= focus.bottom(),
            "the focus is below the fold"
        );
        assert!(
            visible_top.y >= content.top() - 0.01,
            "scrolled past the top"
        );
        assert!(
            visible_bottom <= content.bottom() + 0.01,
            "scrolled past the bottom"
        );
    }

    #[test]
    fn fitting_survives_degenerate_rectangles() {
        let camera = fit_camera(Rect::ZERO, Rect::ZERO, Pos2::ZERO, None);
        assert_eq!(camera, Camera::default());
        let camera = fit_camera(
            Rect::from_min_size(Pos2::ZERO, vec2(0.0, 100.0)),
            Rect::from_min_size(Pos2::ZERO, vec2(100.0, 100.0)),
            Pos2::ZERO,
            None,
        );
        assert_eq!(camera, Camera::default());
    }

    #[test]
    fn the_header_stacks_caption_title_comment_and_badge() {
        let header = Rect::from_min_size(Pos2::ZERO, vec2(DOC_WIDTH, 120.0));
        let rects = header_rects(header, true, 2);
        assert!(
            rects.caption.top() < rects.title.top(),
            "caption over title"
        );
        assert!(
            rects.title.bottom() <= rects.comment.top(),
            "title over note"
        );
        assert!(
            rects.comment.bottom() <= header.bottom(),
            "the comment stays in the header"
        );
        assert!(
            rects.badge.right() <= header.right(),
            "the badge is inside the header"
        );
        assert!(
            rects.badge.left() > rects.caption.left(),
            "the badge sits at the right"
        );
        assert!((rects.badge.center().y - rects.caption.center().y).abs() < 0.01);

        // A rung without a label loses its title row and gets shorter.
        let without = header_rects(header, false, 2);
        assert_eq!(without.title, Rect::NOTHING);
        assert!(header_height(false, 2) < header_height(true, 2));
        assert!(header_height(true, 3) > header_height(true, 2));
    }

    #[test]
    fn the_ghost_cell_is_inset_and_centred() {
        let cell = Rect::from_min_size(pos2(10.0, 20.0), vec2(96.0, 80.0));
        let ghost = ghost_rect(cell);
        assert!(cell.contains(ghost.min) && cell.contains(ghost.max));
        assert!((ghost.center() - cell.center()).length() < 0.01);
        assert_eq!(ghost.width(), cell.width() - 2.0);
        assert_eq!(ghost.height(), cell.height() - 2.0);
    }

    #[test]
    fn a_vertical_stub_joins_the_two_rows_at_the_left_edge() {
        let upper = Rect::from_min_size(pos2(20.0, 0.0), vec2(96.0, 48.0));
        let lower = Rect::from_min_size(pos2(20.0, 80.0), vec2(96.0, 48.0));
        let [from, to] = vertical_stub(upper, lower).expect("two real cells link");
        assert_eq!(from.x, lower.left());
        assert_eq!(to.x, lower.left());
        assert_eq!(from.y, upper.center().y);
        assert_eq!(to.y, lower.center().y);
        assert!(from.y < to.y, "the stub runs downwards");
        assert_eq!(vertical_stub(Rect::ZERO, lower), None);
    }

    #[test]
    fn the_ghost_cell_and_the_cell_mapping_agree() {
        // A pointer inside the ghost of a cell must map back to that cell.
        let grid = Layout::with_cell(pos2(100.0, 200.0), vec2(96.0, 80.0));
        for (col, row) in [(0_u8, 0_u8), (2, 1), (3, 4)] {
            let cell = layout::cell_rect(col, row, grid);
            let ghost = ghost_rect(cell);
            assert_eq!(layout::cell_at(ghost.center(), grid), Some((col, row)));
        }
    }

    #[test]
    fn block_spans_and_grid_extents_follow_the_engine_rows() {
        assert_eq!(queries::block_span(ElementKind::ContactNo), 1);
        assert_eq!(
            queries::block_span(ElementKind::Counter {
                kind: CounterKind::Up
            }),
            4
        );
        assert_eq!(
            queries::block_span(ElementKind::Register {
                mode: softladder_core::RegisterMode::Fifo
            }),
            3
        );

        let rung = Rung {
            elements: vec![PlacedElement::with_var(
                ElementKind::Counter {
                    kind: CounterKind::Up,
                },
                VarRef::new(VarKind::Counter, 0),
                1,
                0,
            )],
            ..Rung::new(1)
        };
        assert_eq!(queries::rung_rows(&rung), 4, "the counter reads four rows");
        assert_eq!(queries::rung_cols(&rung), 2);
        assert!(queries::element_at_cell(&rung, 1, 3).is_some(), "row three");
        assert!(
            queries::element_at_cell(&rung, 1, 4).is_none(),
            "past the end"
        );
    }

    #[test]
    fn a_cell_hit_stays_inside_its_own_network() {
        let app = EditorApp::new(project_with(Rung {
            elements: vec![PlacedElement::with_var(
                ElementKind::ContactNo,
                var("%I0"),
                0,
                0,
            )],
            ..Rung::new(1)
        }));
        let painter_networks =
            with_painter(|painter| read_networks(&app, painter, &Tokens::light()));
        assert_eq!(painter_networks.len(), 1);
        let network = painter_networks.first().expect("one network");
        let view = View {
            origin: Pos2::ZERO,
            zoom: 1.0,
        };
        let grid = view.layout(network.grid_top());
        let inside = layout::cell_rect(0, 0, grid).center();
        assert_eq!(cell_hit(&painter_networks, view, inside), Some((0, 0, 0)));
        // The header of the band is not a cell, and the two agree on the band.
        let above = view.point(2.0, network.top + 2.0);
        assert_eq!(cell_hit(&painter_networks, view, above), None);
        assert_eq!(band_hit(&painter_networks, view, above), Some(0));
        // Past the last row of the wiring area there is no cell either.
        let below = view.point(
            4.0,
            network.grid_top() + ROW_PITCH * (f32::from(network.rows) + 0.5),
        );
        assert_eq!(cell_hit(&painter_networks, view, below), None);
        assert_eq!(band_hit(&painter_networks, view, below), None);
    }

    #[test]
    fn the_badge_reports_errors_warnings_and_ok() {
        let tokens = Tokens::light();
        let rung = Rung::new(7);
        let project = project_with(rung.clone());
        let clean = network_badge(&tokens, &project, &[], 7);
        assert_eq!(clean.text, "ok");
        assert_eq!(clean.colour, tokens.run);

        let warning = Diagnostic::new(Severity::Warning, "SL-W001", "no power".to_owned())
            .with_section(0)
            .with_rung(0);
        let badge = network_badge(&tokens, &project, &[warning], 7);
        assert_eq!(badge.text, "1 warning");
        assert_eq!(badge.colour, tokens.warning);

        let errors = vec![
            Diagnostic::new(Severity::Error, "SL-E004", "no variable".to_owned())
                .with_section(0)
                .with_rung(0),
            Diagnostic::new(Severity::Error, "SL-E004", "no variable".to_owned())
                .with_section(0)
                .with_rung(0),
        ];
        let badge = network_badge(&tokens, &project, &errors, 7);
        assert_eq!(badge.text, "2 errors");
        assert_eq!(badge.colour, tokens.error);
        // A diagnostic that points at another rung leaves this one alone.
        assert_eq!(network_badge(&tokens, &project, &errors, 99).text, "ok");
    }

    #[test]
    fn the_document_stacks_its_networks_without_gaps_or_overlaps() {
        let mut project = Project::new("stack");
        let mut section = Section::new(1, "Main");
        let mut first = Rung::new(1);
        first.label = "first".to_owned();
        first.comment = "a note".to_owned();
        first.elements.push(PlacedElement::with_var(
            ElementKind::ContactNo,
            var("%I0"),
            0,
            0,
        ));
        let mut second = Rung::new(2);
        second.elements.push(PlacedElement::with_var(
            ElementKind::Timer {
                mode: TimerMode::On,
            },
            VarRef::new(VarKind::TimerIec, 0),
            0,
            0,
        ));
        section.rungs.push(1);
        section.rungs.push(2);
        project.sections.push(section);
        project.rungs.push(first);
        project.rungs.push(second);
        let app = EditorApp::new(project);
        let networks = with_painter(|painter| read_networks(&app, painter, &Tokens::light()));
        assert_eq!(networks.len(), 2);
        let (first, second) = (
            networks.first().expect("first"),
            networks.get(1).expect("second"),
        );
        assert_eq!(first.number, 1);
        assert_eq!(second.number, 2);
        assert!(
            first.bottom() <= second.top,
            "the second band starts after the first"
        );
        assert!(first.height > 0.0 && second.height > 0.0);
        // An empty project has nothing to draw.
        let empty = EditorApp::new(Project::new("empty"));
        let none = with_painter(|painter| read_networks(&empty, painter, &Tokens::light()));
        assert!(none.is_empty());
    }

    #[test]
    fn every_element_kind_has_pin_names_or_a_glyph() {
        for kind in [
            ElementKind::ContactNo,
            ElementKind::ContactNc,
            ElementKind::CoilOut,
            ElementKind::CoilSet,
            ElementKind::Connection,
            ElementKind::Compare,
            ElementKind::Operate,
            ElementKind::Timer {
                mode: TimerMode::On,
            },
            ElementKind::Counter {
                kind: CounterKind::Up,
            },
        ] {
            assert!(!describe(kind).is_empty());
            if is_block(kind) {
                assert!(!block_pins(kind).is_empty(), "{kind:?} has no pins");
            }
        }
        assert_eq!(
            block_pins(ElementKind::Counter {
                kind: CounterKind::Down
            })
            .iter()
            .filter(|(_, side)| *side < 0)
            .count(),
            4,
            "a counter always shows the four rows it reads"
        );
    }

    /// The bug this guards: the timer used to draw `PT` as a second input pin,
    /// but the engine only reads the enable (the preset is a parameter), so the
    /// pin could never be connected to anything that mattered.
    #[test]
    fn a_timer_has_one_input_pin_and_no_wirable_preset() {
        let pins = block_pins(ElementKind::Timer {
            mode: TimerMode::On,
        });
        let inputs: Vec<&str> = pins
            .iter()
            .filter(|(_, side)| *side < 0)
            .map(|(label, _)| *label)
            .collect();
        assert_eq!(inputs, ["IN"]);
        assert!(
            !pins.contains(&("PT", -1)),
            "the preset is a parameter, not a wire"
        );
        assert!(pins.contains(&("ET", 1)), "the elapsed time is a readout");
    }

    /// Every pin drawn on the left must be a row the engine reads, or the canvas
    /// is advertising a connection that does nothing.
    #[test]
    fn the_drawn_input_pins_are_the_rows_the_engine_reads() {
        for kind in [
            ElementKind::Timer {
                mode: TimerMode::Off,
            },
            ElementKind::Counter {
                kind: CounterKind::UpDown,
            },
            ElementKind::Register {
                mode: RegisterMode::Lifo,
            },
        ] {
            let drawn: Vec<&str> = block_pins(kind)
                .iter()
                .filter(|(_, side)| *side < 0)
                .map(|(label, _)| *label)
                .collect();
            assert_eq!(
                drawn,
                kind.input_pins(),
                "{kind:?} draws input pins the engine does not read"
            );
            assert_eq!(
                drawn.len(),
                kind.input_rows(),
                "{kind:?} draws more inputs than rows the engine reads"
            );
        }
    }

    /// The user's report: a counter has four wirable inputs, and they must all
    /// be reachable — the engine reads each of the four rows, the band reserves
    /// the four rows, and the power the canvas draws feeds each pin.
    #[test]
    fn every_counter_input_can_be_wired_and_is_shown_fed() {
        use softladder_core::{CounterKind, Value};
        let mut rung = Rung::new(1);
        for row in 0..4u8 {
            let mut contact = PlacedElement::new(ElementKind::ContactNo, 0, row);
            contact.var = Some(VarRef::new(VarKind::PhysIn, u32::from(row)));
            rung.elements.push(contact);
        }
        let mut counter = PlacedElement::new(
            ElementKind::Counter {
                kind: CounterKind::UpDown,
            },
            2,
            0,
        );
        counter.var = Some(VarRef::new(VarKind::Counter, 0));
        counter.params = vec!["1".to_owned()];
        rung.elements.push(counter);
        assert_eq!(
            queries::rung_rows(&rung),
            4,
            "the band must reserve every row the counter reads"
        );

        let mut project = Project::new("counters");
        project.rungs.push(rung);
        let mut section = Section::new(1, "Main");
        section.rungs.push(1);
        project.sections.push(section);
        let mut app = EditorApp::new(project);

        // Close every one of the four inputs and run a scan.
        for row in 0..4u32 {
            let var = VarRef::new(VarKind::PhysIn, row);
            app.set_variable(&var, Value::Bit(true)).expect("the store");
        }
        app.single_scan();

        let power = queries::power_map(&app.project().rungs[0], app.bench().engine().store());
        for row in 0..4u8 {
            let wire = queries::cell_state(&power, 1, row);
            assert!(
                wire.live || wire.fed,
                "the wire feeding the counter's row {row} pin is not fed"
            );
        }

        // The engine's reading of these four rows (and that row 2 counts) is
        // covered by `softladder-core`'s `engine_counter_counts_up_and_reports_done`;
        // what matters here is the editor's half: the band reserves the rows and
        // every pin's wire is fed, so all four inputs are reachable and visible.
    }

    #[test]
    fn a_blocks_parameter_is_labelled_as_a_parameter() {
        let mut timer = PlacedElement::new(
            ElementKind::Timer {
                mode: TimerMode::On,
            },
            0,
            0,
        );
        assert_eq!(block_parameter_text(&timer), None, "no parameter, no label");
        timer.params = vec!["3s".to_owned()];
        assert_eq!(block_parameter_text(&timer).as_deref(), Some("PT 3s"));
        let mut counter = PlacedElement::new(
            ElementKind::Counter {
                kind: CounterKind::Up,
            },
            0,
            0,
        );
        counter.params = vec!["5".to_owned()];
        assert_eq!(block_parameter_text(&counter).as_deref(), Some("PV 5"));
        let mut contact = PlacedElement::new(ElementKind::ContactNo, 0, 0);
        contact.params = vec!["7".to_owned()];
        assert_eq!(block_parameter_text(&contact), None);
    }

    #[test]
    fn a_timer_parameter_is_shown_in_the_engines_units() {
        let mut element = PlacedElement::new(
            ElementKind::Timer {
                mode: TimerMode::On,
            },
            0,
            0,
        );
        element.params = vec!["3000".to_owned()];
        assert_eq!(parameter_units(&element), Some(Value::Word(30)));
        element.params = vec!["3s".to_owned()];
        assert_eq!(parameter_units(&element), Some(Value::Word(3)));
        element.params = vec!["2m".to_owned()];
        assert_eq!(parameter_units(&element), Some(Value::Word(2)));
        element.params = vec!["%MW0".to_owned()];
        assert_eq!(
            parameter_units(&element),
            None,
            "a variable is left to the store"
        );
        element.params.clear();
        assert_eq!(parameter_units(&element), None);
    }

    #[test]
    fn labels_follow_the_tag_toggle() {
        let mut project = Project::new("labels");
        project.symbols.push(softladder_core::Symbol {
            name: "start".to_owned(),
            var: Some(var("%I0")),
            comment: String::new(),
            unit: None,
        });
        let tagged = PlacedElement::with_var(ElementKind::ContactNo, var("%I0"), 0, 0);
        let (name, address) = label_texts(&project, &tagged);
        assert_eq!(name, Some("start"));
        assert_eq!(address, "%I0");
        let (name, address) =
            label_texts(&project, &PlacedElement::new(ElementKind::Compare, 0, 0));
        assert_eq!(name, None);
        assert_eq!(address, "");
    }

    #[test]
    fn drawing_every_state_of_an_empty_editor_is_panic_free() {
        let ctx = egui::Context::default();
        let frame = |app: &mut EditorApp| {
            let _ = ctx.run(egui::RawInput::default(), |ctx| app.draw(ctx));
        };
        // No sections at all.
        let mut app = EditorApp::new(Project::new("empty"));
        frame(&mut app);
        // A section with no rungs.
        let mut stub = Project::new("stub");
        stub.sections.push(Section::new(1, "Main"));
        let mut app = EditorApp::new(stub);
        frame(&mut app);
        app.centre_tab = crate::app::CentreTab::Ladder;
        // A rung with an element on every kind of cell, running and idle.
        let mut app = EditorApp::new(project_with(Rung {
            elements: vec![
                PlacedElement::with_var(ElementKind::ContactNo, var("%I0"), 0, 0),
                PlacedElement::with_params(ElementKind::Compare, 1, 0, &["%MW0", ">", "3"]),
                PlacedElement::with_var(
                    ElementKind::Counter {
                        kind: CounterKind::Up,
                    },
                    VarRef::new(VarKind::Counter, 0),
                    2,
                    0,
                ),
            ],
            ..Rung::new(1)
        }));
        frame(&mut app);
        app.handle(crate::shortcuts::Action::RunStop);
        frame(&mut app);
        app.handle(crate::shortcuts::Action::Pick(ElementKind::ContactNc));
        frame(&mut app);
        app.selection = Some((0, 0));
        frame(&mut app);
        app.theme = crate::design::Theme::Dark;
        frame(&mut app);
    }
}
