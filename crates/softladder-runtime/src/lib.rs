//! Runtime supervisor for SoftLadder.
//!
//! The runtime owns the project, the deterministic [`ScanEngine`] from
//! `softladder-core` and the lifecycle state machine that decides when a scan
//! may run. Ladder logic itself never reads the clock; [`Clock`] is the only
//! place that decides which `now_ms` each cycle receives, and the simulation mode
//! of [`Clock`] makes a batch of cycles reproducible.
//!
//! [`FlightRecorder`] keeps a bounded ring buffer of the variable values seen
//! around each scan, which is what makes a field incident reproducible.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod recorder;
pub mod runtime;

pub use recorder::FlightRecorder;
pub use runtime::{Clock, Runtime, RuntimeState, ScanStats, ScanSummary};
