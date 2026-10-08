//! The shortcut help window (F1).

use egui::RichText;

/// The shortcuts of `docs/EDITOR.md` §Keyboard, as `(keys, action)` pairs.
pub const SHORTCUTS: [(&str, &str); 20] = [
    ("Ctrl/Cmd + Z", "Undo"),
    ("Ctrl/Cmd + Shift + Z, Ctrl/Cmd + Y", "Redo"),
    ("Ctrl/Cmd + N", "New project"),
    ("Ctrl/Cmd + O", "Open"),
    ("Ctrl/Cmd + S", "Save"),
    ("Ctrl/Cmd + Shift + S", "Save as"),
    ("Ctrl/Cmd + R", "Run / Stop"),
    ("Ctrl/Cmd + T", "Single scan"),
    (
        "Ctrl/Cmd + Shift + A",
        "Auto-fill the bench from the program",
    ),
    ("Delete / Backspace", "Delete the selection"),
    ("Arrows", "Move the selection one cell"),
    ("V", "Toggle the vertical link of the selected cell"),
    ("Ctrl/Cmd + = / Ctrl/Cmd + -", "Zoom in / out"),
    ("Ctrl/Cmd + 0", "Reset the view"),
    ("Wheel", "Zoom around the pointer"),
    ("Middle drag, Space + drag", "Pan the canvas"),
    ("F1", "This list"),
    ("Escape", "Drop the tool and the selection"),
    (
        "Palette letters",
        "Arm a palette entry (shown on each button)",
    ),
    ("Click, drag", "Place, select, move"),
];

/// The `(keys, action)` pairs of one group, in the order the dialog shows them.
#[derive(Debug, Clone, Copy)]
pub struct Group {
    /// The heading of the group.
    pub title: &'static str,
    /// The shortcuts of the group.
    pub items: &'static [(&'static str, &'static str)],
}

/// A one-line explanation of a group, for its heading's tooltip.
fn group_hint(title: &str) -> &'static str {
    match title {
        "File" => "Opening and saving projects, and the history of your edits",
        "Editing" => "Placing, moving and removing ladder elements",
        "View" => "Zooming and panning the schematic",
        _ => "Running the simulated controller",
    }
}

/// The shortcuts, grouped the way the ribbon is.
///
/// The groups are views over [`SHORTCUTS`], so the list stays the single source
/// of truth and a new shortcut is added in exactly one place.
pub fn groups() -> Vec<Group> {
    let bounds = [(0, 6), (6, 9), (9, 14), (14, SHORTCUTS.len())];
    let titles = ["File", "Editing", "View", "Online"];
    bounds
        .iter()
        .zip(titles)
        .map(|((from, to), title)| Group {
            title,
            items: SHORTCUTS.get(*from..*to).unwrap_or(&[]),
        })
        .collect()
}

/// Draws the shortcut list as a dialog: grouped, with key caps.
pub fn show(ui: &mut egui::Ui) {
    // The tokens are the context's, so a dialog opened in the dark theme is
    // still drawn with the dark palette.
    let tokens = tokens_of(ui);
    ui.label(
        RichText::new("Keyboard shortcuts")
            .size(crate::design::TypeScale::TITLE)
            .color(tokens.text),
    );
    ui.label(
        RichText::new("Shortcuts use Cmd on macOS and Ctrl elsewhere.")
            .size(crate::design::TypeScale::CAPTION)
            .color(tokens.text_dim),
    );
    ui.add_space(crate::design::SPACE_2);

    egui::ScrollArea::vertical()
        .max_height(420.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for group in groups() {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(group.title.to_uppercase())
                            .size(crate::design::TypeScale::CAPTION)
                            .color(tokens.text_dim)
                            .strong(),
                    );
                })
                .response
                .on_hover_text(group_hint(group.title));
                ui.add_space(crate::design::SPACE_1);
                egui::Grid::new(group.title)
                    .num_columns(2)
                    .spacing([crate::design::SPACE_3, crate::design::SPACE_1])
                    .min_col_width(200.0)
                    .show(ui, |ui| {
                        for (keys, action) in group.items {
                            key_cap(ui, &tokens, keys);
                            ui.label(
                                RichText::new(*action)
                                    .size(crate::design::TypeScale::BODY)
                                    .color(tokens.text),
                            );
                            ui.end_row();
                        }
                    });
                ui.add_space(crate::design::SPACE_3);
            }
        });
}

/// The tokens the dialog draws with, resolved from the context's visuals.
fn tokens_of(ui: &egui::Ui) -> crate::design::Tokens {
    let visuals = ui.visuals();
    if visuals.panel_fill == crate::design::Tokens::dark().panel {
        crate::design::Tokens::dark()
    } else {
        crate::design::Tokens::light()
    }
}

/// One key cap: the key names in a small bordered chip, as a keyboard draws them.
fn key_cap(ui: &mut egui::Ui, tokens: &crate::design::Tokens, keys: &str) {
    let font = egui::FontId::new(
        crate::design::TypeScale::CAPTION,
        egui::FontFamily::Monospace,
    );
    let galley = ui
        .painter()
        .layout_no_wrap(keys.to_owned(), font, tokens.text);
    let size = galley.size() + egui::vec2(crate::design::SPACE_2, crate::design::SPACE_1);
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    ui.painter().rect(
        rect,
        egui::CornerRadius::same(crate::design::RADIUS_CONTROL),
        tokens.surface,
        tokens.hairline(),
        egui::StrokeKind::Inside,
    );
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, tokens.text);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_help_covers_the_documented_shortcuts() {
        assert_eq!(SHORTCUTS.len(), 20);
        for (keys, action) in SHORTCUTS {
            assert!(!keys.is_empty());
            assert!(!action.is_empty());
        }
        let text: String = SHORTCUTS.map(|(keys, _)| keys).join("|");
        for expected in [
            "Ctrl/Cmd + Z",
            "Ctrl/Cmd + Y",
            "Ctrl/Cmd + Shift + A",
            "V",
            "F1",
        ] {
            assert!(text.contains(expected), "missing {expected}");
        }
    }
}
