//! Pure canvas geometry: cell ↔ pixel mapping and the pan/zoom camera.
//!
//! Nothing here touches `egui`'s context or any application state, so the whole
//! mapping between grid coordinates and screen points is unit-tested without a
//! window (see [`cell_rect`] and [`cell_at`]).

use egui::{Pos2, Rect, Vec2};

/// Width of one cell in points at zoom `1.0`.
pub const BASE_CELL_W: f32 = 44.0;

/// Height of one cell in points at zoom `1.0`.
///
/// Taller than wide, because a function block draws its variable *and* its
/// preset inside the cell.
pub const BASE_CELL_H: f32 = 54.0;

/// Highest column the canvas addresses.
///
/// The grid coordinates are `u8` and every edit round-trips through
/// `softladder-core`'s model, so the canvas refuses to address a cell it could
/// not represent.
pub const MAX_COL: u8 = 63;

/// Highest row the canvas addresses.
pub const MAX_ROW: u8 = 31;

/// Smallest zoom factor the camera accepts.
pub const MIN_ZOOM: f32 = 0.25;

/// Largest zoom factor the camera accepts.
pub const MAX_ZOOM: f32 = 4.0;

/// Clamps `zoom` into [`MIN_ZOOM`]..=[`MAX_ZOOM`].
///
/// A non-finite factor (which a degenerate scroll delta can produce) falls back
/// to `1.0`, so the camera can never become unusable.
pub fn clamp_zoom(zoom: f32) -> f32 {
    if !zoom.is_finite() {
        return 1.0;
    }
    zoom.clamp(MIN_ZOOM, MAX_ZOOM)
}

/// Where the grid sits on the canvas and how big one cell is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
    /// Screen position of the top-left corner of cell `(0, 0)`.
    pub origin: Pos2,
    /// Size of one cell in points, already scaled by the zoom.
    pub cell: Vec2,
}

impl Layout {
    /// A layout for `origin` at zoom factor `zoom`.
    pub fn new(origin: Pos2, zoom: f32) -> Self {
        let zoom = clamp_zoom(zoom);
        Self {
            origin,
            cell: Vec2::new(BASE_CELL_W * zoom, BASE_CELL_H * zoom),
        }
    }

    /// A layout with an explicit cell size.
    pub fn with_cell(origin: Pos2, cell: Vec2) -> Self {
        Self { origin, cell }
    }

    /// Zoom factor this layout was built with (relative to [`BASE_CELL_W`]).
    pub fn zoom(self) -> f32 {
        if BASE_CELL_W > 0.0 {
            self.cell.x / BASE_CELL_W
        } else {
            1.0
        }
    }
}

/// Pixel rectangle of cell `(col, row)`.
///
/// The rectangle is exactly one cell of the layout, so a caller that inverts it
/// with [`cell_at`] gets `(col, row)` back.
pub fn cell_rect(col: u8, row: u8, layout: Layout) -> Rect {
    let min = Pos2::new(
        layout.origin.x + f32::from(col) * layout.cell.x,
        layout.origin.y + f32::from(row) * layout.cell.y,
    );
    Rect::from_min_size(min, layout.cell)
}

/// The cell under `pos`, or `None` when `pos` is outside the addressable grid.
///
/// A cell size of zero or less, a non-finite position and every column past
/// [`MAX_COL`] or row past [`MAX_ROW`] all yield `None` instead of a bogus cell.
pub fn cell_at(pos: Pos2, layout: Layout) -> Option<(u8, u8)> {
    let positive = layout.cell.x > 0.0 && layout.cell.y > 0.0;
    if !positive {
        return None;
    }
    let col = (pos.x - layout.origin.x) / layout.cell.x;
    let row = (pos.y - layout.origin.y) / layout.cell.y;
    if !col.is_finite() || !row.is_finite() {
        return None;
    }
    let (col, row) = (col.floor(), row.floor());
    if col < 0.0 || row < 0.0 || col > f32::from(MAX_COL) || row > f32::from(MAX_ROW) {
        return None;
    }
    Some((col as u8, row as u8))
}

/// Pan and zoom of the rung canvas.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    /// Zoom factor, always inside [`MIN_ZOOM`]..=[`MAX_ZOOM`].
    pub zoom: f32,
    /// Pan offset in points, relative to the canvas origin.
    pub pan: Vec2,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            pan: Vec2::ZERO,
        }
    }
}

impl Camera {
    /// The camera as it starts: zoom `1.0`, no pan.
    pub fn new() -> Self {
        Self::default()
    }

    /// Multiplies the zoom by `factor`, clamped, and clears a non-finite pan.
    pub fn zoom_by(&mut self, factor: f32) {
        self.set_zoom(self.zoom * factor);
    }

    /// Sets the zoom, clamped into the accepted range.
    pub fn set_zoom(&mut self, zoom: f32) {
        self.zoom = clamp_zoom(zoom);
        if !self.pan.x.is_finite() || !self.pan.y.is_finite() {
            self.pan = Vec2::ZERO;
        }
    }

    /// Zooms one step in.
    pub fn zoom_in(&mut self) {
        self.zoom_by(1.25);
    }

    /// Zooms one step out.
    pub fn zoom_out(&mut self) {
        self.zoom_by(1.0 / 1.25);
    }

    /// Restores zoom `1.0` and no pan.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// `true` when the camera is exactly as it started.
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// Moves the camera by a drag delta.
    pub fn pan_by(&mut self, delta: Vec2) {
        self.pan += delta;
        if !self.pan.x.is_finite() || !self.pan.y.is_finite() {
            self.pan = Vec2::ZERO;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> Layout {
        Layout::new(Pos2::new(10.0, 20.0), 1.0)
    }

    #[test]
    fn cell_rect_places_cells_on_the_grid() {
        let layout = layout();
        let first = cell_rect(0, 0, layout);
        assert_eq!(first.min, Pos2::new(10.0, 20.0));
        assert_eq!(first.width(), BASE_CELL_W);
        assert_eq!(first.height(), BASE_CELL_H);

        let second = cell_rect(2, 1, layout);
        assert_eq!(
            second.min,
            Pos2::new(10.0 + 2.0 * BASE_CELL_W, 20.0 + BASE_CELL_H)
        );
        assert_eq!(second.size(), layout.cell);
    }

    #[test]
    fn cell_at_inverts_cell_rect_inside_the_grid() {
        let layout = layout();
        for (col, row) in [(0_u8, 0_u8), (1, 0), (0, 3), (7, 5), (MAX_COL, MAX_ROW)] {
            let rect = cell_rect(col, row, layout);
            assert_eq!(cell_at(rect.min, layout), Some((col, row)));
            assert_eq!(cell_at(rect.center(), layout), Some((col, row)));
            // Just inside the far corner is still the same cell.
            let inside = Pos2::new(rect.right() - 0.5, rect.bottom() - 0.5);
            assert_eq!(cell_at(inside, layout), Some((col, row)));
        }
    }

    #[test]
    fn cell_at_rejects_points_outside_the_grid() {
        let layout = layout();
        assert_eq!(cell_at(Pos2::new(0.0, 0.0), layout), None, "left of origin");
        assert_eq!(
            cell_at(Pos2::new(9.9, 25.0), layout),
            None,
            "one point left"
        );
        assert_eq!(cell_at(Pos2::new(25.0, 19.9), layout), None, "above origin");

        let far = Pos2::new(
            layout.origin.x + f32::from(MAX_COL + 1) * BASE_CELL_W + 1.0,
            layout.origin.y,
        );
        assert_eq!(cell_at(far, layout), None, "past the last column");

        let low = Pos2::new(layout.origin.x, layout.origin.y + BASE_CELL_H * 40.0);
        assert_eq!(cell_at(low, layout), None, "past the last row");

        assert_eq!(cell_at(Pos2::new(f32::NAN, 0.0), layout), None);
        assert_eq!(
            cell_at(
                Pos2::new(10.0, 20.0),
                Layout::with_cell(Pos2::ZERO, Vec2::ZERO)
            ),
            None,
            "a degenerate cell size addresses nothing"
        );
    }

    #[test]
    fn zoom_scales_the_cell_size_and_the_mapping_together() {
        let layout = Layout::new(Pos2::ZERO, 2.0);
        assert_eq!(layout.cell, Vec2::new(BASE_CELL_W * 2.0, BASE_CELL_H * 2.0));
        assert_eq!(layout.zoom(), 2.0);
        let rect = cell_rect(1, 1, layout);
        assert_eq!(cell_at(rect.center(), layout), Some((1, 1)));
        assert_eq!(
            cell_at(Pos2::new(BASE_CELL_W * 1.5, 0.0), layout),
            Some((0, 0))
        );
    }

    #[test]
    fn clamp_zoom_bounds_and_recovers_from_nonsense() {
        assert_eq!(clamp_zoom(1.0), 1.0);
        assert_eq!(clamp_zoom(0.0), MIN_ZOOM);
        assert_eq!(clamp_zoom(-3.0), MIN_ZOOM);
        assert_eq!(clamp_zoom(100.0), MAX_ZOOM);
        assert_eq!(clamp_zoom(f32::NAN), 1.0);
        assert_eq!(clamp_zoom(f32::INFINITY), 1.0);
    }

    #[test]
    fn the_camera_clamps_zoom_and_resets() {
        let mut camera = Camera::new();
        assert!(camera.is_default());

        camera.zoom_in();
        assert!(camera.zoom > 1.0);
        for _ in 0..40 {
            camera.zoom_in();
        }
        assert_eq!(camera.zoom, MAX_ZOOM);
        for _ in 0..80 {
            camera.zoom_out();
        }
        assert_eq!(camera.zoom, MIN_ZOOM);

        camera.pan_by(Vec2::new(12.0, -4.0));
        assert_eq!(camera.pan, Vec2::new(12.0, -4.0));
        camera.reset();
        assert!(camera.is_default());

        camera.set_zoom(f32::NAN);
        assert_eq!(camera.zoom, 1.0);
    }

    #[test]
    fn a_non_finite_pan_falls_back_to_zero() {
        let mut camera = Camera::new();
        camera.pan_by(Vec2::new(f32::INFINITY, 0.0));
        assert_eq!(camera.pan, Vec2::ZERO);
        camera.pan = Vec2::new(f32::NAN, 1.0);
        camera.set_zoom(2.0);
        assert_eq!(camera.pan, Vec2::ZERO);
    }
}
