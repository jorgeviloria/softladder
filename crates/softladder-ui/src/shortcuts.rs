//! Keyboard → [`Action`] mapping.
//!
//! The mapping is a pair of pure functions over `egui`'s logical key and
//! modifier state, so every shortcut in `docs/EDITOR.md` is unit-tested rather
//! than exercised by hand. The application decides *when* to ask:
//!
//! * [`global_action`] covers the file, history, run and view shortcuts and is
//!   consulted even while a text field has focus (`Ctrl+S` must save).
//! * [`canvas_action`] covers the selection, movement and palette shortcuts and
//!   is only consulted when no text field wants the keyboard, so typing a
//!   variable name can never delete an element.

use egui::{Key, Modifiers};

use softladder_core::ElementKind;

use crate::palette;

/// Everything the editor's keyboard can ask for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    /// Reverse the most recent command.
    Undo,
    /// Re-apply the most recently undone command.
    Redo,
    /// Start an empty project.
    New,
    /// Open a project file.
    Open,
    /// Save to the current path.
    Save,
    /// Save to a path chosen by the user.
    SaveAs,
    /// Start the bench when stopped, stop it when running.
    RunStop,
    /// Advance the bench by exactly one scan.
    SingleScan,
    /// Replace the bench panel with the program's physical variables.
    AutoFillBench,
    /// Delete the selected element.
    Delete,
    /// Move the selection one cell left.
    MoveLeft,
    /// Move the selection one cell right.
    MoveRight,
    /// Move the selection one cell up.
    MoveUp,
    /// Move the selection one cell down.
    MoveDown,
    /// Toggle the selected cell's vertical link.
    ToggleVerticalLink,
    /// Zoom the canvas in.
    ZoomIn,
    /// Zoom the canvas out.
    ZoomOut,
    /// Restore zoom `1.0` and no pan.
    ZoomReset,
    /// Show the shortcut list window.
    ShortcutHelp,
    /// Drop the current selection or tool.
    Cancel,
    /// Arm the palette with an element kind.
    Pick(ElementKind),
}

/// The action a *global* shortcut key asks for, if any.
///
/// Shortcuts use `Modifiers::command`, which is `Ctrl` on Windows and Linux and
/// `Cmd` on macOS, so the same table works on every platform.
pub fn global_action(key: Key, modifiers: Modifiers) -> Option<Action> {
    if key == Key::F1 {
        return Some(Action::ShortcutHelp);
    }
    if !modifiers.command {
        return None;
    }
    match (key, modifiers.shift) {
        (Key::Z, false) => Some(Action::Undo),
        (Key::Z, true) => Some(Action::Redo),
        (Key::Y, _) => Some(Action::Redo),
        (Key::N, _) => Some(Action::New),
        (Key::O, _) => Some(Action::Open),
        (Key::S, false) => Some(Action::Save),
        (Key::S, true) => Some(Action::SaveAs),
        (Key::R, _) => Some(Action::RunStop),
        (Key::T, _) => Some(Action::SingleScan),
        (Key::A, true) => Some(Action::AutoFillBench),
        (Key::Equals | Key::Plus, _) => Some(Action::ZoomIn),
        (Key::Minus, _) => Some(Action::ZoomOut),
        (Key::Num0, _) => Some(Action::ZoomReset),
        _ => None,
    }
}

/// The action a *canvas* shortcut key asks for, if any.
///
/// The caller must not consult this while a text field has focus. Any modifier
/// other than `Shift` is ignored, so `Ctrl+Z` is never also a canvas action.
pub fn canvas_action(key: Key, modifiers: Modifiers) -> Option<Action> {
    if modifiers.command || modifiers.alt {
        return None;
    }
    match key {
        Key::Delete | Key::Backspace => Some(Action::Delete),
        Key::ArrowLeft => Some(Action::MoveLeft),
        Key::ArrowRight => Some(Action::MoveRight),
        Key::ArrowUp => Some(Action::MoveUp),
        Key::ArrowDown => Some(Action::MoveDown),
        Key::V if !modifiers.shift => Some(Action::ToggleVerticalLink),
        Key::Escape => Some(Action::Cancel),
        other => palette::kind_for_key(other).map(Action::Pick),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(shift: bool) -> Modifiers {
        Modifiers {
            command: true,
            shift,
            ..Modifiers::default()
        }
    }

    fn plain() -> Modifiers {
        Modifiers::default()
    }

    #[test]
    fn every_documented_global_shortcut_maps_to_its_action() {
        assert_eq!(global_action(Key::Z, command(false)), Some(Action::Undo));
        assert_eq!(global_action(Key::Z, command(true)), Some(Action::Redo));
        assert_eq!(global_action(Key::Y, command(false)), Some(Action::Redo));
        assert_eq!(global_action(Key::Y, command(true)), Some(Action::Redo));
        assert_eq!(global_action(Key::N, command(false)), Some(Action::New));
        assert_eq!(global_action(Key::O, command(false)), Some(Action::Open));
        assert_eq!(global_action(Key::S, command(false)), Some(Action::Save));
        assert_eq!(global_action(Key::S, command(true)), Some(Action::SaveAs));
        assert_eq!(global_action(Key::R, command(false)), Some(Action::RunStop));
        assert_eq!(
            global_action(Key::T, command(false)),
            Some(Action::SingleScan)
        );
        assert_eq!(
            global_action(Key::A, command(true)),
            Some(Action::AutoFillBench)
        );
        assert_eq!(
            global_action(Key::Equals, command(false)),
            Some(Action::ZoomIn)
        );
        assert_eq!(
            global_action(Key::Plus, command(true)),
            Some(Action::ZoomIn)
        );
        assert_eq!(
            global_action(Key::Minus, command(false)),
            Some(Action::ZoomOut)
        );
        assert_eq!(
            global_action(Key::Num0, command(false)),
            Some(Action::ZoomReset)
        );
        assert_eq!(global_action(Key::F1, plain()), Some(Action::ShortcutHelp));
        assert_eq!(
            global_action(Key::F1, command(true)),
            Some(Action::ShortcutHelp)
        );
    }

    #[test]
    fn global_shortcuts_need_the_command_modifier() {
        for key in [Key::Z, Key::Y, Key::N, Key::O, Key::S, Key::R, Key::T] {
            assert_eq!(global_action(key, plain()), None, "{key:?} without Ctrl");
        }
        assert_eq!(
            global_action(Key::A, command(false)),
            None,
            "Ctrl+A is select all"
        );
        assert_eq!(
            global_action(Key::V, command(false)),
            None,
            "Ctrl+V is paste"
        );
        assert_eq!(global_action(Key::Num1, command(false)), None);
    }

    #[test]
    fn canvas_shortcuts_cover_selection_movement_and_delete() {
        assert_eq!(canvas_action(Key::Delete, plain()), Some(Action::Delete));
        assert_eq!(canvas_action(Key::Backspace, plain()), Some(Action::Delete));
        assert_eq!(
            canvas_action(Key::ArrowLeft, plain()),
            Some(Action::MoveLeft)
        );
        assert_eq!(
            canvas_action(Key::ArrowRight, plain()),
            Some(Action::MoveRight)
        );
        assert_eq!(canvas_action(Key::ArrowUp, plain()), Some(Action::MoveUp));
        assert_eq!(
            canvas_action(Key::ArrowDown, plain()),
            Some(Action::MoveDown)
        );
        assert_eq!(
            canvas_action(Key::V, plain()),
            Some(Action::ToggleVerticalLink)
        );
        assert_eq!(canvas_action(Key::Escape, plain()), Some(Action::Cancel));
    }

    #[test]
    fn canvas_shortcuts_yield_to_the_command_modifier() {
        assert_eq!(canvas_action(Key::Delete, command(false)), None);
        assert_eq!(canvas_action(Key::V, command(false)), None);
        assert_eq!(
            canvas_action(Key::ArrowLeft, command(false)),
            None,
            "Ctrl+Arrow belongs to the text field, not the canvas"
        );
        let alt = Modifiers {
            alt: true,
            ..Modifiers::default()
        };
        assert_eq!(canvas_action(Key::V, alt), None);
    }

    #[test]
    fn every_palette_element_has_a_letter_shortcut_reachable_from_the_canvas() {
        let mut found = Vec::new();
        for key in all_letters() {
            if let Some(Action::Pick(kind)) = canvas_action(key, plain()) {
                found.push(kind);
            }
        }
        for kind in palette::all_kinds() {
            assert!(
                found.contains(&kind),
                "{kind:?} has no canvas letter shortcut"
            );
        }
        assert_eq!(found.len(), palette::all_kinds().len());
    }

    #[test]
    fn unknown_keys_do_nothing() {
        assert_eq!(canvas_action(Key::F5, plain()), None);
        assert_eq!(canvas_action(Key::Space, plain()), None);
        assert_eq!(global_action(Key::Escape, plain()), None);
    }

    /// Every letter key `egui` exposes, for the palette-shortcut sweep.
    fn all_letters() -> [Key; 26] {
        [
            Key::A,
            Key::B,
            Key::C,
            Key::D,
            Key::E,
            Key::F,
            Key::G,
            Key::H,
            Key::I,
            Key::J,
            Key::K,
            Key::L,
            Key::M,
            Key::N,
            Key::O,
            Key::P,
            Key::Q,
            Key::R,
            Key::S,
            Key::T,
            Key::U,
            Key::V,
            Key::W,
            Key::X,
            Key::Y,
            Key::Z,
        ]
    }
}
