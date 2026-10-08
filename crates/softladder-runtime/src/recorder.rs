//! Bounded ring buffer of recent variable values.
//!
//! The flight recorder is the black box of a SoftLadder runtime: the caller
//! decides which variables matter and records their values once per scan. Once
//! the buffer is full the oldest entry is dropped, so memory use stays bounded
//! no matter how long a program runs.

use std::collections::VecDeque;

use softladder_core::{Value, VarRef};

/// Size used by [`FlightRecorder::default`].
pub const DEFAULT_CAPACITY: usize = 1024;

/// One recorded frame: the scan number and the variables sampled with it.
pub type Frame = (u64, Vec<(VarRef, Value)>);

/// Ring buffer of sampled variable values.
#[derive(Debug, Clone, PartialEq)]
pub struct FlightRecorder {
    capacity: usize,
    frames: VecDeque<Frame>,
}

impl Default for FlightRecorder {
    fn default() -> Self {
        Self::new(DEFAULT_CAPACITY)
    }
}

impl FlightRecorder {
    /// Creates a recorder that keeps at most `capacity` frames.
    ///
    /// A capacity of zero is treated as one so that a recorder always keeps the
    /// most recent frame.
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            frames: VecDeque::new(),
        }
    }

    /// Maximum number of frames kept.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Appends a frame, evicting the oldest one when the buffer is full.
    pub fn record(&mut self, tick: u64, vars: Vec<(VarRef, Value)>) {
        while self.frames.len() >= self.capacity {
            self.frames.pop_front();
        }
        self.frames.push_back((tick, vars));
    }

    /// Returns a copy of the recorded frames, oldest first.
    pub fn dump(&self) -> Vec<Frame> {
        self.frames.iter().cloned().collect()
    }

    /// Number of recorded frames.
    pub fn len(&self) -> usize {
        self.frames.len()
    }

    /// `true` when nothing has been recorded yet.
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// Drops every recorded frame.
    pub fn clear(&mut self) {
        self.frames.clear();
    }

    /// The most recently recorded frame, if any.
    pub fn last(&self) -> Option<&Frame> {
        self.frames.back()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn var(text: &str) -> VarRef {
        text.parse::<VarRef>().expect("test variable parses")
    }

    #[test]
    fn frames_are_returned_oldest_first() {
        let mut recorder = FlightRecorder::new(4);
        recorder.record(1, vec![(var("%Q0"), Value::Bit(false))]);
        recorder.record(2, vec![(var("%Q0"), Value::Bit(true))]);
        assert_eq!(recorder.len(), 2);
        let dump = recorder.dump();
        assert_eq!(dump[0].0, 1);
        assert_eq!(dump[1].0, 2);
        assert_eq!(dump[1].1, vec![(var("%Q0"), Value::Bit(true))]);
        assert_eq!(recorder.last().map(|frame| frame.0), Some(2));
    }

    #[test]
    fn old_frames_are_evicted() {
        let mut recorder = FlightRecorder::new(2);
        for tick in 0..5 {
            recorder.record(tick, Vec::new());
        }
        assert_eq!(recorder.len(), 2);
        assert_eq!(
            recorder
                .dump()
                .iter()
                .map(|frame| frame.0)
                .collect::<Vec<_>>(),
            vec![3, 4]
        );
        assert_eq!(recorder.capacity(), 2);
    }

    #[test]
    fn capacity_is_never_zero() {
        let mut recorder = FlightRecorder::new(0);
        recorder.record(0, Vec::new());
        assert_eq!(recorder.len(), 1);
        assert!(!recorder.is_empty());
        recorder.clear();
        assert!(recorder.is_empty());
        assert_eq!(FlightRecorder::default().capacity(), DEFAULT_CAPACITY);
    }
}
