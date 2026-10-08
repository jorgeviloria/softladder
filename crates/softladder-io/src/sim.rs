//! In-memory simulation driver.
//!
//! The simulation driver keeps its own copy of the process image so that tests
//! (and the editor's "simulate" mode) can drive inputs and observe outputs
//! without hardware. [`IoDriver::read`] copies inputs out of the driver into
//! the caller's image and [`IoDriver::write`] copies outputs back in; when the
//! two images differ in size only the overlapping channels are exchanged, so a
//! mismatch can never panic.

use crate::image::{IoDriver, IoError, IoImage};

/// Digital and analog channel counts used by [`SimDriver::default`].
pub const DEFAULT_DIGITAL_CHANNELS: usize = 16;
/// Analog channel count used by [`SimDriver::default`].
pub const DEFAULT_ANALOG_CHANNELS: usize = 4;

/// Simulation backend holding the field side of the process image.
#[derive(Debug, Clone, PartialEq)]
pub struct SimDriver {
    image: IoImage,
}

impl Default for SimDriver {
    fn default() -> Self {
        Self::new(DEFAULT_DIGITAL_CHANNELS, DEFAULT_ANALOG_CHANNELS)
    }
}

impl SimDriver {
    /// Creates a simulator with `digital` digital and `analog` analog channels.
    pub fn new(digital: usize, analog: usize) -> Self {
        Self {
            image: IoImage::new(digital, analog),
        }
    }

    /// Sets a simulated digital input, as a panel button would.
    pub fn set_input(&mut self, channel: usize, value: bool) -> Result<(), IoError> {
        self.image.set_digital_input(channel, value)
    }

    /// Reads a simulated digital output.
    pub fn get_output(&self, channel: usize) -> Option<bool> {
        self.image.digital_output(channel)
    }

    /// Sets a simulated analog input.
    pub fn set_analog_input(&mut self, channel: usize, value: f64) -> Result<(), IoError> {
        self.image.set_analog_input(channel, value)
    }

    /// Reads a simulated analog output.
    pub fn get_analog_output(&self, channel: usize) -> Option<f64> {
        self.image.analog_output(channel)
    }

    /// The driver-side image, for inspection in tests and tooling.
    pub fn image(&self) -> &IoImage {
        &self.image
    }

    /// Mutable access to the driver-side image.
    pub fn image_mut(&mut self) -> &mut IoImage {
        &mut self.image
    }
}

impl IoDriver for SimDriver {
    fn name(&self) -> &str {
        "sim"
    }

    fn read(&mut self, image: &mut IoImage) -> Result<(), IoError> {
        let digital = image.digital_in.len().min(self.image.digital_in.len());
        image.digital_in[..digital].copy_from_slice(&self.image.digital_in[..digital]);

        let analog = image.analog_in.len().min(self.image.analog_in.len());
        image.analog_in[..analog].copy_from_slice(&self.image.analog_in[..analog]);
        Ok(())
    }

    fn write(&mut self, image: &IoImage) -> Result<(), IoError> {
        let digital = image.digital_out.len().min(self.image.digital_out.len());
        self.image.digital_out[..digital].copy_from_slice(&image.digital_out[..digital]);

        let analog = image.analog_out.len().min(self.image.analog_out.len());
        self.image.analog_out[..analog].copy_from_slice(&image.analog_out[..analog]);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inputs_and_outputs_round_trip_through_the_image() {
        let mut driver = SimDriver::new(4, 2);
        assert_eq!(driver.name(), "sim");

        driver.set_input(0, true).expect("channel in range");
        driver.set_analog_input(1, 12.5).expect("channel in range");

        let mut image = IoImage::new(4, 2);
        driver.read(&mut image).expect("read succeeds");
        assert_eq!(image.digital_input(0), Some(true));
        assert_eq!(image.digital_input(1), Some(false));
        assert_eq!(image.analog_input(1), Some(12.5));

        image.set_digital_output(2, true).expect("channel in range");
        image.set_analog_output(0, -1.25).expect("channel in range");
        driver.write(&image).expect("write succeeds");
        assert_eq!(driver.get_output(2), Some(true));
        assert_eq!(driver.get_output(0), Some(false));
        assert_eq!(driver.get_analog_output(0), Some(-1.25));
    }

    #[test]
    fn mismatched_image_sizes_only_exchange_the_overlap() {
        let mut driver = SimDriver::new(2, 1);
        driver.set_input(1, true).expect("channel in range");
        let mut image = IoImage::new(8, 4);
        driver.read(&mut image).expect("read succeeds");
        assert_eq!(image.digital_input(1), Some(true));
        assert_eq!(image.digital_input(7), Some(false));

        let mut small = IoImage::new(1, 1);
        small.set_digital_output(0, true).expect("channel in range");
        driver.write(&small).expect("write succeeds");
        assert_eq!(driver.get_output(0), Some(true));
        assert_eq!(driver.get_output(1), Some(false));
    }

    #[test]
    fn out_of_range_channels_are_reported() {
        let mut driver = SimDriver::default();
        assert!(matches!(
            driver.set_input(99, true),
            Err(IoError::DigitalOutOfRange { .. })
        ));
        assert_eq!(driver.get_output(99), None);
        assert_eq!(driver.image().digital_in.len(), DEFAULT_DIGITAL_CHANNELS);
        assert_eq!(driver.image().analog_in.len(), DEFAULT_ANALOG_CHANNELS);
    }
}
