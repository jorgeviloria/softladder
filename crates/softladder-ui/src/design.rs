//! Design tokens: the colours, spacing, type scale and shared chrome that every
//! panel draws with.
//!
//! Light is the default theme, because a ladder diagram is a schematic and every
//! industrial programming tool puts it on paper; dark is a toggle, not a
//! separate design. See `docs/UX.md` §4 for the table these values come from.

use egui::{Color32, CornerRadius, Stroke, Visuals};

/// Which palette is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Theme {
    /// Light surfaces with a white ladder "paper". The default.
    #[default]
    Light,
    /// Dark surfaces, for dim rooms and personal preference.
    Dark,
}

/// Spacing scale. Nothing in the interface uses a value outside it.
pub const SPACE_1: f32 = 4.0;
/// Two steps of the spacing scale.
pub const SPACE_2: f32 = 8.0;
/// Three steps of the spacing scale.
pub const SPACE_3: f32 = 12.0;
/// Four steps of the spacing scale.
pub const SPACE_4: f32 = 16.0;
/// Six steps of the spacing scale.
pub const SPACE_6: f32 = 24.0;

/// Corner radius of an interactive control.
pub const RADIUS_CONTROL: u8 = 4;
/// Corner radius of a card or the ladder paper.
pub const RADIUS_CARD: u8 = 6;
/// Corner radius of a pill or a tag chip.
pub const RADIUS_PILL: u8 = 2;

/// Type scale, in points.
pub struct TypeScale;

impl TypeScale {
    /// Addresses, units, counts.
    pub const CAPTION: f32 = 11.0;
    /// Most of the interface.
    pub const BODY: f32 = 12.0;
    /// Panel headers and tab labels.
    pub const EMPHASIS: f32 = 13.0;
    /// Empty-state headings.
    pub const TITLE: f32 = 20.0;
}

/// The resolved colours of one theme.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tokens {
    /// Which theme this is.
    pub theme: Theme,
    /// Window background and gutters.
    pub surface: Color32,
    /// Panels, tree, inspector.
    pub panel: Color32,
    /// The ladder document.
    pub paper: Color32,
    /// The canvas grid.
    pub paper_grid: Color32,
    /// Separators and outlines.
    pub border: Color32,
    /// Primary text.
    pub text: Color32,
    /// Addresses, units, secondary lines.
    pub text_dim: Color32,
    /// Selection, focus, active tab.
    pub accent: Color32,
    /// Selected row or network background.
    pub accent_soft: Color32,
    /// Live power flow and true contacts.
    pub energised: Color32,
    /// De-energised wires and symbols.
    pub wire_idle: Color32,
    /// Warnings.
    pub warning: Color32,
    /// Errors.
    pub error: Color32,
    /// The RUN pill.
    pub run: Color32,
    /// The STOP pill.
    pub stop: Color32,
}

impl Default for Tokens {
    fn default() -> Self {
        Self::light()
    }
}

impl Tokens {
    /// The light palette.
    pub fn light() -> Self {
        Self {
            theme: Theme::Light,
            surface: Color32::from_rgb(0xF4, 0xF5, 0xF7),
            panel: Color32::from_rgb(0xFF, 0xFF, 0xFF),
            paper: Color32::from_rgb(0xFF, 0xFF, 0xFF),
            paper_grid: Color32::from_rgb(0xE8, 0xEA, 0xED),
            border: Color32::from_rgb(0xD5, 0xD8, 0xDD),
            text: Color32::from_rgb(0x1B, 0x1D, 0x21),
            text_dim: Color32::from_rgb(0x6B, 0x72, 0x80),
            accent: Color32::from_rgb(0x0F, 0x6F, 0xC5),
            accent_soft: Color32::from_rgb(0xE3, 0xF0, 0xFC),
            energised: Color32::from_rgb(0x00, 0xA6, 0x5A),
            wire_idle: Color32::from_rgb(0x5A, 0x64, 0x72),
            warning: Color32::from_rgb(0xB2, 0x6A, 0x00),
            error: Color32::from_rgb(0xC6, 0x28, 0x28),
            run: Color32::from_rgb(0x00, 0xA6, 0x5A),
            stop: Color32::from_rgb(0xC6, 0x28, 0x28),
        }
    }

    /// The dark palette.
    pub fn dark() -> Self {
        Self {
            theme: Theme::Dark,
            surface: Color32::from_rgb(0x1E, 0x1F, 0x22),
            panel: Color32::from_rgb(0x26, 0x28, 0x2C),
            paper: Color32::from_rgb(0x17, 0x18, 0x1A),
            paper_grid: Color32::from_rgb(0x2A, 0x2C, 0x30),
            border: Color32::from_rgb(0x3A, 0x3D, 0x42),
            text: Color32::from_rgb(0xE6, 0xE7, 0xE9),
            text_dim: Color32::from_rgb(0x9A, 0xA0, 0xA6),
            accent: Color32::from_rgb(0x4C, 0x9A, 0xFF),
            accent_soft: Color32::from_rgb(0x1E, 0x3A, 0x5F),
            energised: Color32::from_rgb(0x35, 0xC4, 0x6F),
            wire_idle: Color32::from_rgb(0x98, 0xA2, 0xB3),
            warning: Color32::from_rgb(0xE0, 0xA0, 0x30),
            error: Color32::from_rgb(0xF2, 0x6B, 0x6B),
            run: Color32::from_rgb(0x35, 0xC4, 0x6F),
            stop: Color32::from_rgb(0xF2, 0x6B, 0x6B),
        }
    }

    /// The palette for a theme choice.
    pub fn for_theme(theme: Theme) -> Self {
        match theme {
            Theme::Light => Self::light(),
            Theme::Dark => Self::dark(),
        }
    }

    /// The same palette with the other theme.
    pub fn toggled(self) -> Self {
        match self.theme {
            Theme::Light => Self::dark(),
            Theme::Dark => Self::light(),
        }
    }

    /// The colour of a colour role, for a severity or a state.
    pub fn severity(&self, severity: softladder_core::Severity) -> Color32 {
        match severity {
            softladder_core::Severity::Info => self.text_dim,
            softladder_core::Severity::Warning => self.warning,
            softladder_core::Severity::Error => self.error,
        }
    }

    /// A 1 px border stroke.
    pub fn hairline(&self) -> Stroke {
        Stroke::new(1.0_f32, self.border)
    }

    /// A 1 px accent stroke, for focus and selection.
    pub fn focus(&self) -> Stroke {
        Stroke::new(1.0_f32, self.accent)
    }

    /// Installs this palette and the type scale on a context.
    pub fn apply(&self, ctx: &egui::Context) {
        let mut visuals = match self.theme {
            Theme::Light => Visuals::light(),
            Theme::Dark => Visuals::dark(),
        };
        visuals.panel_fill = self.panel;
        visuals.window_fill = self.panel;
        visuals.extreme_bg_color = self.surface;
        visuals.faint_bg_color = self.surface;
        visuals.override_text_color = Some(self.text);
        visuals.hyperlink_color = self.accent;
        visuals.window_stroke = self.hairline();
        visuals.window_corner_radius = CornerRadius::same(RADIUS_CARD);
        visuals.menu_corner_radius = CornerRadius::same(RADIUS_CONTROL);
        visuals.selection.bg_fill = self.accent_soft;
        visuals.selection.stroke = self.focus();
        visuals.widgets.noninteractive.bg_fill = self.panel;
        visuals.widgets.noninteractive.bg_stroke = self.hairline();
        visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, self.text);
        visuals.widgets.noninteractive.corner_radius = CornerRadius::same(RADIUS_CONTROL);
        visuals.widgets.inactive.bg_fill = self.surface;
        visuals.widgets.inactive.bg_stroke = self.hairline();
        visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, self.text);
        visuals.widgets.inactive.corner_radius = CornerRadius::same(RADIUS_CONTROL);
        visuals.widgets.hovered.bg_fill = self.accent_soft;
        visuals.widgets.hovered.bg_stroke = self.focus();
        visuals.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, self.text);
        visuals.widgets.hovered.corner_radius = CornerRadius::same(RADIUS_CONTROL);
        visuals.widgets.active.bg_fill = self.accent_soft;
        visuals.widgets.active.bg_stroke = self.focus();
        visuals.widgets.active.fg_stroke = Stroke::new(1.0_f32, self.text);
        visuals.widgets.active.corner_radius = CornerRadius::same(RADIUS_CONTROL);
        visuals.widgets.open.bg_fill = self.surface;
        visuals.widgets.open.bg_stroke = self.hairline();
        visuals.widgets.open.fg_stroke = Stroke::new(1.0_f32, self.text);
        visuals.widgets.open.corner_radius = CornerRadius::same(RADIUS_CONTROL);
        ctx.set_visuals(visuals);

        let mut style = (*ctx.style()).clone();
        use egui::{FontFamily, FontId, TextStyle};
        let proportional = FontFamily::Proportional;
        let monospace = FontFamily::Monospace;
        style.text_styles = [
            (
                TextStyle::Small,
                FontId::new(TypeScale::CAPTION, proportional.clone()),
            ),
            (
                TextStyle::Body,
                FontId::new(TypeScale::BODY, proportional.clone()),
            ),
            (
                TextStyle::Button,
                FontId::new(TypeScale::BODY, proportional.clone()),
            ),
            (
                TextStyle::Heading,
                FontId::new(TypeScale::TITLE, proportional),
            ),
            (
                TextStyle::Monospace,
                FontId::new(TypeScale::BODY, monospace),
            ),
        ]
        .into();
        style.spacing.item_spacing = egui::vec2(SPACE_2, SPACE_1 + 2.0);
        style.spacing.button_padding = egui::vec2(SPACE_2, SPACE_1);
        style.spacing.window_margin = egui::Margin::same(SPACE_3 as i8);
        style.spacing.menu_margin = egui::Margin::same(SPACE_1 as i8);
        style.spacing.indent = SPACE_4;
        style.spacing.interact_size.y = 20.0;
        style.spacing.scroll.bar_width = 8.0;
        ctx.set_style(style);
    }
}

/// A panel header: a small caps label with a hairline underneath.
pub fn section_header(ui: &mut egui::Ui, tokens: &Tokens, text: &str) {
    ui.add_space(SPACE_1);
    ui.horizontal(|ui| {
        ui.add_space(SPACE_1);
        ui.label(
            egui::RichText::new(text.to_uppercase())
                .size(TypeScale::CAPTION)
                .color(tokens.text_dim)
                .strong(),
        );
    });
    ui.add_space(SPACE_1);
    ui.separator();
}

/// A hairline rule across the available width.
pub fn rule(ui: &mut egui::Ui, tokens: &Tokens) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 1.0), egui::Sense::hover());
    ui.painter()
        .hline(rect.x_range(), rect.center().y, tokens.hairline());
}

/// A status pill: a rounded, tinted chip used for RUN/STOP, counts and badges.
pub fn pill(ui: &mut egui::Ui, colour: Color32, text: &str) {
    let font = egui::FontId::new(TypeScale::CAPTION, egui::FontFamily::Proportional);
    let galley = ui.painter().layout_no_wrap(
        text.to_owned(),
        font.clone(),
        Color32::WHITE.gamma_multiply(1.0),
    );
    let size = egui::vec2(galley.size().x + SPACE_3, galley.size().y + SPACE_1);
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::same(RADIUS_PILL), colour);
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, Color32::WHITE);
}

/// A muted, tinted "quiet" pill, used for counts and badges.
pub fn quiet_pill(ui: &mut egui::Ui, colour: Color32, text: &str) {
    let font = egui::FontId::new(TypeScale::CAPTION, egui::FontFamily::Proportional);
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, colour);
    let size = egui::vec2(
        galley.size().x + SPACE_2 + SPACE_1,
        galley.size().y + SPACE_1,
    );
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    ui.painter().rect_filled(
        rect,
        CornerRadius::same(RADIUS_PILL),
        colour.gamma_multiply(0.12),
    );
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, colour);
}

/// Text in the caption style, in the dim colour.
pub fn caption(text: impl Into<String>) -> egui::RichText {
    egui::RichText::new(text.into())
        .size(TypeScale::CAPTION)
        .color(Color32::from_rgb(0x6B, 0x72, 0x80))
}

/// Text in the caption style with an explicit colour.
pub fn caption_coloured(text: impl Into<String>, colour: Color32) -> egui::RichText {
    egui::RichText::new(text.into())
        .size(TypeScale::CAPTION)
        .color(colour)
}

/// Monospaced text, for addresses and expressions.
pub fn mono(text: impl Into<String>) -> egui::RichText {
    egui::RichText::new(text.into()).monospace()
}

/// A card: the framed surface a document or a form sits on.
pub fn card(tokens: &Tokens) -> egui::Frame {
    egui::Frame::new()
        .fill(tokens.panel)
        .stroke(tokens.hairline())
        .corner_radius(CornerRadius::same(RADIUS_CARD))
        .inner_margin(egui::Margin::same(SPACE_3 as i8))
}

/// Draws a centred empty state: a title, an explanation and an optional hint.
pub fn empty_state(ui: &mut egui::Ui, tokens: &Tokens, title: &str, body: &str, hint: &str) {
    ui.vertical_centered(|ui| {
        ui.add_space(SPACE_6);
        ui.label(
            egui::RichText::new(title)
                .size(TypeScale::TITLE)
                .color(tokens.text),
        );
        ui.add_space(SPACE_2);
        ui.label(egui::RichText::new(body).color(tokens.text_dim));
        if !hint.is_empty() {
            ui.add_space(SPACE_2);
            ui.label(
                egui::RichText::new(hint)
                    .size(TypeScale::CAPTION)
                    .color(tokens.text_dim),
            );
        }
        ui.add_space(SPACE_6);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_themes_define_every_colour() {
        for tokens in [Tokens::light(), Tokens::dark()] {
            // A theme that leaves something transparent would show through.
            for colour in [
                tokens.surface,
                tokens.panel,
                tokens.paper,
                tokens.paper_grid,
                tokens.border,
                tokens.text,
                tokens.text_dim,
                tokens.accent,
                tokens.accent_soft,
                tokens.energised,
                tokens.wire_idle,
                tokens.warning,
                tokens.error,
                tokens.run,
                tokens.stop,
            ] {
                assert_eq!(colour.a(), 255, "a theme colour is translucent");
            }
            assert_eq!(tokens.focus().width, 1.0_f32);
            assert_eq!(tokens.hairline().width, 1.0_f32);
        }
    }

    #[test]
    fn the_two_themes_are_actually_different() {
        let light = Tokens::light();
        let dark = Tokens::dark();
        assert_ne!(light.panel, dark.panel);
        assert_ne!(light.text, dark.text);
        assert_eq!(light.toggled().theme, Theme::Dark);
        assert_eq!(dark.toggled().theme, Theme::Light);
        assert_eq!(Tokens::for_theme(Theme::Light), light);
        assert_eq!(Tokens::for_theme(Theme::Dark), dark);
        assert_eq!(Theme::default(), Theme::Light);
    }

    #[test]
    fn text_on_the_light_panel_has_enough_contrast() {
        let light = Tokens::light();
        let luminance = |colour: Color32| {
            let channel = |value: u8| {
                let value = f32::from(value) / 255.0;
                if value <= 0.03928 {
                    value / 12.92
                } else {
                    ((value + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * channel(colour.r())
                + 0.7152 * channel(colour.g())
                + 0.0722 * channel(colour.b())
        };
        let ratio = |a: Color32, b: Color32| {
            let (light, dark) = {
                let (a, b) = (luminance(a), luminance(b));
                if a > b {
                    (a, b)
                } else {
                    (b, a)
                }
            };
            (light + 0.05) / (dark + 0.05)
        };
        assert!(
            ratio(light.text, light.panel) >= 7.0,
            "primary text is not AAA"
        );
        assert!(
            ratio(light.text_dim, light.panel) >= 4.0,
            "secondary text is too faint"
        );
        let contrast = ratio(light.energised, light.paper);
        assert!(contrast >= 3.0, "the live colour is too faint: {contrast}");
    }

    /// The scale is a set of constants, so this assertion is constant-folded;
    /// it is here to document and protect the invariant, not to test the compiler.
    #[allow(clippy::assertions_on_constants)]
    #[test]
    fn the_spacing_scale_is_ordered() {
        assert!(SPACE_1 < SPACE_2 && SPACE_2 < SPACE_3 && SPACE_3 < SPACE_4 && SPACE_4 < SPACE_6);
        assert!(RADIUS_PILL < RADIUS_CONTROL && RADIUS_CONTROL < RADIUS_CARD);
        assert!(TypeScale::CAPTION < TypeScale::BODY);
        assert!(TypeScale::BODY < TypeScale::EMPHASIS);
        assert!(TypeScale::EMPHASIS < TypeScale::TITLE);
    }
}
