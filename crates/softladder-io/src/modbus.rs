//! Modbus driver — **planned for M5**.
//!
//! Nothing in this module is implemented yet. The M5 driver will implement
//! [`crate::IoDriver`] on top of Modbus TCP and Modbus RTU, mapping coils and
//! discrete inputs to `digital_in`/`digital_out` and input/output registers to
//! `analog_in`/`analog_out`. The register mapping is described by
//! `%IW`/`%QW` variable ranges so that a project stays portable between a
//! simulated bench and real hardware.
//!
//! Until then, importing or exporting a ClassicLadder project with Modbus
//! configuration is rejected by `softladder-project` with
//! `ProjectError::NotYetImplemented`.

/// Milestone that delivers the Modbus driver.
pub const MILESTONE: &str = "M5";
