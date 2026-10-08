//! The rung canvas: grid, power rail, elements and the live indication.
//!
//! The canvas draws the rung the left panel selected. All geometry comes from
//! [`crate::layout`], so a pointer position becomes a cell through the same
//! mapping the tests exercise, and all live values come from the bench's engine
//! store — the canvas itself remembers nothing.

use egui::{
    Align2, Color32, CornerRadius, FontId, Painter, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2,
};
use softladder_core::{Accessor, CounterKind, ElementKind, PlacedElement, RegisterMode, TimerMode};

use crate::app::{block_of, store, EditorApp, Tool};
use crate::layout::{self, Layout};
use crate::palette;
use crate::queries;

/// Canvas background.
const BACKGROUND: Color32 = Color32::from_rgb(22, 24, 29);
/// Grid line colour.
const GRID: Color32 = Color32::from_rgb(38, 41, 48);
/// Power-rail colour.
const RAIL: Color32 = Color32::from_rgb(120, 128, 140);
/// Colour of a de-energised element.
const DEAD: Color32 = Color32::from_rgb(130, 138, 150);
/// Colour of an energised element.
const LIVE: Color32 = Color32::from_rgb(90, 220, 120);
/// Selection highlight.
const SELECT: Color32 = Color32::from_rgb(255, 190, 80);
/// Problem marker.
const PROBLEM: Color32 = Color32::from_rgb(230, 90, 90);
/// Dim text colour.
const DIM: Color32 = Color32::from_rgb(150, 158, 170);

/// Distance between the canvas edge and the grid.
const MARGIN: f32 = 26.0;
/// Height reserved above the grid for the rung header.
const HEADER: f32 = 30.0;

/// Draws the canvas and turns pointer and keyboard input into editor calls.
pub fn show(app: &mut EditorApp, ui: &mut egui::Ui) {
    let sense = Sense::click_and_drag();
    let (response, painter) = ui.allocate_painter(ui.available_size(), sense);
    let rect = response.rect;
    painter.rect_filled(rect, CornerRadius::ZERO, BACKGROUND);
    let base = rect.min + Vec2::new(MARGIN, HEADER);

    let space = ui.input(|input| input.key_down(egui::Key::Space));
    handle_zoom(app, ui, &response, base);
    handle_pan(app, &response, space);

    let origin = base + app.camera.pan;
    let layout = Layout::new(origin, app.camera.zoom);
    paint_grid(&painter, rect, layout);

    let rung = app.selected_rung_ref().cloned();
    let power = match rung.as_ref() {
        Some(rung) => queries::power_grid(rung, store(app)),
        None => Vec::new(),
    };
    paint_rung_header(app, &painter, base, rect, rung.as_ref());
    let hovered = response
        .hover_pos()
        .and_then(|pos| layout::cell_at(pos, layout));
    match rung.as_ref() {
        Some(rung) => {
            paint_rail(&painter, layout, rung.row_count());
            for element in &rung.elements {
                let energised = queries::cell_power(&power, element.col, element.row);
                let selected = app.selection == Some((element.col, element.row));
                paint_element(app, &painter, layout, element, energised, selected);
            }
        }
        None => {
            paint_notice(&painter, rect, "No rung selected");
        }
    }
    if let Tool::Place(kind) = app.tool {
        if let Some((col, row)) = hovered {
            let cell = layout::cell_rect(col, row, layout);
            painter.rect_stroke(
                cell,
                CornerRadius::same(2),
                Stroke::new(2.0_f32, SELECT),
                StrokeKind::Outside,
            );
            painter.text(
                cell.center_bottom() + Vec2::new(0.0, 2.0),
                Align2::CENTER_TOP,
                palette::short_name(kind),
                FontId::proportional(10.0),
                SELECT,
            );
        }
    }
    handle_pointer(app, &response, layout, space);
}

/// Applies a pointer click or drag to the editor.
fn handle_pointer(app: &mut EditorApp, response: &egui::Response, layout: Layout, space: bool) {
    if space {
        return;
    }
    let primary = egui::PointerButton::Primary;
    if response.drag_started_by(primary) {
        // `interact_pointer_pos` is where the pointer is *now*, which is already
        // one cell away once egui decides a drag has started; the element being
        // dragged is the one under the press origin.
        let origin = response
            .ctx
            .input(|input| input.pointer.press_origin())
            .or_else(|| response.interact_pointer_pos());
        if let Some(cell) = origin.and_then(|pos| layout::cell_at(pos, layout)) {
            if app
                .selected_rung
                .is_some_and(|rung| app.element_at(rung, cell.0, cell.1).is_some())
            {
                app.drag = Some(crate::app::Drag {
                    from: cell,
                    over: Some(cell),
                });
            }
        }
    }
    if response.dragged_by(primary) {
        if let (Some(drag), Some(cell)) = (
            app.drag.as_mut(),
            response
                .interact_pointer_pos()
                .and_then(|pos| layout::cell_at(pos, layout)),
        ) {
            drag.over = Some(cell);
        }
    }
    if response.drag_stopped_by(primary) {
        if let Some(drag) = app.drag.take() {
            let Some(rung) = app.selected_rung else {
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
    if response.clicked() {
        if let Some(cell) = response
            .interact_pointer_pos()
            .and_then(|pos| layout::cell_at(pos, layout))
        {
            match app.tool {
                Tool::Place(_) => app.place_at(cell.0, cell.1),
                Tool::Select => {
                    app.selection = Some(cell);
                    app.load_properties();
                }
            }
        }
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
fn handle_zoom(app: &mut EditorApp, ui: &egui::Ui, response: &egui::Response, base: Pos2) {
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
        let origin = base + app.camera.pan;
        let grid = (pointer - origin) / old_zoom;
        app.camera.pan = pointer - grid * new_zoom - base;
    }
    app.camera.set_zoom(new_zoom);
}

/// Draws the whole visible grid.
fn paint_grid(painter: &Painter, rect: Rect, layout: Layout) {
    let drawable = layout.cell.x > 1.0 && layout.cell.y > 1.0;
    if !drawable {
        return;
    }
    let stroke = Stroke::new(1.0_f32, GRID);
    let first_col = ((rect.left() - layout.origin.x) / layout.cell.x)
        .floor()
        .max(0.0);
    let mut x = layout.origin.x + first_col * layout.cell.x;
    let mut guard = 0;
    while x <= rect.right() && guard < 4096 {
        painter.line_segment(
            [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
            stroke,
        );
        x += layout.cell.x;
        guard += 1;
    }
    let first_row = ((rect.top() - layout.origin.y) / layout.cell.y)
        .floor()
        .max(0.0);
    let mut y = layout.origin.y + first_row * layout.cell.y;
    let mut guard = 0;
    while y <= rect.bottom() && guard < 4096 {
        painter.line_segment(
            [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
            stroke,
        );
        y += layout.cell.y;
        guard += 1;
    }
}

/// Draws the left power rail beside column zero.
fn paint_rail(painter: &Painter, layout: Layout, rows: u8) {
    let top = layout::cell_rect(0, 0, layout).top();
    let bottom = layout::cell_rect(0, rows.saturating_sub(1), layout).bottom();
    let x = layout.origin.x - 10.0;
    painter.line_segment(
        [Pos2::new(x, top), Pos2::new(x, bottom)],
        Stroke::new(3.0_f32, RAIL),
    );
    for row in 0..rows {
        let y = layout::cell_rect(0, row, layout).center().y;
        painter.line_segment(
            [Pos2::new(x, y), Pos2::new(layout.origin.x, y)],
            Stroke::new(3.0_f32, RAIL),
        );
    }
}

/// Draws the rung title, its comment and the red error edge.
fn paint_rung_header(
    app: &EditorApp,
    painter: &Painter,
    base: Pos2,
    rect: Rect,
    rung: Option<&softladder_core::Rung>,
) {
    let Some(rung) = rung else {
        return;
    };
    let title = format!("#{} {}", rung.id, rung.label);
    painter.text(
        Pos2::new(base.x, rect.top() + 6.0),
        Align2::LEFT_TOP,
        &title,
        FontId::proportional(13.0),
        Color32::from_rgb(215, 220, 230),
    );
    if !rung.comment.is_empty() {
        painter.text(
            Pos2::new(base.x + 120.0, rect.top() + 7.0),
            Align2::LEFT_TOP,
            &rung.comment,
            FontId::proportional(11.0),
            DIM,
        );
    }
    if queries::rung_has_problem(app.project(), app.editor.problems(), rung.id) {
        let rows = rung.row_count();
        let max_col = rung.elements.iter().map(|e| e.col).max().unwrap_or(0);
        let layout = Layout::new(base + app.camera.pan, app.camera.zoom);
        let top_left = layout::cell_rect(0, 0, layout).min;
        let bottom_right = layout::cell_rect(max_col, rows.saturating_sub(1), layout).max;
        painter.rect_stroke(
            Rect::from_min_max(top_left, bottom_right),
            CornerRadius::same(3),
            Stroke::new(2.0_f32, PROBLEM),
            StrokeKind::Outside,
        );
        painter.text(
            Pos2::new(base.x, rect.top() + 7.0),
            Align2::LEFT_TOP,
            "!",
            FontId::proportional(14.0),
            PROBLEM,
        );
    }
}

/// Draws the placeholder shown when there is no rung to draw.
fn paint_notice(painter: &Painter, rect: Rect, text: &str) {
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        FontId::proportional(14.0),
        DIM,
    );
}

/// Draws one element, in the live or dead colour.
fn paint_element(
    app: &EditorApp,
    painter: &Painter,
    layout: Layout,
    element: &PlacedElement,
    energised: bool,
    selected: bool,
) {
    let rect = layout::cell_rect(element.col, element.row, layout);
    let zoom = layout.zoom();
    let live = if energised { LIVE } else { DEAD };
    let width: f32 = if energised { 2.6 } else { 1.4 };
    let stroke = Stroke::new(width, live);
    let font = FontId::monospace((10.0 * zoom).max(7.0));
    let height = rect.height() * 0.18;
    let y = rect.center().y;

    if element.connected_with_top && element.row > 0 {
        let above = layout::cell_rect(element.col, element.row - 1, layout);
        painter.line_segment(
            [
                Pos2::new(rect.center().x, above.center().y),
                Pos2::new(rect.center().x, y),
            ],
            stroke,
        );
    }

    match element.kind {
        ElementKind::ContactNo
        | ElementKind::ContactNc
        | ElementKind::ContactRising
        | ElementKind::ContactFalling => {
            let gap = rect.width() * 0.17;
            let left = rect.center().x - gap;
            let right = rect.center().x + gap;
            painter.line_segment([Pos2::new(rect.left(), y), Pos2::new(left, y)], stroke);
            painter.line_segment([Pos2::new(right, y), Pos2::new(rect.right(), y)], stroke);
            painter.line_segment(
                [Pos2::new(left, y - height), Pos2::new(left, y + height)],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(right, y - height), Pos2::new(right, y + height)],
                stroke,
            );
            match element.kind {
                ElementKind::ContactNc => {
                    painter.line_segment(
                        [
                            Pos2::new(left - 2.0, y + height),
                            Pos2::new(right + 2.0, y - height),
                        ],
                        stroke,
                    );
                }
                ElementKind::ContactRising => {
                    painter.text(
                        Pos2::new(rect.center().x, y),
                        Align2::CENTER_CENTER,
                        "P",
                        font.clone(),
                        live,
                    );
                }
                ElementKind::ContactFalling => {
                    painter.text(
                        Pos2::new(rect.center().x, y),
                        Align2::CENTER_CENTER,
                        "N",
                        font.clone(),
                        live,
                    );
                }
                _ => {}
            }
        }
        ElementKind::CoilOut
        | ElementKind::CoilOutNeg
        | ElementKind::CoilSet
        | ElementKind::CoilReset
        | ElementKind::CoilJump
        | ElementKind::CoilCall => {
            let radius = rect.height() * 0.21;
            let cx = rect.center().x;
            painter.line_segment(
                [Pos2::new(rect.left(), y), Pos2::new(cx - radius, y)],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(cx + radius, y), Pos2::new(rect.right(), y)],
                stroke,
            );
            painter.circle_stroke(Pos2::new(cx, y), radius, stroke);
            match element.kind {
                ElementKind::CoilOutNeg => {
                    painter.line_segment(
                        [
                            Pos2::new(cx - radius - 2.0, y + radius),
                            Pos2::new(cx + radius + 2.0, y - radius),
                        ],
                        stroke,
                    );
                }
                ElementKind::CoilSet => {
                    painter.text(
                        Pos2::new(cx, y),
                        Align2::CENTER_CENTER,
                        "S",
                        font.clone(),
                        live,
                    );
                }
                ElementKind::CoilReset => {
                    painter.text(
                        Pos2::new(cx, y),
                        Align2::CENTER_CENTER,
                        "R",
                        font.clone(),
                        live,
                    );
                }
                ElementKind::CoilJump => {
                    painter.text(
                        Pos2::new(cx, y),
                        Align2::CENTER_CENTER,
                        "J",
                        font.clone(),
                        live,
                    );
                }
                ElementKind::CoilCall => {
                    painter.text(
                        Pos2::new(cx, y),
                        Align2::CENTER_CENTER,
                        "C",
                        font.clone(),
                        live,
                    );
                }
                _ => {}
            }
        }
        ElementKind::Connection => {
            painter.circle_filled(rect.center(), (rect.height() * 0.10).max(2.0), live);
        }
        ElementKind::Timer { .. }
        | ElementKind::Counter { .. }
        | ElementKind::Register { .. }
        | ElementKind::Compare
        | ElementKind::Operate => {
            let inner = rect.shrink2(Vec2::new(2.0, 3.0));
            painter.rect_stroke(inner, CornerRadius::same(3), stroke, StrokeKind::Inside);
            let title = element.var.as_ref().map_or_else(
                || palette::short_name(element.kind).to_owned(),
                ToString::to_string,
            );
            painter.text(
                inner.center_top() + Vec2::new(0.0, 2.0),
                Align2::CENTER_TOP,
                title,
                font.clone(),
                live,
            );
            painter.text(
                inner.center_bottom() - Vec2::new(0.0, 2.0),
                Align2::CENTER_BOTTOM,
                block_detail(app, element),
                font.clone(),
                if energised { live } else { DIM },
            );
        }
    }
    paint_var(painter, rect, element, &font, live);
    if selected {
        painter.rect_stroke(
            rect,
            CornerRadius::same(2),
            Stroke::new(2.0_f32, SELECT),
            StrokeKind::Outside,
        );
    }
}

/// The second line inside a function block: preset, elapsed, count or operands.
fn block_detail(app: &EditorApp, element: &PlacedElement) -> String {
    let preset = element.params.first().cloned().unwrap_or_default();
    match element.kind {
        ElementKind::Timer { .. } => {
            let elapsed = block_of(app, element, Accessor::Value);
            format!(
                "{}/{}",
                queries::value_text(elapsed),
                if preset.is_empty() { "—" } else { &preset }
            )
        }
        ElementKind::Counter { .. } => {
            let value = block_of(app, element, Accessor::Value);
            format!(
                "{}/{}",
                queries::value_text(value),
                if preset.is_empty() { "—" } else { &preset }
            )
        }
        ElementKind::Register { .. } => {
            let count = block_of(app, element, Accessor::Count);
            format!(
                "{} held/{}",
                queries::value_text(count),
                if preset.is_empty() { "—" } else { &preset }
            )
        }
        _ => element.params.join(" "),
    }
}

/// Draws the element's variable under the cell.
fn paint_var(painter: &Painter, rect: Rect, element: &PlacedElement, font: &FontId, live: Color32) {
    let Some(var) = element.var.as_ref() else {
        return;
    };
    painter.text(
        rect.center_bottom() + Vec2::new(0.0, 1.0),
        Align2::CENTER_TOP,
        var.to_string(),
        font.clone(),
        live,
    );
}

/// Names the element kind for the palette tooltip and the properties strip.
pub fn describe(kind: ElementKind) -> &'static str {
    match kind {
        ElementKind::Timer {
            mode: TimerMode::On,
        } => "TON",
        ElementKind::Timer {
            mode: TimerMode::Off,
        } => "TOF",
        ElementKind::Timer {
            mode: TimerMode::Pulse,
        } => "TP",
        ElementKind::Counter {
            kind: CounterKind::Up,
        } => "CTU",
        ElementKind::Counter {
            kind: CounterKind::Down,
        } => "CTD",
        ElementKind::Counter {
            kind: CounterKind::UpDown,
        } => "CTUD",
        ElementKind::Register {
            mode: RegisterMode::Fifo,
        } => "FIFO",
        ElementKind::Register {
            mode: RegisterMode::Lifo,
        } => "LIFO",
        other => palette::short_name(other),
    }
}
