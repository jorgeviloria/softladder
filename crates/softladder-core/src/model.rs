//! The ladder-logic project model.
//!
//! A [`Project`] owns a flat list of [`Rung`]s and a list of [`Section`]s that
//! reference those rungs by id. Keeping the rungs flat means a section can be
//! re-ordered or shared without duplicating element data, and it mirrors the
//! way the ClassicLadder container stores one file per rung.
//!
//! All types in this module are plain data: they carry no behaviour, perform
//! no I/O and always round-trip through `serde`.

use serde::{Deserialize, Serialize};

use crate::sfc::SequentialPage;
use crate::sim::SimulationPanel;
use crate::vars::VarRef;

/// Flavour of an IEC 61131-3 timer block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum TimerMode {
    /// On-delay timer (TON).
    #[default]
    On,
    /// Off-delay timer (TOF).
    Off,
    /// Pulse timer (TP), a one-shot of the preset length.
    Pulse,
}

/// Flavour of a counter block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum CounterKind {
    /// Count up (CTU).
    #[default]
    Up,
    /// Count down (CTD).
    Down,
    /// Count up and down (CTUD).
    UpDown,
}

/// Flavour of a register block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum RegisterMode {
    /// First in, first out.
    #[default]
    Fifo,
    /// Last in, first out.
    Lifo,
}

/// Kind of ladder element placed on a rung.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum ElementKind {
    /// Normally-open contact `-[ ]-`.
    #[default]
    ContactNo,
    /// Normally-closed contact `-[/]-`.
    ContactNc,
    /// Rising-edge contact `-[P]-`.
    ContactRising,
    /// Falling-edge contact `-[N]-`.
    ContactFalling,
    /// Normal output coil `-( )-`.
    CoilOut,
    /// Negated output coil `-(/)-`.
    CoilOutNeg,
    /// Set (latch) coil `-(S)-`.
    CoilSet,
    /// Reset (unlatch) coil `-(R)-`.
    CoilReset,
    /// Jump coil `-(J)-`.
    CoilJump,
    /// Subroutine call coil `-(C)-`.
    CoilCall,
    /// IEC timer block.
    Timer {
        /// Timer flavour.
        mode: TimerMode,
    },
    /// Counter block.
    Counter {
        /// Counter flavour.
        kind: CounterKind,
    },
    /// Comparison block.
    Compare,
    /// Arithmetic / assignment block.
    Operate,
    /// Register (FIFO/LIFO) block.
    Register {
        /// Register flavour.
        mode: RegisterMode,
    },
    /// Connection used to draw parallel branches between elements.
    Connection,
}

impl ElementKind {
    /// `true` for every output coil variant.
    pub fn is_coil(self) -> bool {
        matches!(
            self,
            ElementKind::CoilOut
                | ElementKind::CoilOutNeg
                | ElementKind::CoilSet
                | ElementKind::CoilReset
                | ElementKind::CoilJump
                | ElementKind::CoilCall
        )
    }

    /// `true` for every contact variant.
    pub fn is_contact(self) -> bool {
        matches!(
            self,
            ElementKind::ContactNo
                | ElementKind::ContactNc
                | ElementKind::ContactRising
                | ElementKind::ContactFalling
        )
    }

    /// `true` for the function blocks that carry a preset in `params`.
    pub fn is_block(self) -> bool {
        matches!(
            self,
            ElementKind::Timer { .. } | ElementKind::Counter { .. } | ElementKind::Register { .. }
        )
    }

    /// The labels of the input pins the engine reads, in row order.
    ///
    /// This is the **single source of truth** for how many rows a block reads and
    /// what each of them means: the engine's block span comes from
    /// [`ElementKind::input_rows`], and the editor draws exactly these pins, so
    /// the drawing cannot advertise a wire the engine never reads. A timer has
    /// one input (its enable — the preset is a *parameter*, `%TM0.P`, not a
    /// wire), a counter four (`R`, `LD`, `CU`, `CD`) and a register three
    /// (`R`, `IN`, `OUT`).
    ///
    /// The order is the order the engine destructures its inputs in
    /// (`[reset, load, up, down]`), so a label must not be moved without moving
    /// the engine with it; `softladder-core`'s tests pin both.
    pub fn input_pins(self) -> &'static [&'static str] {
        match self {
            ElementKind::Timer { .. } => &["IN"],
            ElementKind::Counter { .. } => &["R", "LD", "CU", "CD"],
            ElementKind::Register { .. } => &["R", "IN", "OUT"],
            _ => &[],
        }
    }

    /// The rows the element reads as a block, at least one.
    ///
    /// Every element occupies at least its own row; a block occupies one row per
    /// input pin.
    pub fn input_rows(self) -> usize {
        self.input_pins().len().max(1)
    }
}

/// An element placed at a `(column, row)` position on a rung.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct PlacedElement {
    /// What the element is.
    pub kind: ElementKind,
    /// Variable the element reads or drives; `Connection` elements have none.
    pub var: Option<VarRef>,
    /// Zero-based column, increasing from the left power rail.
    pub col: u8,
    /// Zero-based row, increasing downwards. Rows are parallel branches.
    pub row: u8,
    /// `true` when this cell is wired to the cell directly above it in the same
    /// column. Vertical links build parallel branches and their merge points;
    /// see `docs/SEMANTICS.md` §2.
    #[serde(default)]
    pub connected_with_top: bool,
    /// Free-form parameters: timer/counter preset, operate target and
    /// expression, comparison operands and operator, and so on.
    pub params: Vec<String>,
}

impl PlacedElement {
    /// Creates an element with no variable and no parameters.
    pub fn new(kind: ElementKind, col: u8, row: u8) -> Self {
        Self {
            kind,
            var: None,
            col,
            row,
            connected_with_top: false,
            params: Vec::new(),
        }
    }

    /// Creates an element bound to `var`.
    pub fn with_var(kind: ElementKind, var: VarRef, col: u8, row: u8) -> Self {
        Self {
            var: Some(var),
            ..Self::new(kind, col, row)
        }
    }

    /// Creates an element carrying `params`.
    pub fn with_params(kind: ElementKind, col: u8, row: u8, params: &[&str]) -> Self {
        Self {
            params: params.iter().map(|p| (*p).to_owned()).collect(),
            ..Self::new(kind, col, row)
        }
    }

    /// Returns a copy of this element wired to the cell above it.
    pub fn linked_up(mut self) -> Self {
        self.connected_with_top = true;
        self
    }
}

/// A single ladder rung: a horizontal power rail with elements on it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Rung {
    /// Stable identifier, unique inside the project.
    pub id: u32,
    /// Optional short label shown in the editor.
    pub label: String,
    /// Optional free-form comment.
    pub comment: String,
    /// Elements placed on the rung, in no particular order.
    pub elements: Vec<PlacedElement>,
    /// How empty cells behave in this rung; see [`WireMode`].
    #[serde(default)]
    pub wire_mode: WireMode,
}

impl Rung {
    /// Creates an empty rung with the given id.
    pub fn new(id: u32) -> Self {
        Self {
            id,
            ..Self::default()
        }
    }

    /// Returns the elements of `row` sorted by column.
    pub fn row(&self, row: u8) -> Vec<&PlacedElement> {
        let mut elements: Vec<&PlacedElement> =
            self.elements.iter().filter(|e| e.row == row).collect();
        elements.sort_by_key(|e| e.col);
        elements
    }

    /// The number of distinct rows used by this rung (at least one).
    pub fn row_count(&self) -> u8 {
        self.elements
            .iter()
            .map(|e| e.row)
            .max()
            .map_or(1, |max| max.saturating_add(1))
    }
}

/// How an empty cell behaves inside a *live* row (a row that holds at least one
/// element).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum WireMode {
    /// Empty cells conduct: the row is wired implicitly, so gaps do not break the
    /// circuit. This is what the editor produces when elements are placed apart
    /// from each other, and the default for projects authored in SoftLadder.
    #[default]
    Implicit,
    /// Only explicit [`ElementKind::Connection`] cells conduct and a gap breaks
    /// the circuit. ClassicLadder behaves this way, so imported projects are
    /// switched to this mode to keep their behaviour identical.
    Explicit,
}

/// Language a section is written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum SectionLanguage {
    /// Relay ladder diagram (executed from M0 onwards).
    #[default]
    Ladder,
    /// Sequential function chart (executed from M4 onwards).
    Sfc,
}

/// A named group of rungs, optionally a subroutine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Section {
    /// Stable identifier, unique inside the project.
    pub id: u32,
    /// Display name.
    pub name: String,
    /// Language the section is written in.
    pub language: SectionLanguage,
    /// When set, the section is a subroutine called with this index.
    pub subroutine: Option<u32>,
    /// Ids of the rungs the section executes, in execution order.
    pub rungs: Vec<u32>,
    /// Sequential chart of an `Sfc` section, or [`None`] for a ladder section
    /// and for an SFC section whose page has not been drawn yet.
    ///
    /// The field is additive (see `docs/FORMAT.md`): it defaults to [`None`]
    /// and is omitted from the serialized form when it is `None`, so documents
    /// written before SFC pages existed keep their exact bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sequential_page: Option<SequentialPage>,
}

impl Section {
    /// Creates an empty ladder section.
    pub fn new(id: u32, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            language: SectionLanguage::Ladder,
            subroutine: None,
            rungs: Vec::new(),
            sequential_page: None,
        }
    }

    /// Creates an SFC section that owns `page`.
    pub fn sfc(id: u32, name: impl Into<String>, page: SequentialPage) -> Self {
        Self {
            language: SectionLanguage::Sfc,
            sequential_page: Some(page),
            ..Self::new(id, name)
        }
    }
}

/// A named symbol (mnemonic) attached to a variable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Symbol {
    /// Symbolic name, unique inside the project.
    pub name: String,
    /// Variable the symbol resolves to. Optional, so a symbol may be declared
    /// before its variable is chosen; the editor resolves it for display.
    #[serde(default)]
    pub var: Option<VarRef>,
    /// Human readable description.
    pub comment: String,
    /// Optional engineering unit.
    pub unit: Option<String>,
}

/// Scan timing configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanConfig {
    /// Target period between two scans, in milliseconds.
    pub period_ms: u32,
    /// Target period between two physical input reads, in milliseconds.
    pub input_period_ms: u32,
}

impl Default for ScanConfig {
    fn default() -> Self {
        Self {
            period_ms: 10,
            input_period_ms: 10,
        }
    }
}

/// A complete SoftLadder project.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    /// Format version of the project file; always at least 1.
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    /// Project name.
    pub name: String,
    /// Author of the project.
    pub author: String,
    /// Free-form project comment.
    pub comment: String,
    /// Sections in execution order.
    pub sections: Vec<Section>,
    /// All rungs of the project, referenced by sections.
    pub rungs: Vec<Rung>,
    /// Symbol table.
    pub symbols: Vec<Symbol>,
    /// Simulation bench laid out for this project (widgets without positions).
    #[serde(default)]
    pub simulation: SimulationPanel,
    /// Scan timing configuration.
    pub scan: ScanConfig,
}

/// Current project schema version.
///
/// Version 2 replaced the `bit` field of [`crate::vars::VarRef`] with a general
/// `accessor`, so that ClassicLadder sub-values such as `%TM0.Q` or `%R0.I` can
/// be expressed. See `docs/FORMAT.md`.
pub const SCHEMA_VERSION: u32 = 2;

fn default_schema_version() -> u32 {
    SCHEMA_VERSION
}

impl Default for Project {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            name: String::new(),
            author: String::new(),
            comment: String::new(),
            sections: Vec::new(),
            rungs: Vec::new(),
            symbols: Vec::new(),
            simulation: SimulationPanel::default(),
            scan: ScanConfig::default(),
        }
    }
}

impl Project {
    /// Creates an empty project with the given name.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Self::default()
        }
    }

    /// Looks up a rung by id.
    pub fn rung(&self, id: u32) -> Option<&Rung> {
        self.rungs.iter().find(|rung| rung.id == id)
    }

    /// Looks up a rung by id, mutably.
    pub fn rung_mut(&mut self, id: u32) -> Option<&mut Rung> {
        self.rungs.iter_mut().find(|rung| rung.id == id)
    }

    /// Looks up a section by id.
    pub fn section(&self, id: u32) -> Option<&Section> {
        self.sections.iter().find(|section| section.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The labels are the contract between the engine and the editor, so they are
    /// pinned here: if one changes, the drawing and the engine's destructuring
    /// change with it.
    #[test]
    fn the_input_pins_are_the_engine_contract() {
        assert_eq!(
            ElementKind::Timer {
                mode: TimerMode::On
            }
            .input_pins(),
            ["IN"]
        );
        assert_eq!(
            ElementKind::Counter {
                kind: CounterKind::Up
            }
            .input_pins(),
            ["R", "LD", "CU", "CD"]
        );
        assert_eq!(
            ElementKind::Register {
                mode: RegisterMode::Fifo
            }
            .input_pins(),
            ["R", "IN", "OUT"]
        );
        assert_eq!(ElementKind::ContactNo.input_pins(), [] as [&str; 0]);
    }

    #[test]
    fn every_element_reads_at_least_its_own_row() {
        for kind in [
            ElementKind::ContactNo,
            ElementKind::CoilOut,
            ElementKind::Timer {
                mode: TimerMode::On,
            },
            ElementKind::Counter {
                kind: CounterKind::UpDown,
            },
            ElementKind::Register {
                mode: RegisterMode::Lifo,
            },
        ] {
            assert!(kind.input_rows() >= 1, "{kind:?}");
            assert_eq!(
                kind.input_rows(),
                kind.input_pins().len().max(1),
                "{kind:?} rows and pins disagree"
            );
        }
    }

    #[test]
    fn default_project_carries_the_current_schema_version() {
        assert_eq!(Project::default().schema_version, SCHEMA_VERSION);
        assert_eq!(SCHEMA_VERSION, 2);
        assert_eq!(ScanConfig::default().period_ms, 10);
    }

    #[test]
    fn rung_rows_are_sorted_by_column() {
        let mut rung = Rung::new(7);
        rung.elements
            .push(PlacedElement::new(ElementKind::ContactNo, 4, 0));
        rung.elements
            .push(PlacedElement::new(ElementKind::ContactNo, 1, 0));
        rung.elements
            .push(PlacedElement::new(ElementKind::ContactNo, 2, 1));
        let row0: Vec<u8> = rung.row(0).iter().map(|e| e.col).collect();
        assert_eq!(row0, vec![1, 4]);
        assert_eq!(rung.row_count(), 2);
    }

    #[test]
    fn element_kind_classification() {
        assert!(ElementKind::ContactNc.is_contact());
        assert!(ElementKind::CoilReset.is_coil());
        assert!(ElementKind::Timer {
            mode: TimerMode::On
        }
        .is_block());
        assert!(!ElementKind::Connection.is_coil());
    }
}
