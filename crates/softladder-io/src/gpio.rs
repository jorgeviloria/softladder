//! GPIO driver — **planned for M5**.
//!
//! Nothing in this module is implemented yet. The M5 driver will talk to the
//! Linux GPIO character device (`/dev/gpiochipN`) and mirror ClassicLadder's
//! Raspberry Pi backend: one `%I`/`%Q` channel per configured line, with an
//! optional inverted logic flag per line.
//!
//! The driver is deliberately kept behind the [`crate::IoDriver`] trait so that
//! the scan engine, the editor and the tests never depend on a GPIO library.

/// Milestone that delivers the GPIO driver.
pub const MILESTONE: &str = "M5";
