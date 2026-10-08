//! HAL driver — **planned for M7**.
//!
//! Nothing in this module is implemented yet. The M7 driver will expose pins
//! and signals compatible with a LinuxCNC-style HAL so that a SoftLadder
//! program can be wired next to motion control logic without a plugin per
//! machine.
//!
//! As with every other backend the implementation plugs into
//! [`crate::IoDriver`]; the scan engine itself stays free of hardware
//! concerns.

/// Milestone that delivers the HAL driver.
pub const MILESTONE: &str = "M7";
