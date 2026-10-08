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

/// Draws the shortcut list.
pub fn show(ui: &mut egui::Ui) {
    egui::Grid::new("shortcuts_grid")
        .num_columns(2)
        .striped(true)
        .show(ui, |ui| {
            for (keys, action) in SHORTCUTS {
                ui.monospace(keys);
                ui.label(RichText::new(action));
                ui.end_row();
            }
        });
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
