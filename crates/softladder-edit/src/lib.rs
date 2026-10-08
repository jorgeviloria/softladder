//! Application layer of the SoftLadder editor.
//!
//! The UI holds no authoritative state (see `docs/ARCHITECTURE.md` §9): it
//! renders [`Project`] and [`Runtime`] and emits [`Command`]s. This crate is
//! what sits behind that contract:
//!
//! * [`Editor`] — the authoritative [`Project`], the current file, the dirty
//!   flag, bounded undo/redo history and the Problems list. Every edit goes
//!   through a [`Command`], so the whole editing model is testable without a
//!   window or an event loop.
//! * [`Command`] — one reversible edit. [`Editor::apply`] validates it, mutates
//!   the project and records only what it touched.
//! * [`Bench`] — the simulation bench: the project's [`SimulationPanel`] plus
//!   the operator's [`PanelState`], driven by a [`Runtime`] with a deterministic
//!   [`Clock::simulated`]. It is a separate value from the [`Editor`], so a
//!   program can be edited and hot-reloaded while a bench keeps running.
//! * [`EditError`] — every rejection is an error value; nothing in this crate
//!   panics on malformed input.
//!
//! [`Project`]: softladder_core::Project
//! [`SimulationPanel`]: softladder_core::SimulationPanel
//! [`PanelState`]: softladder_core::PanelState
//! [`Clock::simulated`]: softladder_runtime::Clock::simulated

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod bench;
pub mod command;
pub mod editor;
pub mod error;
mod history;

pub use bench::Bench;
pub use command::Command;
pub use editor::{Editor, MAX_HISTORY};
pub use error::EditError;

/// The report a [`Bench::step`] returns, re-exported from `softladder-core`.
pub use softladder_core::ScanReport;
/// The supervisor a [`Bench`] wraps, re-exported from `softladder-runtime`.
pub use softladder_runtime::Runtime;
/// The lifecycle state of a [`Bench`], re-exported from `softladder-runtime`.
pub use softladder_runtime::RuntimeState;
