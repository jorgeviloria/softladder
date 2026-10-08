//! I/O image, driver abstraction and simulation driver for SoftLadder.
//!
//! The scan engine never touches hardware. Instead every driver exchanges data
//! through an [`IoImage`]: [`IoDriver::read`] copies the physical inputs into
//! the image before a scan, and [`IoDriver::write`] pushes the computed outputs
//! back out after it. Keeping the boundary this narrow means the whole control
//! program can be tested against [`SimDriver`] with no hardware at all.
//!
//! Real drivers are scheduled for later milestones:
//!
//! * [`modbus`] — Modbus TCP/RTU client and server (M5).
//! * [`gpio`] — Linux GPIO character device and Raspberry Pi headers (M5).
//! * [`hal`] — LinuxCNC-style HAL pins and signals (M7).

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod gpio;
pub mod hal;
pub mod image;
pub mod modbus;
pub mod sim;

pub use image::{IoDriver, IoError, IoImage};
pub use sim::SimDriver;
