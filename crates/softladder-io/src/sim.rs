//! In-memory simulation driver, scriptable from `.sltest` scenarios.
//!
//! The simulation driver keeps its own copy of the process image so that tests
//! (and the editor's "simulate" mode) can drive inputs and observe outputs
//! without hardware. [`IoDriver::read`] copies inputs out of the driver into
//! the caller's image and [`IoDriver::write`] copies outputs back in; when the
//! two images differ in size only the overlapping channels are exchanged, so a
//! mismatch can never panic.
//!
//! A [`SimScript`] is the deterministic input timeline the M8 `.sltest`
//! harness will run: a sorted list of [`SimEvent`]s keyed by scan cycle.
//! [`ScriptPlayer`] applies each event exactly once while a run advances,
//! so the same script and the same program always produce the same outputs.

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

    /// Applies one scripted event, as the `.sltest` timeline would.
    ///
    /// Channels outside the driver image are skipped, following the same
    /// overlap rule as [`IoDriver::read`]: a script written for a larger PLC
    /// can never panic a smaller one.
    pub fn apply_event(&mut self, event: &SimEvent) {
        for (channel, value) in &event.digital {
            let _ = self.image.set_digital_input(*channel, *value);
        }
        for (channel, value) in &event.analog {
            let _ = self.image.set_analog_input(*channel, *value);
        }
    }
}

/// One input change at a given scan cycle (0-based).
///
/// An event typically carries a single channel, but grouping the channels
/// that change on the same cycle keeps multi-channel timelines readable.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SimEvent {
    /// Scan cycle the change applies to, counting from `0`.
    pub cycle: u64,
    /// Digital inputs to set: `(channel, value)` pairs.
    pub digital: Vec<(usize, bool)>,
    /// Analog inputs to set: `(channel, value)` pairs.
    pub analog: Vec<(usize, f64)>,
}

impl SimEvent {
    /// Creates an event at `cycle` with the given channel changes.
    pub fn new(cycle: u64, digital: Vec<(usize, bool)>, analog: Vec<(usize, f64)>) -> Self {
        Self {
            cycle,
            digital,
            analog,
        }
    }
}

/// Deterministic input timeline for a simulated run.
///
/// Events are sorted by cycle on construction (stable), so a hand-written
/// scenario does not need to be ordered. An empty script is the manual
/// bench: inputs only change through [`SimDriver::set_input`].
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SimScript {
    events: Vec<SimEvent>,
}

impl SimScript {
    /// Creates a script from `events`, sorting them by cycle.
    pub fn new(mut events: Vec<SimEvent>) -> Self {
        events.sort_by_key(|event| event.cycle);
        Self { events }
    }

    /// Events in the order they run.
    pub fn events(&self) -> &[SimEvent] {
        &self.events
    }

    /// `true` when the script carries no event.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Number of events in the script.
    pub fn len(&self) -> usize {
        self.events.len()
    }
}

/// Applies a [`SimScript`] to a driver while a run advances.
///
/// [`ScriptPlayer::advance`] runs every event with `event.cycle <= cycle`
/// that has not run yet, in script order, and counts them. Create a new
/// player (or call [`ScriptPlayer::reset`]) when a run restarts from cycle
/// `0`; going backwards never re-runs events by itself.
#[derive(Debug, Clone)]
pub struct ScriptPlayer {
    script: SimScript,
    next: usize,
}

impl ScriptPlayer {
    /// Creates a player at the start of `script`.
    pub fn new(script: SimScript) -> Self {
        Self { script, next: 0 }
    }

    /// Script being played.
    pub fn script(&self) -> &SimScript {
        &self.script
    }

    /// Index of the next event to run.
    pub fn next(&self) -> usize {
        self.next
    }

    /// Rewinds to the start of the script.
    pub fn reset(&mut self) {
        self.next = 0;
    }

    /// Applies every pending event with `event.cycle <= cycle`.
    ///
    /// Returns how many events ran.
    pub fn advance(&mut self, driver: &mut SimDriver, cycle: u64) -> usize {
        let mut applied = 0;
        while self.next < self.script.events.len() && self.script.events[self.next].cycle <= cycle {
            let event = self.script.events[self.next].clone();
            driver.apply_event(&event);
            self.next += 1;
            applied += 1;
        }
        applied
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

    #[test]
    fn a_script_sorts_events_and_plays_each_exactly_once() {
        let script = SimScript::new(vec![
            SimEvent::new(2, vec![(0, false)], vec![]),
            SimEvent::new(0, vec![(0, true)], vec![(0, 1.5)]),
            SimEvent::new(1, vec![(1, true)], vec![]),
        ]);
        assert_eq!(script.len(), 3);
        assert!(!script.is_empty());
        assert_eq!(
            script
                .events()
                .iter()
                .map(|event| event.cycle)
                .collect::<Vec<_>>(),
            vec![0, 1, 2]
        );

        let mut driver = SimDriver::new(4, 2);
        let mut player = ScriptPlayer::new(script);
        assert_eq!(player.next(), 0);

        assert_eq!(player.advance(&mut driver, 0), 1);
        assert!(driver.image().digital_in[0]);
        assert_eq!(driver.image().analog_in[0], 1.5);
        // Replaying the same cycle runs nothing twice.
        assert_eq!(player.advance(&mut driver, 0), 0);

        assert_eq!(player.advance(&mut driver, 1), 1);
        assert!(driver.image().digital_in[1]);

        assert_eq!(player.advance(&mut driver, 5), 1);
        assert!(!driver.image().digital_in[0]);
        assert_eq!(player.next(), 3);
    }

    #[test]
    fn scripted_channels_out_of_range_are_skipped() {
        let mut driver = SimDriver::new(1, 1);
        driver.apply_event(&SimEvent::new(
            0,
            vec![(0, true), (99, true)],
            vec![(0, 2.5), (99, 2.5)],
        ));
        assert!(driver.image().digital_in[0]);
        assert_eq!(driver.image().analog_in[0], 2.5);
    }

    #[test]
    fn resetting_the_player_replays_the_script() {
        let script = SimScript::new(vec![SimEvent::new(0, vec![(0, true)], vec![])]);
        let mut player = ScriptPlayer::new(script);
        let mut driver = SimDriver::new(2, 0);
        assert_eq!(player.advance(&mut driver, 0), 1);
        player.reset();
        assert_eq!(player.next(), 0);
        driver.set_input(0, false).expect("channel in range");
        assert_eq!(player.advance(&mut driver, 0), 1);
        assert!(driver.image().digital_in[0]);
    }
}
