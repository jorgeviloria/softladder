//! Pure-domain core of SoftLadder.
//!
//! This crate is the heart of the project and has no dependencies on the other
//! SoftLadder crates. It performs no I/O, draws no UI and never reads the
//! clock: time always enters the scan engine as a parameter. That keeps every
//! scan deterministic, reproducible and trivially unit-testable.
//!
//! * [`vars`] — `%`-notation variable references and ClassicLadder aliases.
//! * [`model`] — the ladder-logic project model.
//! * [`expr`] — expression tokenizer, Pratt parser and evaluator.
//! * [`scan`] — variable store, function blocks and the deterministic engine.
//! * [`sim`] — simulation panel (bench layout) and its runtime positions.
//! * [`sfc`] — Sequential Function Chart model (engine lands in M4).
//! * [`diag`] — diagnostics shared by loading, linting and scanning.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod diag;
pub mod expr;
pub mod model;
pub mod scan;
pub mod sfc;
pub mod sim;
pub mod vars;

pub use diag::{Diagnostic, Severity};
pub use expr::{compare_values, eval, parse, BinaryOp, EvalError, Expr, UnaryOp, Value, VarSource};
pub use model::{
    CounterKind, ElementKind, PlacedElement, Project, RegisterMode, Rung, ScanConfig, Section,
    SectionLanguage, Symbol, TimerMode, SCHEMA_VERSION,
};
pub use scan::{
    lint, Counter, EdgeBank, RegisterState, ScanEngine, ScanReport, StoreError, TimeBase, TimerIec,
    VarStore,
};
pub use sfc::{SequentialPage, Step, Transition};
pub use sim::{PanelState, SimAnalog, SimGauge, SimLamp, SimReading, SimSwitch, SimulationPanel};
pub use vars::{Accessor, VarKind, VarParseError, VarRef};
