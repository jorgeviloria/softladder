//! The element palette and the defaults a freshly placed element gets.
//!
//! Every palette entry is one [`ElementKind`] plus the letter key that arms it
//! and the text its button shows. [`replace_command`] turns a palette entry and
//! a cell into the single [`Command`] the editor applies, so "what the palette
//! does" is decided here and tested without a window.

use egui::Key;
use softladder_core::{
    CounterKind, ElementKind, PlacedElement, RegisterMode, TimerMode, VarKind, VarRef,
};
use softladder_edit::Command;

/// One button of the element palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    /// Element the button places.
    pub kind: ElementKind,
    /// Short text drawn on the button.
    pub label: &'static str,
    /// Letter that arms this entry from the canvas.
    pub letter: char,
    /// Logical key matching [`Entry::letter`].
    pub key: Key,
    /// One-line description shown on hover.
    pub tooltip: &'static str,
}

/// The palette, in the order `docs/EDITOR.md` lays it out: contacts first, then
/// coils, then the function blocks and the plain connection.
pub static ENTRIES: [Entry; 21] = [
    Entry {
        kind: ElementKind::ContactNo,
        label: "-[ ]-",
        letter: 'N',
        key: Key::N,
        tooltip: "Normally-open contact",
    },
    Entry {
        kind: ElementKind::ContactNc,
        label: "-[/]-",
        letter: 'C',
        key: Key::C,
        tooltip: "Normally-closed contact",
    },
    Entry {
        kind: ElementKind::ContactRising,
        label: "-[P]-",
        letter: 'P',
        key: Key::P,
        tooltip: "Rising-edge contact",
    },
    Entry {
        kind: ElementKind::ContactFalling,
        label: "-[N]-",
        letter: 'F',
        key: Key::F,
        tooltip: "Falling-edge contact",
    },
    Entry {
        kind: ElementKind::CoilOut,
        label: "-( )-",
        letter: 'O',
        key: Key::O,
        tooltip: "Output coil",
    },
    Entry {
        kind: ElementKind::CoilOutNeg,
        label: "-(/)-",
        letter: 'X',
        key: Key::X,
        tooltip: "Negated output coil",
    },
    Entry {
        kind: ElementKind::CoilSet,
        label: "-(S)-",
        letter: 'S',
        key: Key::S,
        tooltip: "Set (latch) coil",
    },
    Entry {
        kind: ElementKind::CoilReset,
        label: "-(R)-",
        letter: 'R',
        key: Key::R,
        tooltip: "Reset (unlatch) coil",
    },
    Entry {
        kind: ElementKind::CoilJump,
        label: "-(J)-",
        letter: 'J',
        key: Key::J,
        tooltip: "Jump coil; its parameter is a rung index or label",
    },
    Entry {
        kind: ElementKind::CoilCall,
        label: "-(C)-",
        letter: 'L',
        key: Key::L,
        tooltip: "Subroutine call coil",
    },
    Entry {
        kind: ElementKind::Timer {
            mode: TimerMode::On,
        },
        label: "TON",
        letter: 'T',
        key: Key::T,
        tooltip: "On-delay timer, preset 3000",
    },
    Entry {
        kind: ElementKind::Timer {
            mode: TimerMode::Off,
        },
        label: "TOF",
        letter: 'D',
        key: Key::D,
        tooltip: "Off-delay timer, preset 3000",
    },
    Entry {
        kind: ElementKind::Timer {
            mode: TimerMode::Pulse,
        },
        label: "TP",
        letter: 'K',
        key: Key::K,
        tooltip: "Pulse timer, preset 3000",
    },
    Entry {
        kind: ElementKind::Counter {
            kind: CounterKind::Up,
        },
        label: "CTU",
        letter: 'U',
        key: Key::U,
        tooltip: "Count-up counter, preset 5",
    },
    Entry {
        kind: ElementKind::Counter {
            kind: CounterKind::Down,
        },
        label: "CTD",
        letter: 'E',
        key: Key::E,
        tooltip: "Count-down counter, preset 5",
    },
    Entry {
        kind: ElementKind::Counter {
            kind: CounterKind::UpDown,
        },
        label: "CTUD",
        letter: 'B',
        key: Key::B,
        tooltip: "Up/down counter, preset 5",
    },
    Entry {
        kind: ElementKind::Register {
            mode: RegisterMode::Fifo,
        },
        label: "FIFO",
        letter: 'G',
        key: Key::G,
        tooltip: "First-in first-out register, preset 500",
    },
    Entry {
        kind: ElementKind::Register {
            mode: RegisterMode::Lifo,
        },
        label: "LIFO",
        letter: 'Z',
        key: Key::Z,
        tooltip: "Last-in first-out register, preset 500",
    },
    Entry {
        kind: ElementKind::Compare,
        label: "CMP",
        letter: 'M',
        key: Key::M,
        tooltip: "Comparison block, expression `%MW0 = 0`",
    },
    Entry {
        kind: ElementKind::Operate,
        label: "OPE",
        letter: 'A',
        key: Key::A,
        tooltip: "Arithmetic/assignment block, `%MW0 = 0`",
    },
    Entry {
        kind: ElementKind::Connection,
        label: "wire",
        letter: 'W',
        key: Key::W,
        tooltip: "Connection used to draw parallel branches",
    },
];

/// The whole palette.
pub fn entries() -> &'static [Entry] {
    &ENTRIES
}

/// Every element kind the palette can place, in palette order.
pub fn all_kinds() -> Vec<ElementKind> {
    ENTRIES.iter().map(|entry| entry.kind).collect()
}

/// The entry armed by `key`, if any.
pub fn entry_for_key(key: Key) -> Option<&'static Entry> {
    ENTRIES.iter().find(|entry| entry.key == key)
}

/// The element kind armed by `key`, if any.
pub fn kind_for_key(key: Key) -> Option<ElementKind> {
    entry_for_key(key).map(|entry| entry.kind)
}

/// The entry that places `kind`, comparing timers/counters/registers by flavour.
pub fn entry_for_kind(kind: ElementKind) -> Option<&'static Entry> {
    ENTRIES.iter().find(|entry| entry.kind == kind)
}

/// Short human-readable name of an element kind.
pub fn short_name(kind: ElementKind) -> &'static str {
    match kind {
        ElementKind::ContactNo => "NO contact",
        ElementKind::ContactNc => "NC contact",
        ElementKind::ContactRising => "rising contact",
        ElementKind::ContactFalling => "falling contact",
        ElementKind::CoilOut => "output coil",
        ElementKind::CoilOutNeg => "negated coil",
        ElementKind::CoilSet => "set coil",
        ElementKind::CoilReset => "reset coil",
        ElementKind::CoilJump => "jump coil",
        ElementKind::CoilCall => "call coil",
        ElementKind::Timer {
            mode: TimerMode::On,
        } => "TON timer",
        ElementKind::Timer {
            mode: TimerMode::Off,
        } => "TOF timer",
        ElementKind::Timer {
            mode: TimerMode::Pulse,
        } => "TP timer",
        ElementKind::Counter {
            kind: CounterKind::Up,
        } => "CTU counter",
        ElementKind::Counter {
            kind: CounterKind::Down,
        } => "CTD counter",
        ElementKind::Counter {
            kind: CounterKind::UpDown,
        } => "CTUD counter",
        ElementKind::Register {
            mode: RegisterMode::Fifo,
        } => "FIFO register",
        ElementKind::Register {
            mode: RegisterMode::Lifo,
        } => "LIFO register",
        ElementKind::Compare => "compare block",
        ElementKind::Operate => "operate block",
        ElementKind::Connection => "connection",
    }
}

/// The variable a freshly placed element of `kind` gets in column `col`.
///
/// The index follows the column, so a contact in column 0 is `%I0`, a coil in
/// column 1 is `%Q1` and a timer in column 2 is `%TM2`. That makes a placed
/// element immediately scannable without an extra round of typing, and it is
/// deterministic, which is what the unit tests pin down. Elements that address
/// no variable of their own (`CMP`, `OPE`, `wire`, jump and call) get `None`.
pub fn default_var(kind: ElementKind, col: u8) -> Option<VarRef> {
    let index = u32::from(col);
    let kind = match kind {
        ElementKind::ContactNo
        | ElementKind::ContactNc
        | ElementKind::ContactRising
        | ElementKind::ContactFalling => VarKind::PhysIn,
        ElementKind::CoilOut
        | ElementKind::CoilOutNeg
        | ElementKind::CoilSet
        | ElementKind::CoilReset => VarKind::PhysOut,
        ElementKind::Timer { .. } => VarKind::TimerIec,
        ElementKind::Counter { .. } => VarKind::Counter,
        ElementKind::Register { .. } => VarKind::Register,
        ElementKind::CoilJump
        | ElementKind::CoilCall
        | ElementKind::Compare
        | ElementKind::Operate
        | ElementKind::Connection => return None,
    };
    Some(VarRef::new(kind, index))
}

/// The parameters a freshly placed element of `kind` gets.
///
/// The presets match `docs/EDITOR.md` §"Placing and editing": a timer starts at
/// `3000`, a counter at `5`, a register at `500` and both expression blocks at
/// `%MW0 = 0`. Jump and call coils get target `0` so they carry the parameter
/// the lint requires; everything else gets none.
pub fn default_params(kind: ElementKind) -> Vec<&'static str> {
    match kind {
        ElementKind::Timer { .. } => vec!["3000"],
        ElementKind::Counter { .. } => vec!["5"],
        ElementKind::Register { .. } => vec!["500"],
        ElementKind::Compare | ElementKind::Operate => vec!["%MW0", "=", "0"],
        ElementKind::CoilJump | ElementKind::CoilCall => vec!["0"],
        _ => Vec::new(),
    }
}

/// A fully defaulted element of `kind` at `(col, row)`.
pub fn element(kind: ElementKind, col: u8, row: u8) -> PlacedElement {
    let mut element = PlacedElement::new(kind, col, row);
    element.var = default_var(kind, col);
    element.params = default_params(kind)
        .into_iter()
        .map(str::to_owned)
        .collect();
    element
}

/// The command that puts a defaulted palette element on a cell.
///
/// It is a [`Command::ReplaceElement`], so placing on an occupied cell is a
/// normal edit and undoing it restores the previous element in one step.
pub fn replace_command(rung: u32, kind: ElementKind, col: u8, row: u8) -> Command {
    Command::ReplaceElement {
        rung,
        element: element(kind, col, row),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn var_text(kind: ElementKind, col: u8) -> Option<String> {
        default_var(kind, col).map(|var| var.to_string())
    }

    #[test]
    fn the_palette_covers_every_element_kind_once() {
        let kinds = all_kinds();
        let unique: HashSet<ElementKind> = kinds.iter().copied().collect();
        assert_eq!(
            unique.len(),
            kinds.len(),
            "no duplicate kind in the palette"
        );
        for kind in [
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
                mode: TimerMode::On,
            },
            ElementKind::Timer {
                mode: TimerMode::Off,
            },
            ElementKind::Timer {
                mode: TimerMode::Pulse,
            },
            ElementKind::Counter {
                kind: CounterKind::Up,
            },
            ElementKind::Counter {
                kind: CounterKind::Down,
            },
            ElementKind::Counter {
                kind: CounterKind::UpDown,
            },
            ElementKind::Register {
                mode: RegisterMode::Fifo,
            },
            ElementKind::Register {
                mode: RegisterMode::Lifo,
            },
            ElementKind::Compare,
            ElementKind::Operate,
            ElementKind::Connection,
        ] {
            assert!(
                unique.contains(&kind),
                "{kind:?} is missing from the palette"
            );
        }
        assert_eq!(entries().len(), 21);
    }

    #[test]
    fn palette_letters_and_keys_are_unique() {
        let mut letters = HashSet::new();
        let mut keys = HashSet::new();
        for entry in entries() {
            assert!(
                letters.insert(entry.letter),
                "letter {} is used twice",
                entry.letter
            );
            assert!(keys.insert(entry.key), "key {:?} is used twice", entry.key);
            assert!(!entry.label.is_empty());
            assert!(!entry.tooltip.is_empty());
            assert_ne!(entry.letter, 'V', "V toggles the vertical link");
        }
        for entry in entries() {
            assert_eq!(entry_for_key(entry.key), Some(entry));
            assert_eq!(kind_for_key(entry.key), Some(entry.kind));
            assert_eq!(entry_for_kind(entry.kind), Some(entry));
        }
        assert_eq!(entry_for_key(Key::Q), None);
        assert_eq!(kind_for_key(Key::V), None);
        assert_eq!(
            entry_for_kind(ElementKind::Compare).map(|e| e.letter),
            Some('M')
        );
    }

    #[test]
    fn contacts_default_to_inputs_and_coils_to_outputs() {
        for kind in [
            ElementKind::ContactNo,
            ElementKind::ContactNc,
            ElementKind::ContactRising,
            ElementKind::ContactFalling,
        ] {
            assert_eq!(var_text(kind, 0).as_deref(), Some("%I0"));
            assert_eq!(var_text(kind, 3).as_deref(), Some("%I3"));
            assert!(default_params(kind).is_empty());
        }
        for kind in [
            ElementKind::CoilOut,
            ElementKind::CoilOutNeg,
            ElementKind::CoilSet,
            ElementKind::CoilReset,
        ] {
            assert_eq!(var_text(kind, 1).as_deref(), Some("%Q1"));
            assert_eq!(var_text(kind, 2).as_deref(), Some("%Q2"));
            assert!(default_params(kind).is_empty());
        }
    }

    #[test]
    fn blocks_default_to_their_own_variable_family_and_preset() {
        let ton = ElementKind::Timer {
            mode: TimerMode::On,
        };
        assert_eq!(var_text(ton, 0).as_deref(), Some("%TM0.Q"));
        assert_eq!(default_params(ton), vec!["3000"]);

        let ctu = ElementKind::Counter {
            kind: CounterKind::Up,
        };
        assert_eq!(var_text(ctu, 4).as_deref(), Some("%C4.D"));
        assert_eq!(default_params(ctu), vec!["5"]);

        let fifo = ElementKind::Register {
            mode: RegisterMode::Fifo,
        };
        assert_eq!(var_text(fifo, 0).as_deref(), Some("%R0"));
        assert_eq!(default_params(fifo), vec!["500"]);

        assert_eq!(var_text(ElementKind::Compare, 0), None);
        assert_eq!(default_params(ElementKind::Compare), vec!["%MW0", "=", "0"]);
        assert_eq!(default_params(ElementKind::Operate), vec!["%MW0", "=", "0"]);
        assert_eq!(default_params(ElementKind::CoilJump), vec!["0"]);
        assert_eq!(default_params(ElementKind::CoilCall), vec!["0"]);
        assert_eq!(var_text(ElementKind::Connection, 0), None);
        assert!(default_params(ElementKind::Connection).is_empty());
    }

    #[test]
    fn element_defaults_are_immediately_scannable() {
        let contact = element(ElementKind::ContactNo, 0, 0);
        assert_eq!(contact.kind, ElementKind::ContactNo);
        assert_eq!(contact.col, 0);
        assert_eq!(contact.row, 0);
        assert_eq!(contact.var.map(|v| v.to_string()).as_deref(), Some("%I0"));
        assert!(contact.params.is_empty());
        assert!(!contact.connected_with_top, "placement never links upwards");

        let timer = element(
            ElementKind::Timer {
                mode: TimerMode::On,
            },
            2,
            1,
        );
        assert_eq!(timer.params, vec!["3000".to_owned()]);
        assert_eq!(timer.var.map(|v| v.to_string()).as_deref(), Some("%TM2.Q"));
        assert_eq!(timer.row, 1);
    }

    #[test]
    fn the_placement_command_replaces_and_carries_the_defaults() {
        let command = replace_command(7, ElementKind::CoilOut, 2, 1);
        assert_eq!(command.label(), "replace element");
        match &command {
            Command::ReplaceElement { rung, element } => {
                assert_eq!(*rung, 7);
                assert_eq!(element.kind, ElementKind::CoilOut);
                assert_eq!(element.col, 2);
                assert_eq!(element.row, 1);
                assert_eq!(
                    element.var.as_ref().map(ToString::to_string).as_deref(),
                    Some("%Q2")
                );
            }
            other => panic!("expected ReplaceElement, got {other:?}"),
        }
        assert_eq!(
            replace_command(1, ElementKind::Operate, 0, 0).label(),
            "replace element"
        );
    }

    #[test]
    fn short_names_describe_every_palette_kind() {
        for entry in entries() {
            let name = short_name(entry.kind);
            assert!(!name.is_empty());
            assert_ne!(name, entry.label, "{} should read as words", entry.label);
        }
        assert_eq!(
            short_name(ElementKind::Timer {
                mode: TimerMode::Pulse
            }),
            "TP timer"
        );
        assert_eq!(
            short_name(ElementKind::Register {
                mode: RegisterMode::Lifo
            }),
            "LIFO register"
        );
        assert_eq!(short_name(ElementKind::Connection), "connection");
    }
}
