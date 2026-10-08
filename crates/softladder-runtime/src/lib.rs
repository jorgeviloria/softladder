//! Runtime supervisor for SoftLadder.
//!
//! The runtime owns the project, the deterministic [`ScanEngine`] from
//! `softladder-core` and the lifecycle state machine that decides when a scan
//! may run. Ladder logic itself never reads the clock; the supervisor is the
//! only place that consults [`std::time::Instant`], and only to fill in the
//! [`ScanStats`] performance counters.
//!
//! [`FlightRecorder`] keeps a bounded ring buffer of the variable values seen
//! around each scan, which is what makes a field incident reproducible.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod recorder;
pub mod runtime;

pub use recorder::FlightRecorder;
pub use runtime::{Runtime, RuntimeState, ScanStats};
