//! The instruction palette drawn inside the ribbon.
//!
//! `docs/UX.md` §1: every vendor tool groups its instruction set by family and
//! shows an *icon* with a short label and the shortcut in the tooltip — forty
//! flat text buttons are unusable. This module turns [`crate::palette`]'s entries
//! into exactly that: a pointer tool plus the families
//! [`Family::BitLogic`], [`Family::Coils`], [`Family::Timers`],
//! [`Family::Counters`], [`Family::Data`] and [`Family::Program`], each a
//! labelled group of icon buttons drawn with
//! [`crate::symbols::element_glyph`] so an instruction looks the same here, in
//! the project tree and on the canvas.
//!
//! The grouping is a pure function of an [`ElementKind`] and is unit-tested; the
//! drawing only reads it. Clicking dispatches [`Action::Pick`] through
//! [`EditorApp::handle`], which is what arms the tool and updates the ribbon.

use egui::{Color32, CornerRadius, FontFamily, FontId, Response, RichText, Sense, Stroke, Ui};

use softladder_core::ElementKind;

use crate::app::{EditorApp, Tool};
use crate::design::{Tokens, TypeScale, RADIUS_CONTROL, SPACE_1, SPACE_2};
use crate::palette::Entry;
use crate::panels::icons::{self, Icon};
use crate::shortcuts::Action;
use crate::symbols;

/// A family of instructions, the way the vendors group them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    /// Contacts: normally open, normally closed and the edge contacts.
    BitLogic,
    /// Output, negated, set and reset coils.
    Coils,
    /// On-delay, off-delay and pulse timers.
    Timers,
    /// Count-up, count-down and up/down counters.
    Counters,
    /// Registers and the compare/operate expression blocks.
    Data,
    /// Jump, call and the connection used to draw parallel branches.
    Program,
}

impl Family {
    /// Every family, in the order the palette shows them.
    pub const ALL: [Family; 6] = [
        Family::BitLogic,
        Family::Coils,
        Family::Timers,
        Family::Counters,
        Family::Data,
        Family::Program,
    ];

    /// The label drawn above the group.
    pub fn label(self) -> &'static str {
        match self {
            Family::BitLogic => "Bit logic",
            Family::Coils => "Coils",
            Family::Timers => "Timers",
            Family::Counters => "Counters",
            Family::Data => "Data",
            Family::Program => "Program control",
        }
    }

    /// The entries of this family, in palette order.
    pub fn entries(self) -> Vec<&'static Entry> {
        crate::palette::entries()
            .iter()
            .filter(|entry| family_of(entry.kind) == self)
            .collect()
    }
}

/// The family an element kind belongs to.
///
/// Jumps and calls sit with the connection under *Program control* rather than
/// with the output coils, because that is where the vendors file them.
pub fn family_of(kind: ElementKind) -> Family {
    match kind {
        ElementKind::ContactNo
        | ElementKind::ContactNc
        | ElementKind::ContactRising
        | ElementKind::ContactFalling => Family::BitLogic,
        ElementKind::CoilOut
        | ElementKind::CoilOutNeg
        | ElementKind::CoilSet
        | ElementKind::CoilReset => Family::Coils,
        ElementKind::CoilJump | ElementKind::CoilCall | ElementKind::Connection => Family::Program,
        ElementKind::Timer { .. } => Family::Timers,
        ElementKind::Counter { .. } => Family::Counters,
        ElementKind::Register { .. } | ElementKind::Compare | ElementKind::Operate => Family::Data,
    }
}

/// A short label for an entry, used next to its icon.
///
/// The palette's own `label` is the glyph spelling the ladder uses (`-[ ]-`),
/// which is unreadable as a button caption; this is the word the vendors print.
/// A function block returns `""` and falls back to its mnemonic (`TON`, `CTU`,
/// `FIFO`), which is what a programmer reads anyway.
pub fn short_label(kind: ElementKind) -> &'static str {
    match kind {
        ElementKind::ContactNo => "NO",
        ElementKind::ContactNc => "NC",
        ElementKind::ContactRising => "Rising",
        ElementKind::ContactFalling => "Falling",
        ElementKind::CoilOut => "Coil",
        ElementKind::CoilOutNeg => "Coil /",
        ElementKind::CoilSet => "Set",
        ElementKind::CoilReset => "Reset",
        ElementKind::CoilJump => "Jump",
        ElementKind::CoilCall => "Call",
        ElementKind::Timer { .. } | ElementKind::Counter { .. } | ElementKind::Register { .. } => {
            ""
        }
        ElementKind::Compare => "CMP",
        ElementKind::Operate => "OPE",
        ElementKind::Connection => "Wire",
    }
}

/// The caption of an entry: the mnemonic for a block, the word otherwise.
pub fn caption_of(entry: &Entry) -> &'static str {
    let short = short_label(entry.kind);
    if short.is_empty() {
        entry.label
    } else {
        short
    }
}

/// Draws the palette: the pointer tool and the instruction families.
///
/// The palette lives in the ribbon, so it is kept to a compact band of icon
/// buttons: a glyph, its short label, and a tooltip that carries the full
/// description and the shortcut letter.
pub fn show(app: &mut EditorApp, ui: &mut Ui) {
    let tokens = app.tokens;
    let selected_tool = app.tool;
    let mut pick: Option<ElementKind> = None;
    let mut pointer = false;

    // A horizontally scrollable band, so no family is ever unreachable on a
    // narrow window; the wheel scrolls it, the way the ribbon scrolls in TIA.
    egui::ScrollArea::horizontal()
        .id_salt("palette-band")
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ribbon(ui, &tokens, selected_tool, &mut pick, &mut pointer)
        });

    if pointer {
        app.handle(Action::Cancel);
    } else if let Some(kind) = pick {
        app.handle(Action::Pick(kind));
    }
}

/// Lays out the tool group and every instruction family in one band.
fn ribbon(
    ui: &mut Ui,
    tokens: &Tokens,
    selected_tool: Tool,
    pick: &mut Option<ElementKind>,
    pointer: &mut bool,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(SPACE_2, SPACE_1);
        group(ui, tokens, "Tool", |ui| {
            let response = chip(
                ui,
                tokens,
                &PointerGlyph,
                "Select",
                selected_tool == Tool::Select,
                0.0,
            )
            .on_hover_text("Pointer tool: click to select, drag to move");
            if response.clicked() {
                *pointer = true;
            }
        });
        for family in Family::ALL {
            let entries = family.entries();
            if entries.is_empty() {
                continue;
            }
            // One width per family, so a group reads as a column of equal
            // buttons and no caption is ever clipped.
            let width = entries
                .iter()
                .map(|entry| caption_width(ui, caption_of(entry)))
                .fold(0.0_f32, f32::max)
                .max(CHIP_W);
            group(ui, tokens, family.label(), |ui| {
                for entry in entries {
                    let armed = selected_tool == Tool::Place(entry.kind);
                    let response = chip(ui, tokens, &entry.kind, caption_of(entry), armed, width)
                        .on_hover_text(format!(
                            "{}  ({})\n{}",
                            entry.tooltip, entry.letter, entry.label
                        ));
                    if response.clicked() {
                        *pick = Some(entry.kind);
                    }
                }
            });
        }
    });
}

/// A group box with its caption above it, the way the ribbon label sits above
/// every other command group.
fn group(ui: &mut Ui, tokens: &Tokens, title: &str, contents: impl FnOnce(&mut Ui)) {
    ui.vertical(|ui| {
        ui.add_space(SPACE_1);
        ui.horizontal(|ui| {
            ui.add_space(SPACE_1);
            ui.label(
                RichText::new(title.to_uppercase())
                    .size(TypeScale::CAPTION)
                    .color(tokens.text_dim)
                    .strong(),
            );
        });
        egui::Frame::new()
            .fill(tokens.panel)
            .stroke(Stroke::new(1.0_f32, tokens.border))
            .corner_radius(CornerRadius::same(RADIUS_CONTROL))
            .inner_margin(egui::Margin::same(SPACE_1 as i8))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(SPACE_1, SPACE_1);
                    contents(ui);
                });
            });
    });
}

/// Anything that can be drawn as a palette chip: an element kind, or the pointer.
trait Chip {
    /// Paints the chip's glyph into `rect`.
    fn paint(&self, painter: &egui::Painter, rect: egui::Rect, tokens: &Tokens);
}

impl Chip for ElementKind {
    fn paint(&self, painter: &egui::Painter, rect: egui::Rect, tokens: &Tokens) {
        symbols::element_glyph(painter, rect, tokens, *self);
    }
}

/// The pointer tool's glyph, so the tool group reads like the instruction groups.
struct PointerGlyph;

impl Chip for PointerGlyph {
    fn paint(&self, painter: &egui::Painter, rect: egui::Rect, tokens: &Tokens) {
        icons::draw(painter, rect, tokens, Icon::Pointer);
    }
}

/// Height of one palette chip: a glyph and a caption line under it.
const CHIP_H: f32 = 40.0;
/// Smallest width of a palette chip, so a two-letter caption still has a target.
const CHIP_W: f32 = 34.0;
/// Height reserved for the glyph inside a chip.
const GLYPH_H: f32 = 17.0;

/// The width a caption needs, including the chip's padding.
fn caption_width(ui: &Ui, caption: &str) -> f32 {
    let font = FontId::new(TypeScale::CAPTION, FontFamily::Proportional);
    ui.painter()
        .layout_no_wrap(caption.to_owned(), font, Color32::PLACEHOLDER)
        .size()
        .x
        + SPACE_2
}

/// Draws one chip: a glyph above its caption, with a clear armed state.
///
/// An icon *above* its label is what keeps the whole instruction set inside the
/// ribbon: a chip is only as wide as its caption, so all six families fit in the
/// band. The tooltip carries the full description and the shortcut letter.
fn chip(
    ui: &mut Ui,
    tokens: &Tokens,
    glyph: &dyn Chip,
    caption: &str,
    armed: bool,
    width: f32,
) -> Response {
    let font = FontId::new(TypeScale::CAPTION, FontFamily::Proportional);
    let width = if width > 0.0 { width } else { CHIP_W };
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, CHIP_H), Sense::click());

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
        egui::StrokeKind::Inside,
    );
    // A plate behind the glyph, so the icon reads as a button face rather than
    // as a drawing floating in the panel.
    let plate = egui::Rect::from_center_size(
        egui::pos2(rect.center().x, rect.top() + SPACE_1 + GLYPH_H / 2.0),
        egui::vec2(rect.width() - SPACE_1, GLYPH_H + SPACE_1),
    );
    ui.painter().rect(
        plate,
        CornerRadius::same(2),
        tokens.surface,
        Stroke::NONE,
        egui::StrokeKind::Inside,
    );

    // An armed chip draws its glyph and caption in the accent colour, so which
    // tool is in hand is unmistakable without reading the caption.
    let ink = if armed { tokens.accent } else { tokens.text };
    let glyph_tokens = Tokens {
        wire_idle: ink,
        text: ink,
        ..*tokens
    };
    let glyph_rect = egui::Rect::from_center_size(
        egui::pos2(rect.center().x, rect.top() + SPACE_1 + GLYPH_H / 2.0),
        egui::vec2((rect.width() - SPACE_2).min(30.0), GLYPH_H),
    );
    glyph.paint(ui.painter(), glyph_rect, &glyph_tokens);
    // The caption is laid out to fit the chip, so it is never cut in half.
    let galley = ui
        .painter()
        .layout(caption.to_owned(), font, ink, rect.width() - SPACE_1);
    ui.painter().galley(
        egui::pos2(
            rect.center().x - galley.size().x / 2.0,
            glyph_rect.bottom() + SPACE_1,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The window the headless frames are laid out in.
    const TEST_SIZE: egui::Vec2 = egui::vec2(1440.0, 900.0);

    /// A context for the headless frames, shared by the tests of this module.
    fn ctx() -> egui::Context {
        egui::Context::default()
    }
    use std::collections::HashSet;

    #[test]
    fn every_palette_entry_belongs_to_exactly_one_family() {
        let mut seen: Vec<ElementKind> = Vec::new();
        for family in Family::ALL {
            for entry in family.entries() {
                assert!(
                    !seen.contains(&entry.kind),
                    "{:?} appears in two families",
                    entry.kind
                );
                seen.push(entry.kind);
                assert_eq!(family_of(entry.kind), family);
            }
        }
        assert_eq!(
            seen.len(),
            crate::palette::entries().len(),
            "a palette entry is in no family and would be unreachable"
        );
        assert_eq!(Family::ALL.len(), 6);
    }

    #[test]
    fn the_families_are_labelled_and_non_empty() {
        let labels: HashSet<&str> = Family::ALL.iter().map(|family| family.label()).collect();
        assert_eq!(
            labels.len(),
            Family::ALL.len(),
            "two families share a label"
        );
        for family in Family::ALL {
            assert!(!family.entries().is_empty(), "{family:?} is empty");
            assert!(!family.label().is_empty());
        }
        assert_eq!(Family::BitLogic.entries().len(), 4);
        assert_eq!(Family::Coils.entries().len(), 4);
        assert_eq!(Family::Timers.entries().len(), 3);
        assert_eq!(Family::Counters.entries().len(), 3);
        assert_eq!(Family::Data.entries().len(), 4);
        assert_eq!(Family::Program.entries().len(), 3);
    }

    #[test]
    fn jumps_calls_and_wires_are_program_control_not_coils() {
        assert_eq!(family_of(ElementKind::CoilJump), Family::Program);
        assert_eq!(family_of(ElementKind::CoilCall), Family::Program);
        assert_eq!(family_of(ElementKind::Connection), Family::Program);
        assert_eq!(family_of(ElementKind::CoilSet), Family::Coils);
        assert_eq!(family_of(ElementKind::Compare), Family::Data);
        assert_eq!(family_of(ElementKind::Operate), Family::Data);
    }

    #[test]
    fn every_chip_has_a_readable_caption_and_never_a_glyph_spelling() {
        for entry in crate::palette::entries() {
            let caption = caption_of(entry);
            assert!(!caption.is_empty());
            assert!(
                !caption.contains("-[") && !caption.contains("-(") && !caption.contains("wire"),
                "`{caption}` is the ladder glyph spelling, not a label"
            );
            assert!(
                caption.chars().count() <= 10,
                "`{caption}` will not fit next to an icon"
            );
        }
        let lookup = |kind: ElementKind| {
            crate::palette::entries()
                .iter()
                .find(|entry| entry.kind == kind)
                .map(|entry| caption_of(entry))
                .expect("the kind is in the palette")
        };
        assert_eq!(
            lookup(ElementKind::Timer {
                mode: softladder_core::TimerMode::On
            }),
            "TON",
            "a block keeps its mnemonic"
        );
        assert_eq!(lookup(ElementKind::ContactNo), "NO");
        assert_eq!(lookup(ElementKind::ContactNc), "NC");
        assert_eq!(lookup(ElementKind::Compare), "CMP");
        assert_eq!(lookup(ElementKind::Operate), "OPE");
        // Every caption is short enough for a chip.
        for entry in crate::palette::entries() {
            assert!(caption_of(entry).chars().count() <= 7);
        }
        assert_eq!(lookup(ElementKind::Connection), "Wire");
    }

    #[test]
    fn only_the_mnemonics_fall_back_to_the_palette_label() {
        for entry in crate::palette::entries() {
            let short = short_label(entry.kind);
            let block = matches!(
                entry.kind,
                ElementKind::Timer { .. }
                    | ElementKind::Counter { .. }
                    | ElementKind::Register { .. }
            );
            assert_eq!(
                short.is_empty(),
                block,
                "{:?} has the wrong label",
                entry.kind
            );
        }
    }

    /// The drawing is exercised headlessly, in both themes, over a project whose
    /// rungs are missing: a painter call that panicked would fail here.
    #[test]
    fn drawing_the_palette_is_panic_free() {
        use softladder_core::{Project, Rung, Section};
        let mut project = Project::new("palette");
        let mut section = Section::new(1, "Main");
        section.rungs.push(1);
        project.sections.push(section);
        project.rungs.push(Rung::new(1));
        let mut app = EditorApp::new(project);
        for theme in [crate::design::Theme::Light, crate::design::Theme::Dark] {
            app.theme = theme;
            app.tokens = Tokens::for_theme(theme);
            crate::panels::test_frame(&ctx(), &mut app, TEST_SIZE);
        }
        // And with a tool armed, so the selected state is painted too.
        app.handle(Action::Pick(ElementKind::CoilSet));
        crate::panels::test_frame(&ctx(), &mut app, TEST_SIZE);
    }

    /// An empty project has no rungs; the ribbon must still draw.
    #[test]
    fn the_palette_draws_for_an_empty_project() {
        let mut app = EditorApp::new(softladder_core::Project::new("empty"));
        crate::panels::test_frame(&ctx(), &mut app, TEST_SIZE);
    }
}
