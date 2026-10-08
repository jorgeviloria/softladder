//! The small interface icon set, drawn with `egui` paths.
//!
//! `docs/UX.md` §4 asks for one hand-drawn vector set on a 16 px grid, shared by
//! every panel: the project tree, the inspector and the status bar all use these,
//! and the *element* icons come from [`crate::symbols::element_glyph`] so an
//! instruction looks the same in the palette, in the tree and on the canvas.
//!
//! Nothing here is an image asset and nothing here allocates: every shape is
//! expressed as a fraction of the rectangle the caller hands over, so the same
//! code draws a 12 px status-bar glyph and a 20 px tree glyph.

use egui::{Align2, Color32, CornerRadius, FontFamily, FontId, Painter, Pos2, Rect, Shape, Stroke};

use crate::design::Tokens;

/// A small interface glyph, independent of the ladder elements.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    /// The programmable controller at the root of the project tree.
    Plc,
    /// A program (a folder of sections).
    Program,
    /// A section (a program block).
    Section,
    /// A rung (a network).
    Rung,
    /// The PLC tag table.
    Tags,
    /// The simulation bench.
    Bench,
    /// The watch and force table.
    Watch,
    /// The diagnostics list.
    Problems,
    /// An error.
    Error,
    /// A warning.
    Warning,
    /// Informational.
    Info,
    /// The pointer tool.
    Pointer,
    /// A chevron pointing right, for a collapsed node.
    ChevronRight,
    /// A chevron pointing down, for an expanded node.
    ChevronDown,
}

/// The number of points a tree row icon is drawn in.
pub const SMALL: f32 = 14.0;

/// Draws `icon` centred in `rect` using `tokens` for its colours.
///
/// A degenerate `rect` (zero or negative width or height) is drawn as nothing
/// rather than as a panicking shape.
pub fn draw(painter: &Painter, rect: Rect, tokens: &Tokens, icon: Icon) {
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
    }
    let stroke = Stroke::new(1.4_f32, colour(tokens, icon));
    let ink = colour(tokens, icon);
    // A local coordinate helper: `(0, 0)` is the top-left of the icon, `(1, 1)`
    // the bottom-right.
    let at = |x: f32, y: f32| {
        Pos2::new(
            rect.left() + rect.width() * x,
            rect.top() + rect.height() * y,
        )
    };
    match icon {
        Icon::Plc => {
            let body = Rect::from_min_max(at(0.08, 0.20), at(0.92, 0.80));
            painter.rect(
                body,
                CornerRadius::same(1),
                Color32::TRANSPARENT,
                stroke,
                egui::StrokeKind::Middle,
            );
            painter.hline(
                body.x_range(),
                body.top() + body.height() * 0.30,
                Stroke::new(1.0_f32, ink),
            );
            for x in [0.28_f32, 0.5, 0.72] {
                painter.circle_filled(at(x, 0.32), (rect.width() * 0.045).max(1.0), ink);
            }
            // The pin header along the top, as a PLC rack draws it.
            painter.rect_filled(
                Rect::from_min_max(at(0.30, 0.08), at(0.70, 0.18)),
                CornerRadius::same(1),
                ink,
            );
        }
        Icon::Program => {
            let body = Rect::from_min_max(at(0.14, 0.16), at(0.86, 0.86));
            painter.rect(
                body,
                CornerRadius::same(1),
                Color32::TRANSPARENT,
                stroke,
                egui::StrokeKind::Middle,
            );
            for y in [0.36_f32, 0.5, 0.64] {
                painter.hline(
                    (body.left() + body.width() * 0.18)..=(body.right() - body.width() * 0.18),
                    at(0.0, y).y,
                    Stroke::new(1.0_f32, ink),
                );
            }
        }
        Icon::Section => {
            painter.rect(
                Rect::from_min_max(at(0.12, 0.14), at(0.88, 0.86)),
                CornerRadius::same(1),
                Color32::TRANSPARENT,
                stroke,
                egui::StrokeKind::Middle,
            );
            painter.rect_filled(
                Rect::from_min_max(at(0.20, 0.24), at(0.56, 0.40)),
                CornerRadius::same(1),
                ink,
            );
        }
        Icon::Rung => {
            let y = 0.5_f32;
            painter.hline(
                at(0.06, y).x..=at(0.94, y).x,
                at(0.0, y).y,
                Stroke::new(1.4_f32, ink),
            );
            for x in [0.34_f32, 0.50] {
                painter.vline(at(x, 0.0).x, (at(0.0, 0.22).y)..=(at(0.0, 0.78).y), stroke);
            }
            painter.circle_stroke(at(0.78, y), (rect.height() * 0.22).max(1.5), stroke);
        }
        Icon::Tags => {
            // A tag: rounded label with a punched hole.
            let body = Rect::from_min_max(at(0.10, 0.24), at(0.90, 0.76));
            painter.rect(
                body,
                CornerRadius::same(2),
                Color32::TRANSPARENT,
                stroke,
                egui::StrokeKind::Middle,
            );
            painter.circle_filled(at(0.26, 0.5), (rect.width() * 0.07).max(1.0), ink);
            painter.hline(
                at(0.42, 0.5).x..=at(0.78, 0.5).x,
                at(0.0, 0.5).y,
                Stroke::new(1.2_f32, ink),
            );
        }
        Icon::Bench => {
            // A toggle panel: two rails with a puck on the lower one.
            painter.rect(
                Rect::from_min_max(at(0.10, 0.18), at(0.90, 0.82)),
                CornerRadius::same(2),
                Color32::TRANSPARENT,
                stroke,
                egui::StrokeKind::Middle,
            );
            painter.hline(
                at(0.22, 0.40).x..=at(0.78, 0.40).x,
                at(0.0, 0.40).y,
                Stroke::new(1.2_f32, ink),
            );
            painter.hline(
                at(0.22, 0.62).x..=at(0.78, 0.62).x,
                at(0.0, 0.62).y,
                Stroke::new(1.2_f32, ink),
            );
            painter.circle_filled(at(0.64, 0.40), (rect.width() * 0.09).max(1.2), ink);
            painter.circle_filled(at(0.36, 0.62), (rect.width() * 0.09).max(1.2), ink);
        }
        Icon::Watch => {
            painter.circle_stroke(at(0.5, 0.5), (rect.width() * 0.30).max(1.5), stroke);
            let hand = Stroke::new(1.3_f32, ink);
            painter.line_segment([at(0.5, 0.5), at(0.5, 0.28)], hand);
            painter.line_segment([at(0.5, 0.5), at(0.68, 0.58)], hand);
        }
        Icon::Problems => {
            painter.circle_stroke(
                at(0.5, 0.5),
                (rect.width() * 0.38).max(2.0),
                Stroke::new(1.4_f32, ink),
            );
            painter.line_segment([at(0.5, 0.30), at(0.5, 0.56)], Stroke::new(1.6_f32, ink));
            painter.circle_filled(at(0.5, 0.70), (rect.width() * 0.07).max(1.0), ink);
        }
        Icon::Error => {
            painter.circle_filled(at(0.5, 0.5), (rect.width() * 0.44).max(2.0), ink);
        }
        Icon::Warning => {
            painter.add(Shape::convex_polygon(
                vec![at(0.5, 0.06), at(0.98, 0.92), at(0.02, 0.92)],
                ink,
                Stroke::NONE,
            ));
        }
        Icon::Info => {
            painter.circle_filled(at(0.5, 0.5), (rect.width() * 0.44).max(2.0), ink);
        }
        Icon::Pointer => {
            painter.add(Shape::convex_polygon(
                vec![
                    at(0.28, 0.08),
                    at(0.78, 0.52),
                    at(0.54, 0.56),
                    at(0.66, 0.88),
                    at(0.52, 0.94),
                    at(0.40, 0.62),
                    at(0.24, 0.78),
                ],
                ink,
                Stroke::new(1.0_f32, ink),
            ));
        }
        Icon::ChevronRight => {
            painter.line_segment([at(0.38, 0.18), at(0.68, 0.50)], stroke);
            painter.line_segment([at(0.68, 0.50), at(0.38, 0.82)], stroke);
        }
        Icon::ChevronDown => {
            painter.line_segment([at(0.18, 0.38), at(0.50, 0.68)], stroke);
            painter.line_segment([at(0.50, 0.68), at(0.82, 0.38)], stroke);
        }
    }
}

/// Draws a severity dot: the filled circle the problems list and the rung rows
/// use instead of a text marker.
pub fn severity_dot(painter: &Painter, centre: Pos2, colour: Color32, filled: bool) {
    if filled {
        painter.circle_filled(centre, 4.0, colour);
    } else {
        painter.circle_stroke(centre, 3.5, Stroke::new(1.5_f32, colour));
    }
}

/// A one-character severity marker, drawn as text.
///
/// The problems list shows a glyph *and* a word, because a colour alone is not
/// an accessible signal.
pub fn severity_mark(painter: &Painter, rect: Rect, colour: Color32, mark: &str) {
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        mark,
        FontId::new(
            (rect.height() * 0.9).clamp(8.0, 13.0),
            FontFamily::Proportional,
        ),
        colour,
    );
}

/// The colour an icon is drawn in.
fn colour(tokens: &Tokens, icon: Icon) -> Color32 {
    match icon {
        Icon::Error => tokens.error,
        Icon::Warning => tokens.warning,
        Icon::Info => tokens.accent,
        Icon::Problems => tokens.text_dim,
        _ => tokens.text_dim,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Paints `icon` into `rect`, allocating a separate (always positive)
    /// rectangle so a degenerate size reaches the painter and not the layout.
    fn paint(icon: Icon, rect: Rect) {
        let ctx = egui::Context::default();
        let input = || egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(64.0, 64.0))),
            ..egui::RawInput::default()
        };
        let _ = ctx.run(input(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let (slot, _) =
                    ui.allocate_exact_size(egui::vec2(48.0, 48.0), egui::Sense::hover());
                for tokens in [Tokens::light(), Tokens::dark()] {
                    draw(
                        ui.painter(),
                        rect.translate(slot.min.to_vec2()),
                        &tokens,
                        icon,
                    );
                    severity_dot(ui.painter(), slot.center(), tokens.error, true);
                    severity_dot(ui.painter(), slot.center(), tokens.error, false);
                    severity_mark(ui.painter(), slot, tokens.error, "!");
                }
            });
        });
    }

    #[test]
    fn every_icon_draws_at_every_size_without_panicking() {
        for icon in [
            Icon::Plc,
            Icon::Program,
            Icon::Section,
            Icon::Rung,
            Icon::Tags,
            Icon::Bench,
            Icon::Watch,
            Icon::Problems,
            Icon::Error,
            Icon::Warning,
            Icon::Info,
            Icon::Pointer,
            Icon::ChevronRight,
            Icon::ChevronDown,
        ] {
            for side in [1.0_f32, SMALL, 20.0, 32.0] {
                paint(
                    icon,
                    Rect::from_min_size(Pos2::ZERO, egui::vec2(side, side)),
                );
            }
            // A degenerate rectangle is dropped, not drawn.
            paint(icon, Rect::from_min_size(Pos2::ZERO, egui::vec2(0.0, 0.0)));
            paint(icon, Rect::from_min_size(Pos2::ZERO, egui::vec2(-4.0, 7.0)));
        }
    }

    #[test]
    fn severity_icons_use_the_tokens() {
        let tokens = Tokens::light();
        assert_eq!(colour(&tokens, Icon::Error), tokens.error);
        assert_eq!(colour(&tokens, Icon::Warning), tokens.warning);
        assert_eq!(colour(&tokens, Icon::Plc), tokens.text_dim);
    }
}
