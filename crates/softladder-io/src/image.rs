//! The process image and the driver trait every I/O backend implements.

use thiserror::Error;

/// Error returned by an I/O driver or by a channel access.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum IoError {
    /// A digital channel index is outside the image.
    #[error("digital channel {channel} is out of range ({channels} channels)")]
    DigitalOutOfRange {
        /// Requested channel.
        channel: usize,
        /// Number of channels in the image.
        channels: usize,
    },
    /// An analog channel index is outside the image.
    #[error("analog channel {channel} is out of range ({channels} channels)")]
    AnalogOutOfRange {
        /// Requested channel.
        channel: usize,
        /// Number of channels in the image.
        channels: usize,
    },
    /// The backend itself failed.
    #[error("driver `{driver}` failed: {message}")]
    Driver {
        /// Name of the failing driver.
        driver: String,
        /// Human readable cause.
        message: String,
    },
}

/// Snapshot of the process image exchanged with the field.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct IoImage {
    /// Digital inputs read from the field.
    pub digital_in: Vec<bool>,
    /// Digital outputs written to the field.
    pub digital_out: Vec<bool>,
    /// Analog inputs read from the field.
    pub analog_in: Vec<f64>,
    /// Analog outputs written to the field.
    pub analog_out: Vec<f64>,
}

impl IoImage {
    /// Creates an image with `digital` digital and `analog` analog channels.
    pub fn new(digital: usize, analog: usize) -> Self {
        Self {
            digital_in: vec![false; digital],
            digital_out: vec![false; digital],
            analog_in: vec![0.0; analog],
            analog_out: vec![0.0; analog],
        }
    }

    /// Number of digital channels.
    pub fn digital_channels(&self) -> usize {
        self.digital_in.len().min(self.digital_out.len())
    }

    /// Number of analog channels.
    pub fn analog_channels(&self) -> usize {
        self.analog_in.len().min(self.analog_out.len())
    }

    /// Reads a digital input.
    pub fn digital_input(&self, channel: usize) -> Option<bool> {
        self.digital_in.get(channel).copied()
    }

    /// Writes a digital input, as a driver would after reading the field.
    pub fn set_digital_input(&mut self, channel: usize, value: bool) -> Result<(), IoError> {
        match self.digital_in.get_mut(channel) {
            Some(slot) => {
                *slot = value;
                Ok(())
            }
            None => Err(IoError::DigitalOutOfRange {
                channel,
                channels: self.digital_in.len(),
            }),
        }
    }

    /// Reads a digital output.
    pub fn digital_output(&self, channel: usize) -> Option<bool> {
        self.digital_out.get(channel).copied()
    }

    /// Writes a digital output.
    pub fn set_digital_output(&mut self, channel: usize, value: bool) -> Result<(), IoError> {
        match self.digital_out.get_mut(channel) {
            Some(slot) => {
                *slot = value;
                Ok(())
            }
            None => Err(IoError::DigitalOutOfRange {
                channel,
                channels: self.digital_out.len(),
            }),
        }
    }

    /// Reads an analog input.
    pub fn analog_input(&self, channel: usize) -> Option<f64> {
        self.analog_in.get(channel).copied()
    }

    /// Writes an analog input.
    pub fn set_analog_input(&mut self, channel: usize, value: f64) -> Result<(), IoError> {
        match self.analog_in.get_mut(channel) {
            Some(slot) => {
                *slot = value;
                Ok(())
            }
            None => Err(IoError::AnalogOutOfRange {
                channel,
                channels: self.analog_in.len(),
            }),
        }
    }

    /// Reads an analog output.
    pub fn analog_output(&self, channel: usize) -> Option<f64> {
        self.analog_out.get(channel).copied()
    }

    /// Writes an analog output.
    pub fn set_analog_output(&mut self, channel: usize, value: f64) -> Result<(), IoError> {
        match self.analog_out.get_mut(channel) {
            Some(slot) => {
                *slot = value;
                Ok(())
            }
            None => Err(IoError::AnalogOutOfRange {
                channel,
                channels: self.analog_out.len(),
            }),
        }
    }

    /// Clears every value in the image.
    pub fn clear(&mut self) {
        self.digital_in.iter_mut().for_each(|value| *value = false);
        self.digital_out.iter_mut().for_each(|value| *value = false);
        self.analog_in.iter_mut().for_each(|value| *value = 0.0);
        self.analog_out.iter_mut().for_each(|value| *value = 0.0);
    }
}

/// A backend able to exchange the process image with the field.
pub trait IoDriver {
    /// Short driver name, used in diagnostics.
    fn name(&self) -> &str;

    /// Copies the field inputs into `image` before a scan.
    fn read(&mut self, image: &mut IoImage) -> Result<(), IoError>;

    /// Pushes the outputs from `image` to the field after a scan.
    fn write(&mut self, image: &IoImage) -> Result<(), IoError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_access_is_bounds_checked() {
        let mut image = IoImage::new(2, 1);
        assert_eq!(image.digital_input(0), Some(false));
        assert!(image.set_digital_input(1, true).is_ok());
        assert_eq!(image.digital_input(1), Some(true));
        assert_eq!(image.digital_output(0), Some(false));
        assert!(image.set_digital_output(0, true).is_ok());
        assert_eq!(image.digital_output(0), Some(true));
        assert!(image.set_analog_input(0, 3.5).is_ok());
        assert_eq!(image.analog_input(0), Some(3.5));
        assert_eq!(image.analog_output(0), Some(0.0));

        assert_eq!(
            image.set_digital_input(7, true),
            Err(IoError::DigitalOutOfRange {
                channel: 7,
                channels: 2
            })
        );
        assert_eq!(
            image.set_analog_output(7, 1.0),
            Err(IoError::AnalogOutOfRange {
                channel: 7,
                channels: 1
            })
        );
        assert_eq!(image.digital_input(9), None);
        assert_eq!(image.analog_output(9), None);
        assert_eq!(image.digital_channels(), 2);
        assert_eq!(image.analog_channels(), 1);
    }

    #[test]
    fn clearing_resets_every_channel() {
        let mut image = IoImage::new(1, 1);
        image.set_digital_input(0, true).expect("in range");
        image.set_digital_output(0, true).expect("in range");
        image.set_analog_input(0, 1.0).expect("in range");
        image.set_analog_output(0, 1.0).expect("in range");
        image.clear();
        assert_eq!(image, IoImage::new(1, 1));
    }
}
