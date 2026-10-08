//! The IEC glyphs, drawn as schematics.
//!
//! One implementation serves both the palette (as small icons) and the ladder
//! canvas (at cell size, with labels, pins and live state), so an element never
//! looks like two different things depending on where it is. See `docs/UX.md` §5.
//!
//! Everything is expressed in fractions or absolute offsets from the rectangle it
//! is given, so the same code draws an 18 px icon and a 96 px cell.

use egui::{
    Align2, Color32, CornerRadius, FontFamily, FontId, Painter, Pos2, Rect, Shape, Stroke, Vec2,
};

use softladder_core::ElementKind;

use crate::design::{Tokens, TypeScale, RADIUS_CONTROL};

/// Colours and strokes of one drawn symbol.
#[derive(Debug, Clone, Copy)]
pub struct Style {
    /// The horizontal wire.
    pub wire: Stroke,
    /// The glyph itself (contact bars, coil arcs, boxes).
    pub symbol: Stroke,
    /// Fill of a block body when the cell is energised.
    pub fill: Color32,
    /// Text drawn inside or next to the glyph.
    pub text: Color32,
}

impl Style {
    /// The style of an idle symbol on the paper.
    pub fn idle(tokens: &Tokens) -> Self {
        Self {
            wire: Stroke::new(2.0_f32, tokens.wire_idle),
            symbol: Stroke::new(2.0_f32, tokens.wire_idle),
            fill: Color32::TRANSPARENT,
            text: tokens.text,
        }
    }

    /// The style of a symbol whose cell is energised.
    pub fn live(tokens: &Tokens) -> Self {
        Self {
            wire: Stroke::new(3.0_f32, tokens.energised),
            symbol: Stroke::new(2.5_f32, tokens.energised),
            fill: tokens.energised.gamma_multiply(0.10),
            text: tokens.text,
        }
    }

    /// The same style, energised or not.
    pub fn with_live(self, tokens: &Tokens, energised: bool) -> Self {
        if energised {
            Self::live(tokens)
        } else {
            Self::idle(tokens)
        }
        .with_text(self.text)
    }

    /// The same style with an explicit text colour.
    pub fn with_text(mut self, text: Color32) -> Self {
        self.text = text;
        self
    }

    /// The colour of the glyph and the wire.
    pub fn colour(&self) -> Color32 {
        self.symbol.color
    }
}

/// Which coil glyph to draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Coil {
    /// `-( )-`
    Out,
    /// `-(/)-`
    OutNeg,
    /// `-(S)-`
    Set,
    /// `-(R)-`
    Reset,
    /// `-(J)-`
    Jump,
    /// `-(C)-`
    Call,
}

/// The coil glyph for an element kind, if it is a coil.
pub fn coil_of(kind: ElementKind) -> Option<Coil> {
    match kind {
        ElementKind::CoilOut => Some(Coil::Out),
        ElementKind::CoilOutNeg => Some(Coil::OutNeg),
        ElementKind::CoilSet => Some(Coil::Set),
        ElementKind::CoilReset => Some(Coil::Reset),
        ElementKind::CoilJump => Some(Coil::Jump),
        ElementKind::CoilCall => Some(Coil::Call),
        _ => None,
    }
}

/// Half-height of a contact's bars, as a fraction of the rect height.
const BAR_HALF: f32 = 0.30;
/// Half-width of the gap between a contact's bars, as a fraction of the width.
const BAR_GAP: f32 = 0.10;

/// A horizontal wire across the whole rectangle.
pub fn wire(painter: &Painter, rect: Rect, style: Style) {
    painter.hline(rect.x_range(), rect.center().y, style.wire);
}

/// Two vertical bars with a gap between them: the body of every contact.
fn bars(painter: &Painter, rect: Rect, style: Style, gap: f32) {
    let y = rect.center().y;
    let half = (rect.height() * BAR_HALF).max(4.0);
    for x in [rect.center().x - gap, rect.center().x + gap] {
        painter.vline(x, (y - half)..=(y + half), style.symbol);
    }
}

/// A contact: wire, bars, an optional negating slash and an optional marker.
fn contact(painter: &Painter, rect: Rect, style: Style, negated: bool, marker: Option<&str>) {
    let y = rect.center().y;
    let gap = (rect.width() * BAR_GAP).max(3.0);
    painter.hline(rect.x_range().min..=(rect.center().x - gap), y, style.wire);
    painter.hline((rect.center().x + gap)..=rect.x_range().max, y, style.wire);
    bars(painter, rect, style, gap);
    if negated {
        let half = (rect.height() * BAR_HALF).max(4.0);
        painter.line_segment(
            [
                Pos2::new(rect.center().x - gap, y + half),
                Pos2::new(rect.center().x + gap, y - half),
            ],
            style.symbol,
        );
    }
    if let Some(marker) = marker {
        painter.text(
            Pos2::new(rect.center().x, y),
            Align2::CENTER_CENTER,
            marker,
            FontId::new(
                (rect.height() * 0.44).clamp(7.0, 15.0),
                FontFamily::Proportional,
            ),
            style.text,
        );
    }
}

/// `-| |-`
pub fn contact_no(painter: &Painter, rect: Rect, style: Style) {
    contact(painter, rect, style, false, None);
}

/// `-|/|-`
pub fn contact_nc(painter: &Painter, rect: Rect, style: Style) {
    contact(painter, rect, style, true, None);
}

/// `-|P|-`
pub fn contact_rising(painter: &Painter, rect: Rect, style: Style) {
    contact(painter, rect, style, false, Some("P"));
}

/// `-|N|-`
pub fn contact_falling(painter: &Painter, rect: Rect, style: Style) {
    contact(painter, rect, style, false, Some("N"));
}

/// A coil: `-( )-`, `-(/)-`, `-(S)-`, `-(R)-`, `-(J)-`, `-(C)-`.
///
/// The two arcs face outwards, so the glyph reads as a coil rather than as a box.
pub fn coil(painter: &Painter, rect: Rect, style: Style, coil: Coil) {
    let y = rect.center().y;
    let radius = (rect.height() * 0.30).max(5.0);
    let centre = rect.center().x;
    painter.hline(rect.x_range().min..=(centre - radius), y, style.wire);
    painter.hline((centre + radius)..=rect.x_range().max, y, style.wire);

    let bulge = radius * 0.55;
    for direction in [-1.0f32, 1.0] {
        let at = centre + direction * radius;
        let arc: Vec<Pos2> = (0..=16)
            .map(|step| {
                let t = step as f32 / 16.0 * 2.0 - 1.0;
                Pos2::new(at + direction * bulge * (1.0 - t * t), y + radius * t)
            })
            .collect();
        painter.add(Shape::line(arc, style.symbol));
    }

    let marker = match coil {
        Coil::Out => None,
        Coil::OutNeg => Some("/"),
        Coil::Set => Some("S"),
        Coil::Reset => Some("R"),
        Coil::Jump => Some("J"),
        Coil::Call => Some("C"),
    };
    if let Some(marker) = marker {
        painter.text(
            Pos2::new(centre, y - radius - 1.0),
            Align2::CENTER_BOTTOM,
            marker,
            FontId::new(
                (rect.height() * 0.34).clamp(7.0, 12.0),
                FontFamily::Proportional,
            ),
            style.text,
        );
    }
}

/// The body rectangle of a function block drawn in `rect`.
///
/// [`block`] draws the box, the pins and the wire stubs from this rectangle, and
/// a caller that annotates a block with its live value ([`block_value`], the
/// ladder canvas) needs exactly the same box.
pub fn block_body(rect: Rect) -> Rect {
    Rect::from_min_max(
        Pos2::new(
            rect.left() + rect.width() * BLOCK_INSET_X,
            rect.top() + rect.height() * BLOCK_TOP,
        ),
        Pos2::new(
            rect.right() - rect.width() * BLOCK_INSET_X,
            rect.bottom() - rect.height() * BLOCK_BOTTOM,
        ),
    )
}

/// The rectangle to hand [`block`] so that its input pins sit on a grid.
///
/// [`block`] spreads its pins evenly over the body, and the body is
/// [`BLOCK_TOP`]`..`[`BLOCK_BOTTOM`] of the rectangle it is given. Solving that
/// back is what lets the canvas put a counter's `R`, `LD`, `CU` and `CD` pins
/// exactly on the four rows the engine reads them from, so the wires that feed
/// them meet the box where they should. `first_pin_y` is where the first pin
/// goes, `row_pitch` the distance between pins and `span` the number of pins.
pub fn aligned_block_rect(
    left: f32,
    width: f32,
    first_pin_y: f32,
    span: u8,
    row_pitch: f32,
) -> Rect {
    let span = f32::from(span.max(1));
    let height = span * row_pitch / BLOCK_BODY;
    let body_top = first_pin_y - row_pitch * 0.5;
    Rect::from_min_size(
        Pos2::new(left, body_top - height * BLOCK_TOP),
        Vec2::new(width, height),
    )
}

/// Horizontal inset of a block body, as a fraction of the rectangle.
const BLOCK_INSET_X: f32 = 0.08;
/// Where a block body starts, as a fraction of the rectangle's height.
const BLOCK_TOP: f32 = 0.14;
/// How far a block body stops short of the rectangle's bottom.
const BLOCK_BOTTOM: f32 = 0.14;
/// Height of a block body, as a fraction of the rectangle's height.
const BLOCK_BODY: f32 = 1.0 - BLOCK_TOP - BLOCK_BOTTOM;

/// A function block: the type inside the box, named pins on both sides.
///
/// `pins` are `(label, side)` pairs, `side` negative for an input.
pub fn block(painter: &Painter, rect: Rect, style: Style, title: &str, pins: &[(&str, i32)]) {
    let body = block_body(rect);
    painter.rect(
        body,
        CornerRadius::same(RADIUS_CONTROL),
        style.fill,
        style.symbol,
        egui::StrokeKind::Middle,
    );
    painter.hline(
        rect.x_range().min..=body.left(),
        rect.center().y,
        style.wire,
    );
    painter.hline(
        body.right()..=rect.x_range().max,
        rect.center().y,
        style.wire,
    );
    painter.text(
        body.center(),
        Align2::CENTER_CENTER,
        title,
        FontId::new(
            (rect.height() * 0.30).clamp(7.0, 13.0),
            FontFamily::Proportional,
        ),
        style.text,
    );

    let inputs: Vec<&str> = pins
        .iter()
        .filter(|(_, side)| *side < 0)
        .map(|(label, _)| *label)
        .collect();
    let outputs: Vec<&str> = pins
        .iter()
        .filter(|(_, side)| *side >= 0)
        .map(|(label, _)| *label)
        .collect();
    let font = FontId::new(
        (rect.height() * 0.20).clamp(6.0, 10.0),
        FontFamily::Proportional,
    );
    let slot = |count: usize, index: usize| -> f32 {
        let count = count.max(1) as f32;
        (index as f32 + 0.5) / count
    };
    for (index, label) in inputs.iter().enumerate() {
        let pin_y = body.top() + body.height() * slot(inputs.len(), index);
        painter.hline(
            (body.left() - rect.width() * 0.08)..=body.left(),
            pin_y,
            style.wire,
        );
        painter.text(
            Pos2::new(body.left() + 3.0, pin_y - 1.0),
            Align2::LEFT_BOTTOM,
            *label,
            font.clone(),
            style.text.gamma_multiply(0.85),
        );
    }
    for (index, label) in outputs.iter().enumerate() {
        let pin_y = body.top() + body.height() * slot(outputs.len(), index);
        painter.hline(
            body.right()..=(body.right() + rect.width() * 0.08),
            pin_y,
            style.wire,
        );
        painter.text(
            Pos2::new(body.right() - 3.0, pin_y - 1.0),
            Align2::RIGHT_BOTTOM,
            *label,
            font.clone(),
            style.text.gamma_multiply(0.85),
        );
    }
}

/// The live value of a function block, drawn inside its body under the type.
///
/// A timer shows `ET/PT`, a counter `CV/PV` and a register `count/capacity`; the
/// line is elided to the body, so a long reading never escapes the box. The
/// canvas calls this only while the bench has a live picture, which is what
/// keeps the value from flickering.
pub fn block_value(painter: &Painter, rect: Rect, style: Style, value: &str) {
    let body = block_body(rect);
    if value.is_empty() || body.width() <= 6.0 || body.height() <= 6.0 {
        return;
    }
    let font = FontId::new(
        (rect.height() * 0.22).clamp(6.0, 10.0),
        FontFamily::Monospace,
    );
    let colour = style.text.gamma_multiply(0.9);
    let galley = layout_clipped(painter, value, font, colour, (body.width() - 4.0).max(4.0));
    let at = Pos2::new(
        body.center().x - galley.size().x / 2.0,
        body.bottom() - galley.size().y - 3.0,
    );
    painter
        .with_clip_rect(body.shrink(2.0))
        .galley(at, galley, colour);
}

/// One row of text, elided with `…` when it does not fit `width`.
fn layout_clipped(
    painter: &Painter,
    text: &str,
    font: FontId,
    colour: Color32,
    width: f32,
) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple(text.to_owned(), font, colour, width);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    painter.layout_job(job)
}

/// A compare or operate box showing its expression, clipped to the box.
pub fn expression_box(painter: &Painter, rect: Rect, style: Style, text: &str) {
    let body = Rect::from_min_max(
        Pos2::new(
            rect.left() + rect.width() * 0.05,
            rect.top() + rect.height() * 0.16,
        ),
        Pos2::new(
            rect.right() - rect.width() * 0.05,
            rect.bottom() - rect.height() * 0.16,
        ),
    );
    painter.rect(
        body,
        CornerRadius::same(RADIUS_CONTROL),
        style.fill,
        style.symbol,
        egui::StrokeKind::Middle,
    );
    painter.hline(
        rect.x_range().min..=body.left(),
        rect.center().y,
        style.wire,
    );
    painter.hline(
        body.right()..=rect.x_range().max,
        rect.center().y,
        style.wire,
    );
    let font = FontId::new(
        (rect.height() * 0.24).clamp(6.0, 11.0),
        FontFamily::Monospace,
    );
    let galley = painter.layout_no_wrap(text.to_owned(), font, style.text);
    let clipped = painter.with_clip_rect(body.shrink(3.0));
    clipped.galley(
        Pos2::new(body.left() + 3.0, body.center().y - galley.size().y / 2.0),
        galley,
        style.text,
    );
}

/// Draws the icon of an element kind, for the palette and the project tree.
pub fn element_glyph(painter: &Painter, rect: Rect, tokens: &Tokens, kind: ElementKind) {
    let style = Style {
        wire: Stroke::new(1.6_f32, tokens.wire_idle),
        symbol: Stroke::new(1.6_f32, tokens.wire_idle),
        fill: Color32::TRANSPARENT,
        text: tokens.text,
    };
    if let Some(glyph) = coil_of(kind) {
        coil(painter, rect, style, glyph);
        return;
    }
    match kind {
        ElementKind::ContactNo => contact_no(painter, rect, style),
        ElementKind::ContactNc => contact_nc(painter, rect, style),
        ElementKind::ContactRising => contact_rising(painter, rect, style),
        ElementKind::ContactFalling => contact_falling(painter, rect, style),
        ElementKind::Connection => wire(painter, rect, style),
        ElementKind::Timer { .. } => block(painter, rect, style, "TON", &[("IN", -1), ("Q", 1)]),
        ElementKind::Counter { .. } => block(painter, rect, style, "CTU", &[("CU", -1), ("Q", 1)]),
        ElementKind::Register { .. } => {
            block(painter, rect, style, "FIFO", &[("IN", -1), ("E", 1)])
        }
        ElementKind::Compare => expression_box(painter, rect, style, "a = b"),
        ElementKind::Operate => expression_box(painter, rect, style, "a := b"),
        _ => wire(painter, rect, style),
    }
}

/// The two-line label of an element: tag name above, address below.
///
/// `show_address` forces the address even when a name is present (the View menu
/// toggle); without a name the address is always drawn. The lines end just above
/// `rect.top()`, so the rectangle a caller passes is the glyph, not the cell.
pub fn element_label(
    painter: &Painter,
    rect: Rect,
    tokens: &Tokens,
    name: Option<&str>,
    address: &str,
    show_address: bool,
) {
    element_label_rect(painter, rect, tokens, name, address, show_address);
}

/// [`element_label`], returning the rectangle the drawn lines occupy.
///
/// The canvas hangs a live value chip next to the label with it, without
/// re-deriving the two-line layout the glyph owns.
pub fn element_label_rect(
    painter: &Painter,
    rect: Rect,
    tokens: &Tokens,
    name: Option<&str>,
    address: &str,
    show_address: bool,
) -> Rect {
    let mut lines: Vec<(String, FontId, Color32)> = Vec::new();
    if let Some(name) = name {
        lines.push((
            name.to_owned(),
            FontId::new(TypeScale::CAPTION, FontFamily::Proportional),
            tokens.text,
        ));
    }
    if show_address || name.is_none() {
        let colour = if name.is_some() {
            tokens.text_dim
        } else {
            tokens.text
        };
        lines.push((
            address.to_owned(),
            FontId::new(TypeScale::CAPTION, FontFamily::Monospace),
            colour,
        ));
    }
    let galleys: Vec<_> = lines
        .into_iter()
        .map(|(text, font, colour)| (painter.layout_no_wrap(text, font, colour), colour))
        .collect();
    let total: f32 = galleys.iter().map(|(galley, _)| galley.size().y).sum();
    let mut y = rect.top() - total - 2.0;
    let mut drawn = Rect::NOTHING;
    for (galley, colour) in galleys {
        let at = Pos2::new(rect.center().x - galley.size().x / 2.0, y);
        drawn = drawn.union(Rect::from_min_size(at, galley.size()));
        painter.galley(at, galley.clone(), colour);
        y += galley.size().y;
    }
    drawn
}

/// A value chip drawn next to a live element (`1`, `0`, `TRUE`, `142`).
pub fn value_chip(painter: &Painter, centre: Pos2, tokens: &Tokens, text: &str, live: bool) {
    let font = FontId::new(TypeScale::CAPTION, FontFamily::Monospace);
    let colour = if live {
        tokens.energised
    } else {
        tokens.text_dim
    };
    let galley = painter.layout_no_wrap(text.to_owned(), font, colour);
    let size = galley.size() + Vec2::new(6.0, 2.0);
    let rect = Rect::from_center_size(centre, size);
    painter.rect_filled(rect, CornerRadius::same(2), colour.gamma_multiply(0.14));
    painter.galley(rect.center() - galley.size() / 2.0, galley, colour);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paint(kind: ElementKind, tokens: &Tokens, rect: Rect) {
        let ctx = egui::Context::default();
        let _ = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let (rect, _) = ui.allocate_exact_size(rect.size(), egui::Sense::hover());
                element_glyph(ui.painter(), rect, tokens, kind);
                // And the full-size version with a label and a chip.
                element_label(
                    ui.painter(),
                    rect,
                    tokens,
                    Some("start_button"),
                    "%I0",
                    true,
                );
                value_chip(ui.painter(), rect.center(), tokens, "TRUE", true);
            });
        });
    }

    #[test]
    fn every_element_kind_has_a_glyph() {
        let tokens = Tokens::light();
        let kinds = [
            ElementKind::ContactNo,
            ElementKind::ContactNc,
            ElementKind::ContactRising,
            ElementKind::ContactFalling,
            ElementKind::CoilOut,
            ElementKind::CoilOutNeg,
            ElementKind::CoilSet,
            ElementKind::CoilReset,
            ElementKind::CoilJump,
            ElementKind::CoilCall,
            ElementKind::Timer {
                mode: softladder_core::TimerMode::On,
            },
            ElementKind::Counter {
                kind: softladder_core::CounterKind::Up,
            },
            ElementKind::Register {
                mode: softladder_core::RegisterMode::Fifo,
            },
            ElementKind::Compare,
            ElementKind::Operate,
            ElementKind::Connection,
        ];
        for kind in kinds {
            // A palette icon and a full cell: both must survive.
            paint(
                kind,
                &tokens,
                Rect::from_min_size(Pos2::ZERO, Vec2::new(18.0, 14.0)),
            );
            paint(
                kind,
                &tokens,
                Rect::from_min_size(Pos2::ZERO, Vec2::new(96.0, 64.0)),
            );
            // A degenerate rectangle must not panic either.
            paint(
                kind,
                &tokens,
                Rect::from_min_size(Pos2::ZERO, Vec2::new(1.0, 1.0)),
            );
        }
    }

    #[test]
    fn coils_map_to_their_element_kinds() {
        assert_eq!(coil_of(ElementKind::CoilOut), Some(Coil::Out));
        assert_eq!(coil_of(ElementKind::CoilOutNeg), Some(Coil::OutNeg));
        assert_eq!(coil_of(ElementKind::CoilSet), Some(Coil::Set));
        assert_eq!(coil_of(ElementKind::CoilReset), Some(Coil::Reset));
        assert_eq!(coil_of(ElementKind::CoilJump), Some(Coil::Jump));
        assert_eq!(coil_of(ElementKind::CoilCall), Some(Coil::Call));
        assert_eq!(coil_of(ElementKind::ContactNo), None);
        assert_eq!(coil_of(ElementKind::Compare), None);
    }

    #[test]
    fn the_live_style_is_thicker_and_green() {
        let tokens = Tokens::light();
        let idle = Style::idle(&tokens);
        let live = Style::live(&tokens);
        assert_eq!(idle.colour(), tokens.wire_idle);
        assert_eq!(live.colour(), tokens.energised);
        assert!(live.wire.width > idle.wire.width, "live wires are thicker");
        assert_ne!(live.fill, Color32::TRANSPARENT, "a live block is tinted");
        assert_eq!(idle.fill, Color32::TRANSPARENT);
        assert_eq!(idle.with_live(&tokens, true).colour(), tokens.energised);
        assert_eq!(live.with_live(&tokens, false).colour(), tokens.wire_idle);
    }
}
