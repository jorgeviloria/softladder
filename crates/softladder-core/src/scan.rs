//! Deterministic scan engine, variable store and function-block state.
//!
//! The engine never reads the clock: the caller passes the current simulated
//! time in milliseconds to [`ScanEngine::scan_once`]. Together with the fact
//! that sections and rungs run in declaration order and that no iteration order
//! depends on hashing, this makes a scan a pure function of
//! `(project, store, now_ms)` and therefore reproducible.
//!
//! # Power flow
//!
//! Rungs are evaluated column-major, rows top to bottom, exactly as
//! `docs/SEMANTICS.md` §2 specifies: the left power rail feeds column 0, and a
//! cell sees the OR of the outputs of the cells immediately to its left over
//! the whole vertical block it belongs to (`connected_with_top`). Horizontal
//! flow is implicit inside a *live* row — a row that holds at least one element
//! conducts through its empty cells — while a row with no elements at all is
//! inert and never conducts.
//!
//! The engine precomputes a `(col, row) -> element` cell index and a vertical
//! link table once per project shape (see [`ScanEngine::refresh`]), so
//! evaluating a cell never scans linearly and never allocates. The only
//! per-scan buffers are the two column buffers, which are reused across scans.
//!
//! # Divergences from ClassicLadder
//!
//! * Function blocks are single cells: their input rows are read at the block's
//!   own column (a counter occupies four rows, a register three) and their only
//!   wire output is the block's primary flag.
//! * The elapsed counter `%TM<n>.V` and the preset `%TM<n>.P` are counted in
//!   [`TimeBase`] units. The time base is derived from the preset parameter: a
//!   plain decimal literal means 100 ms per unit (the ClassicLadder scale), an
//!   `s`/`m` suffix means one second / one minute per unit, and an unsuffixed
//!   variable is interpreted as milliseconds.
//! * A coil's output is its input, which makes serial coils behave as expected.

use std::collections::{BTreeMap, HashMap, VecDeque};

use serde::{Deserialize, Serialize};

use crate::diag::{Diagnostic, Severity};
use crate::expr::{eval, parse, Value, VarSource};
use crate::model::{
    CounterKind, ElementKind, PlacedElement, Project, RegisterMode, Rung, Section, SectionLanguage,
    TimerMode,
};
use crate::vars::{Accessor, VarKind, VarRef};

/// Default number of internal bit memories (`%M`).
pub const DEFAULT_MEM_BITS: usize = 500;
/// Default number of internal words (`%MW`).
pub const DEFAULT_MEM_WORDS: usize = 200;
/// Default number of physical input bits (`%I`).
pub const DEFAULT_PHYS_INPUTS: usize = 50;
/// Default number of physical output bits (`%Q`).
pub const DEFAULT_PHYS_OUTPUTS: usize = 50;
/// Default number of physical input words (`%IW`).
pub const DEFAULT_PHYS_INPUT_WORDS: usize = 25;
/// Default number of physical output words (`%QW`).
pub const DEFAULT_PHYS_OUTPUT_WORDS: usize = 25;
/// Default number of IEC timers (`%TM`).
pub const DEFAULT_TIMERS: usize = 50;
/// Default number of counters (`%C`).
pub const DEFAULT_COUNTERS: usize = 50;
/// Default number of registers (`%R`).
pub const DEFAULT_REGISTERS: usize = 10;
/// Default number of sequential steps (`%X`).
pub const DEFAULT_STEPS: usize = 128;
/// Default number of system bits (`%S`).
pub const DEFAULT_SYSTEM_BITS: usize = 50;
/// Default number of user LEDs (`%QLED`).
pub const DEFAULT_LEDS: usize = 8;

/// Default number of slots in a register (FIFO/LIFO) block.
pub const DEFAULT_REGISTER_CAPACITY: usize = 500;

/// Largest index the store is willing to grow to on demand.
///
/// The store follows `docs/SEMANTICS.md` §6 and grows when a variable outside
/// its current capacity is written, but growth is bounded so that an indirect
/// index such as `%MW[%MW0]` can never allocate an unbounded amount of memory.
pub const MAX_GROWTH: usize = 100_000;

/// Largest `delta_ms` the engine feeds to a function block, in milliseconds.
///
/// Clamping keeps a paused or restarted clock from advancing every timer by an
/// arbitrary amount in a single scan.
pub const MAX_DELTA_MS: u64 = 1_000;

/// Number of jumps allowed in a single scan before the mad-loop guard trips.
pub const MAD_LOOP_LIMIT: u64 = 100_000;

/// Maximum number of nested subroutine calls.
pub const MAX_CALL_DEPTH: usize = 25;

/// Highest value a counter reaches before wrapping to zero.
pub const COUNTER_MAX: i32 = 9_999;

/// Error returned when a value cannot be stored into a variable.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StoreError {
    /// The variable index is outside the range the store can address.
    #[error("variable `{0}` is out of range")]
    OutOfRange(VarRef),
    /// The variable exists but its sub-value cannot be written.
    #[error("variable `{0}` is not writable")]
    NotWritable(VarRef),
    /// The indirect index expression could not be resolved.
    #[error("cannot resolve the index of `{0}`: {1}")]
    BadIndex(VarRef, String),
}

/// Time base of an IEC timer: the duration of one `%TM<n>.V` unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum TimeBase {
    /// 100 milliseconds per unit, the ClassicLadder default.
    #[default]
    Millis100,
    /// One second per unit.
    Second,
    /// Sixty minutes per unit.
    Minute60,
}

impl TimeBase {
    /// Duration of one unit of this base, in milliseconds.
    pub fn millis(self) -> u64 {
        match self {
            TimeBase::Millis100 => 100,
            TimeBase::Second => 1_000,
            TimeBase::Minute60 => 60 * 60 * 1_000,
        }
    }

    /// Human readable name used in diagnostics and the editor.
    pub fn name(self) -> &'static str {
        match self {
            TimeBase::Millis100 => "100 ms",
            TimeBase::Second => "1 s",
            TimeBase::Minute60 => "60 min",
        }
    }
}

/// State of an IEC 61131-3 timer block.
///
/// `%TM<n>.V` (elapsed) and `%TM<n>.P` (preset) are counted in [`TimeBase`]
/// units; the millisecond accumulator that quantizes the real scan period is
/// private and never observed directly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimerIec {
    /// Timer flavour.
    pub mode: TimerMode,
    /// Duration of one elapsed/preset unit.
    pub time_base: TimeBase,
    /// Preset duration, in time-base units.
    pub preset: u64,
    /// Elapsed duration, in time-base units.
    pub elapsed: u64,
    /// Whether the timer output (`%TM<n>.Q`) is currently true.
    pub done: bool,
    /// Milliseconds accumulated but not yet converted into a unit.
    acc_ms: u64,
    /// Input flow seen during the previous scan.
    prev_input: bool,
    /// `false` until the first scan observed the input, so that the first scan
    /// can never look like a falling edge.
    seen_input: bool,
    /// `true` while a TOF off-delay is counting down.
    running: bool,
}

impl Default for TimerIec {
    fn default() -> Self {
        Self {
            mode: TimerMode::On,
            time_base: TimeBase::Millis100,
            preset: 0,
            elapsed: 0,
            done: false,
            acc_ms: 0,
            prev_input: false,
            seen_input: false,
            running: false,
        }
    }
}

impl TimerIec {
    /// Creates a timer with a flavour, a preset in time-base units and a base.
    pub fn new(mode: TimerMode, preset: u64, time_base: TimeBase) -> Self {
        Self {
            mode,
            time_base,
            preset,
            ..Self::default()
        }
    }

    /// Preset duration expressed in milliseconds, saturating on overflow.
    pub fn preset_millis(&self) -> u64 {
        self.preset.saturating_mul(self.time_base.millis())
    }

    /// Sets the preset in time-base units.
    ///
    /// Shrinking the preset below the already elapsed duration restarts the
    /// timer, so that a lowered preset takes effect on the next count.
    pub fn set_preset(&mut self, preset: u64) {
        self.preset = preset;
        if self.done && self.elapsed < self.preset {
            self.done = false;
            self.running = false;
            self.elapsed = 0;
            self.acc_ms = 0;
        }
    }

    /// Sets the preset and the time base together.
    pub fn configure(&mut self, preset: u64, time_base: TimeBase) {
        self.time_base = time_base;
        self.set_preset(preset);
    }

    /// Clears the accumulated time, the elapsed counter and the output.
    pub fn reset(&mut self) {
        self.elapsed = 0;
        self.acc_ms = 0;
        self.done = false;
        self.running = false;
    }

    /// Runs one scan of the timer and returns its output (`%TM<n>.Q`).
    ///
    /// `delta_ms` is the real time since the previous scan; the caller clamps it
    /// ([`ScanEngine`] clamps at [`MAX_DELTA_MS`]). The first call with
    /// `delta_ms == 0` only establishes the edge state.
    pub fn update(&mut self, delta_ms: u64, input: bool) -> bool {
        match self.mode {
            TimerMode::On => {
                self.update_on(delta_ms, input);
            }
            TimerMode::Off => {
                self.update_off(delta_ms, input);
            }
            TimerMode::Pulse => {
                self.update_pulse(delta_ms, input);
            }
        }
        self.done
    }

    /// On-delay: count while the input is present, drop out when it is lost.
    fn update_on(&mut self, delta_ms: u64, input: bool) -> bool {
        if !input {
            self.reset();
        } else if !self.done && self.advance(delta_ms) {
            self.done = true;
        }
        self.done
    }

    /// Off-delay: the output follows the input, and the falling edge starts the
    /// delay before the output drops.
    ///
    /// The phase is persistent: once the input falls the delay keeps counting
    /// across scans until it reaches the preset, even though the edge itself
    /// happened in an earlier scan.
    fn update_off(&mut self, delta_ms: u64, input: bool) -> bool {
        if input {
            // The output follows the input while it is high, and the delay is
            // re-armed from scratch every time the input returns.
            self.elapsed = 0;
            self.acc_ms = 0;
            self.done = true;
            self.running = false;
        } else if self.seen_input && self.prev_input {
            // Falling edge: start the off delay.
            self.running = true;
            self.elapsed = 0;
            self.acc_ms = 0;
        }
        let reached = !input && self.running && self.advance(delta_ms);
        if reached {
            self.reset();
        }
        self.prev_input = input;
        self.seen_input = true;
        self.done
    }

    /// Pulse: a rising edge starts a non-retriggerable one-shot.
    ///
    /// The input is edge sensed and the pulse lasts exactly one preset in
    /// wall-clock time. The output stays high after the preset has elapsed, until
    /// the input drops: a high input never starts a second pulse, so the one-shot
    /// cannot be retriggered.
    fn update_pulse(&mut self, delta_ms: u64, input: bool) -> bool {
        if self.running {
            if self.advance(delta_ms) {
                // The preset elapsed: the pulse is over, but the output stays
                // high until the input drops.
                self.running = false;
                self.acc_ms = 0;
            }
        } else if input && !self.prev_input {
            self.done = true;
            self.elapsed = 0;
            self.acc_ms = 0;
            self.running = true;
        } else if !input {
            self.reset();
        }
        self.prev_input = input;
        self.seen_input = true;
        self.done
    }

    /// Accumulates `delta_ms` and turns it into elapsed time-base units.
    ///
    /// The elapsed counter tracks wall-clock time: a scan advances it by as many
    /// whole time-base units as the elapsed milliseconds contain, and the
    /// remainder is carried into the next scan. That keeps a timer correct no
    /// matter how regular the scan period is, and the carry means an irregular
    /// period can neither lose nor gain time. Once the preset is reached the
    /// accumulator is cleared so a completed timer banks nothing.
    fn advance(&mut self, delta_ms: u64) -> bool {
        if self.elapsed >= self.preset {
            return true;
        }
        let base = self.time_base.millis().max(1);
        self.acc_ms = self.acc_ms.saturating_add(delta_ms);
        let units = self.acc_ms / base;
        if units > 0 {
            let taken = units.min(self.preset.saturating_sub(self.elapsed));
            self.elapsed += taken;
            self.acc_ms -= taken.saturating_mul(base);
        }
        if self.elapsed >= self.preset {
            self.acc_ms = 0;
        }
        self.elapsed >= self.preset
    }
}

/// State of a counter block.
///
/// The count wraps inside `0..=9999`; `empty` and `full` remember the direction
/// of the last wrap so that `%C<n>.E` ("wrapped down from zero") and
/// `%C<n>.F` ("wrapped up from 9999") keep their meaning until the next wrap.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Counter {
    /// Value at which `%C<n>.D` reports done.
    pub preset: i32,
    /// Current count, always inside `0..=9999`.
    pub value: i32,
    /// Counter flavour.
    pub kind: CounterKind,
    /// `true` when the count last wrapped down past zero.
    pub empty: bool,
    /// `true` when the count last wrapped up past 9999.
    pub full: bool,
    prev_reset: bool,
    prev_preset: bool,
    prev_up: bool,
    prev_down: bool,
    started: bool,
}

impl Default for Counter {
    fn default() -> Self {
        Self {
            preset: 0,
            value: 0,
            kind: CounterKind::Up,
            empty: false,
            full: false,
            prev_reset: false,
            prev_preset: false,
            prev_up: false,
            prev_down: false,
            started: false,
        }
    }
}

impl Counter {
    /// Creates a counter with a flavour and a preset.
    pub fn new(kind: CounterKind, preset: i32) -> Self {
        Self {
            kind,
            preset,
            ..Self::default()
        }
    }

    /// Runs one scan of the counter and returns its new value.
    ///
    /// The order of application follows the reference: count up, count down,
    /// preset, reset, so a simultaneous preset and reset ends at zero. The count
    /// inputs are edge triggered; the preset and reset inputs are level
    /// triggered. On the first call a down counter starts at its preset, which
    /// is what makes `CTD` usable without a separate load.
    pub fn update(&mut self, count_up: bool, count_down: bool, reset: bool, preset: bool) -> i32 {
        if self.kind == CounterKind::Down && !self.started {
            self.value = self.preset.clamp(0, COUNTER_MAX);
            self.started = true;
        }
        self.emulate(count_up, count_down, reset, preset)
    }

    /// Applies one scan of the four inputs without the one-shot "start at the
    /// preset" behaviour of [`Counter::update`] {@see ScanEngine}.
    pub fn emulate(&mut self, count_up: bool, count_down: bool, reset: bool, preset: bool) -> i32 {
        let up_edge = count_up && !self.prev_up;
        let down_edge = count_down && !self.prev_down;

        match self.kind {
            CounterKind::Up if up_edge => self.count(1),
            CounterKind::Down if down_edge => self.count(-1),
            CounterKind::UpDown => {
                if up_edge {
                    self.count(1);
                }
                if down_edge {
                    self.count(-1);
                }
            }
            _ => {}
        }

        if preset {
            self.value = self.preset.clamp(0, COUNTER_MAX);
        }
        if reset {
            self.value = 0;
            self.empty = false;
            self.full = false;
        }

        // One scan of a counter counts as "started", so `update` will not jump
        // to the preset on top of an `emulate` that already ran.
        self.started = true;
        self.prev_up = count_up;
        self.prev_down = count_down;
        self.prev_reset = reset;
        self.prev_preset = preset;
        self.value
    }

    /// Adds `step` to the count, wrapping inside `0..=9999`.
    fn count(&mut self, step: i32) {
        let next = self.value + step;
        if next > COUNTER_MAX {
            self.value = 0;
            self.full = true;
            self.empty = false;
        } else if next < 0 {
            self.value = COUNTER_MAX;
            self.empty = true;
            self.full = false;
        } else {
            self.value = next;
        }
    }

    /// `%C<n>.D`: the count has reached the preset.
    pub fn done(&self) -> bool {
        self.value == self.preset
    }

    /// `%C<n>.E`: the count wrapped down past zero and has not wrapped again.
    pub fn is_empty(&self) -> bool {
        self.empty
    }

    /// `%C<n>.F`: the count wrapped up past 9999 and has not wrapped again.
    pub fn is_full(&self) -> bool {
        self.full
    }

    /// Overrides the done bit by moving the count to (or away from) the preset.
    pub fn set_done(&mut self, done: bool) {
        self.value = if done {
            self.preset.clamp(0, COUNTER_MAX)
        } else {
            0
        };
    }

    /// Clears the count, the wrap flags and the remembered edges.
    pub fn clear(&mut self) {
        self.value = 0;
        self.empty = false;
        self.full = false;
        self.started = false;
        self.prev_reset = false;
        self.prev_preset = false;
        self.prev_up = false;
        self.prev_down = false;
    }
}

/// State of a register (FIFO/LIFO) block.
///
/// The buffer holds up to `capacity` values; pushing when full and popping when
/// empty are silent no-ops, as `docs/SEMANTICS.md` §3.7 requires.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RegisterState {
    /// Pop order: oldest first (FIFO) or newest first (LIFO).
    pub mode: RegisterMode,
    /// Maximum number of stored values.
    pub capacity: usize,
    /// Value offered to the block on `%R<n>.I`.
    pub in_value: i32,
    /// Value taken out of the block on `%R<n>.O`.
    pub out_value: i32,
    /// Stored values; the front is popped first in FIFO mode, the back in LIFO.
    pub values: VecDeque<i32>,
    prev_in: bool,
    prev_out: bool,
}

impl Default for RegisterState {
    fn default() -> Self {
        Self {
            mode: RegisterMode::Fifo,
            capacity: DEFAULT_REGISTER_CAPACITY,
            in_value: 0,
            out_value: 0,
            values: VecDeque::new(),
            prev_in: false,
            prev_out: false,
        }
    }
}

impl RegisterState {
    /// Creates an empty register with a pop order and a capacity.
    pub fn new(mode: RegisterMode, capacity: usize) -> Self {
        Self {
            mode,
            capacity,
            ..Self::default()
        }
    }

    /// Adjusts the capacity, dropping the oldest values when it shrinks.
    pub fn set_capacity(&mut self, capacity: usize) {
        self.capacity = capacity;
        while self.values.len() > self.capacity {
            self.values.pop_front();
        }
    }

    /// Runs one scan of the block. `reset`, `push` and `pop` are the flows
    /// arriving at the block's three input rows, top to bottom.
    ///
    /// The reset row is level triggered; the push and pop rows are edge
    /// triggered.
    pub fn update(&mut self, reset: bool, push: bool, pop: bool) {
        if reset {
            self.reset();
        } else {
            if push && !self.prev_in {
                let value = self.in_value;
                self.push(value);
            }
            if pop && !self.prev_out {
                if let Some(value) = self.pop() {
                    self.out_value = value;
                }
            }
        }
        self.prev_in = push;
        self.prev_out = pop;
    }

    /// Stores `value` unless the buffer is already full.
    pub fn push(&mut self, value: i32) -> bool {
        if self.is_full() {
            return false;
        }
        self.values.push_back(value);
        true
    }

    /// Removes and returns the next value, or [`None`] when the buffer is empty.
    pub fn pop(&mut self) -> Option<i32> {
        match self.mode {
            RegisterMode::Fifo => self.values.pop_front(),
            RegisterMode::Lifo => self.values.pop_back(),
        }
    }

    /// Drops every stored value and clears the output, as the reset row does.
    pub fn reset(&mut self) {
        self.values.clear();
        self.out_value = 0;
    }

    /// `%R<n>.E`: no value is stored.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// `%R<n>.F`: the buffer holds `capacity` values.
    pub fn is_full(&self) -> bool {
        self.values.len() >= self.capacity
    }

    /// `%R<n>.S`: the number of stored values.
    pub fn stored(&self) -> i32 {
        i32::try_from(self.values.len()).unwrap_or(i32::MAX)
    }
}

/// Previous-state bank used to detect rising and falling edges.
///
/// Only lookups are performed on the map, never iteration, so the result of a
/// scan stays deterministic. The engine itself tracks edges per *cell* (so that
/// a single element consumes an edge exactly once); this type is the public
/// helper for callers that need the same rule, and its first observation of a
/// variable counts as a transition from `false`.
#[derive(Debug, Clone, Default)]
pub struct EdgeBank {
    previous: HashMap<VarRef, bool>,
}

impl EdgeBank {
    /// Creates an empty bank.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns `true` on the scan where `value` rises from `false` to `true`.
    pub fn rising(&mut self, var: &VarRef, value: bool) -> bool {
        let previous = self.previous.insert(var.clone(), value).unwrap_or(false);
        value && !previous
    }

    /// Returns `true` on the scan where `value` falls from `true` to `false`.
    pub fn falling(&mut self, var: &VarRef, value: bool) -> bool {
        let previous = self.previous.insert(var.clone(), value).unwrap_or(false);
        !value && previous
    }

    /// Forgets every remembered state.
    pub fn clear(&mut self) {
        self.previous.clear();
    }

    /// Number of variables currently tracked.
    pub fn len(&self) -> usize {
        self.previous.len()
    }

    /// `true` when no variable is tracked yet.
    pub fn is_empty(&self) -> bool {
        self.previous.is_empty()
    }
}

/// Complete variable state of a running SoftLadder program.
///
/// The vectors are public so that I/O drivers can mirror the process image
/// cheaply; every access made through [`VarStore::get`] / [`VarStore::set`]
/// resolves indirect indices, dispatches on the effective accessor and honours
/// the store's growth rules.
#[derive(Debug, Clone, PartialEq)]
pub struct VarStore {
    /// Internal bit memory (`%M`).
    pub mem_bits: Vec<bool>,
    /// Internal words (`%MW`).
    pub mem_words: Vec<i32>,
    /// Physical input bits (`%I`).
    pub phys_in: Vec<bool>,
    /// Physical output bits (`%Q`).
    pub phys_out: Vec<bool>,
    /// Physical input words (`%IW`).
    pub phys_in_words: Vec<i32>,
    /// Physical output words (`%QW`).
    pub phys_out_words: Vec<i32>,
    /// IEC timer blocks (`%TM`).
    pub timers: Vec<TimerIec>,
    /// Counter blocks (`%C`).
    pub counters: Vec<Counter>,
    /// Register blocks (`%R`).
    pub registers: Vec<RegisterState>,
    /// Sequential step activity bits (`%X`).
    pub steps: Vec<bool>,
    /// Sequential step ages in milliseconds (`%X<n>.V`).
    pub step_ages: Vec<u64>,
    /// System bits (`%S`).
    pub system: Vec<bool>,
    /// User LED bits (`%QLED`).
    pub leds: Vec<bool>,
}

impl Default for VarStore {
    fn default() -> Self {
        Self::with_default_sizes()
    }
}

impl VarStore {
    /// Creates an empty store; every vector is grown on demand.
    pub fn new() -> Self {
        Self {
            mem_bits: Vec::new(),
            mem_words: Vec::new(),
            phys_in: Vec::new(),
            phys_out: Vec::new(),
            phys_in_words: Vec::new(),
            phys_out_words: Vec::new(),
            timers: Vec::new(),
            counters: Vec::new(),
            registers: Vec::new(),
            steps: Vec::new(),
            step_ages: Vec::new(),
            system: Vec::new(),
            leds: Vec::new(),
        }
    }

    /// Creates a store pre-allocated to the ClassicLadder default sizes.
    pub fn with_default_sizes() -> Self {
        Self {
            mem_bits: vec![false; DEFAULT_MEM_BITS],
            mem_words: vec![0; DEFAULT_MEM_WORDS],
            phys_in: vec![false; DEFAULT_PHYS_INPUTS],
            phys_out: vec![false; DEFAULT_PHYS_OUTPUTS],
            phys_in_words: vec![0; DEFAULT_PHYS_INPUT_WORDS],
            phys_out_words: vec![0; DEFAULT_PHYS_OUTPUT_WORDS],
            timers: vec![TimerIec::default(); DEFAULT_TIMERS],
            counters: vec![Counter::default(); DEFAULT_COUNTERS],
            registers: vec![RegisterState::default(); DEFAULT_REGISTERS],
            steps: vec![false; DEFAULT_STEPS],
            step_ages: vec![0; DEFAULT_STEPS],
            system: vec![false; DEFAULT_SYSTEM_BITS],
            leds: vec![false; DEFAULT_LEDS],
        }
    }

    /// Reads a variable, resolving indirect indices and accessors.
    ///
    /// Returns [`None`] when the variable does not exist in this store, when its
    /// index is out of range, or when the accessor does not apply to its kind
    /// (a combination `VarRef::from_str` rejects anyway).
    pub fn get(&self, var: &VarRef) -> Option<Value> {
        let index = self.resolve_index(var)?;
        match (var.kind, var.effective_accessor()) {
            (VarKind::MemBit, _) => Some(Value::Bit(*self.mem_bits.get(index)?)),
            (VarKind::MemWord, None) => Some(Value::Word(*self.mem_words.get(index)?)),
            (VarKind::PhysIn, _) => Some(Value::Bit(*self.phys_in.get(index)?)),
            (VarKind::PhysOut, _) => Some(Value::Bit(*self.phys_out.get(index)?)),
            (VarKind::PhysInWord, None) => Some(Value::Word(*self.phys_in_words.get(index)?)),
            (VarKind::PhysOutWord, None) => Some(Value::Word(*self.phys_out_words.get(index)?)),
            (VarKind::MemWord, Some(Accessor::Bit(bit))) => {
                Some(Value::Bit(word_bit(*self.mem_words.get(index)?, bit)))
            }
            (VarKind::PhysInWord, Some(Accessor::Bit(bit))) => {
                Some(Value::Bit(word_bit(*self.phys_in_words.get(index)?, bit)))
            }
            (VarKind::PhysOutWord, Some(Accessor::Bit(bit))) => {
                Some(Value::Bit(word_bit(*self.phys_out_words.get(index)?, bit)))
            }
            (VarKind::TimerIec, Some(Accessor::Done)) => {
                Some(Value::Bit(self.timers.get(index)?.done))
            }
            (VarKind::TimerIec, Some(Accessor::Value)) => Some(Value::Word(clamp_u64_to_i32(
                self.timers.get(index)?.elapsed,
            ))),
            (VarKind::TimerIec, Some(Accessor::Preset)) => Some(Value::Word(clamp_u64_to_i32(
                self.timers.get(index)?.preset,
            ))),
            (VarKind::Counter, Some(Accessor::Done)) => {
                Some(Value::Bit(self.counters.get(index)?.done()))
            }
            (VarKind::Counter, Some(Accessor::Value)) => {
                Some(Value::Word(self.counters.get(index)?.value))
            }
            (VarKind::Counter, Some(Accessor::Preset)) => {
                Some(Value::Word(self.counters.get(index)?.preset))
            }
            (VarKind::Counter, Some(Accessor::Empty)) => {
                Some(Value::Bit(self.counters.get(index)?.is_empty()))
            }
            (VarKind::Counter, Some(Accessor::Full)) => {
                Some(Value::Bit(self.counters.get(index)?.is_full()))
            }
            (VarKind::Register, Some(Accessor::Empty)) | (VarKind::Register, None) => {
                Some(Value::Bit(self.registers.get(index)?.is_empty()))
            }
            (VarKind::Register, Some(Accessor::Full)) => {
                Some(Value::Bit(self.registers.get(index)?.is_full()))
            }
            (VarKind::Register, Some(Accessor::In)) => {
                Some(Value::Word(self.registers.get(index)?.in_value))
            }
            (VarKind::Register, Some(Accessor::Out)) => {
                Some(Value::Word(self.registers.get(index)?.out_value))
            }
            (VarKind::Register, Some(Accessor::Count)) => {
                Some(Value::Word(self.registers.get(index)?.stored()))
            }
            (VarKind::Step, Some(Accessor::Activity)) | (VarKind::Step, None) => {
                Some(Value::Bit(*self.steps.get(index)?))
            }
            (VarKind::Step, Some(Accessor::Value)) => {
                Some(Value::Word(clamp_u64_to_i32(*self.step_ages.get(index)?)))
            }
            (VarKind::System, _) => Some(Value::Bit(*self.system.get(index)?)),
            (VarKind::Led, _) => Some(Value::Bit(*self.leds.get(index)?)),
            _ => None,
        }
    }

    /// Writes a variable, resolving indirect indices and accessors.
    ///
    /// The store grows on demand (up to [`MAX_GROWTH`]) so that a project may
    /// address variables beyond the default pre-allocation; an index past the
    /// growth limit is a [`StoreError::OutOfRange`], and a sub-value that cannot
    /// be written (for example `%R<n>.E`) is a [`StoreError::NotWritable`].
    pub fn set(&mut self, var: &VarRef, value: Value) -> Result<(), StoreError> {
        let index = self
            .resolve_index(var)
            .ok_or_else(|| self.bad_index(var, "the index cannot be resolved"))?;
        match (var.kind, var.effective_accessor()) {
            (VarKind::MemBit, _) => write_bit(&mut self.mem_bits, index, var, value.as_bool()),
            (VarKind::MemWord, None) => write_word(&mut self.mem_words, index, var, value.as_i64()),
            (VarKind::PhysIn, _) => write_bit(&mut self.phys_in, index, var, value.as_bool()),
            (VarKind::PhysOut, _) => write_bit(&mut self.phys_out, index, var, value.as_bool()),
            (VarKind::PhysInWord, None) => {
                write_word(&mut self.phys_in_words, index, var, value.as_i64())
            }
            (VarKind::PhysOutWord, None) => {
                write_word(&mut self.phys_out_words, index, var, value.as_i64())
            }
            (VarKind::MemWord, Some(Accessor::Bit(bit)))
            | (VarKind::PhysInWord, Some(Accessor::Bit(bit)))
            | (VarKind::PhysOutWord, Some(Accessor::Bit(bit))) => {
                self.set_word_bit(var, index, bit, value.as_bool())
            }
            (VarKind::TimerIec, Some(Accessor::Done)) => {
                let Some(timer) = timer_slot(&mut self.timers, index, var)? else {
                    return Ok(());
                };
                timer.done = value.as_bool();
                Ok(())
            }
            (VarKind::TimerIec, Some(Accessor::Value)) => {
                let Some(timer) = timer_slot(&mut self.timers, index, var)? else {
                    return Ok(());
                };
                timer.elapsed = clamp_i64_to_u64(value.as_i64());
                Ok(())
            }
            (VarKind::TimerIec, Some(Accessor::Preset)) => {
                let Some(timer) = timer_slot(&mut self.timers, index, var)? else {
                    return Ok(());
                };
                timer.set_preset(clamp_i64_to_u64(value.as_i64()));
                Ok(())
            }
            (VarKind::Counter, Some(Accessor::Done)) => {
                let Some(counter) = counter_slot(&mut self.counters, index, var)? else {
                    return Ok(());
                };
                counter.set_done(value.as_bool());
                Ok(())
            }
            (VarKind::Counter, Some(Accessor::Value)) => {
                let Some(counter) = counter_slot(&mut self.counters, index, var)? else {
                    return Ok(());
                };
                counter.value = clamp_i32(value.as_i64()).clamp(0, COUNTER_MAX);
                Ok(())
            }
            (VarKind::Counter, Some(Accessor::Preset)) => {
                let Some(counter) = counter_slot(&mut self.counters, index, var)? else {
                    return Ok(());
                };
                counter.preset = clamp_i32(value.as_i64()).clamp(0, COUNTER_MAX);
                Ok(())
            }
            (VarKind::Counter, Some(Accessor::Empty)) => {
                let Some(counter) = counter_slot(&mut self.counters, index, var)? else {
                    return Ok(());
                };
                counter.empty = value.as_bool();
                Ok(())
            }
            (VarKind::Counter, Some(Accessor::Full)) => {
                let Some(counter) = counter_slot(&mut self.counters, index, var)? else {
                    return Ok(());
                };
                counter.full = value.as_bool();
                Ok(())
            }
            (VarKind::Register, Some(Accessor::In)) => {
                let Some(register) = register_slot(&mut self.registers, index, var)? else {
                    return Ok(());
                };
                register.in_value = clamp_i32(value.as_i64());
                Ok(())
            }
            (VarKind::Register, Some(Accessor::Out)) => {
                let Some(register) = register_slot(&mut self.registers, index, var)? else {
                    return Ok(());
                };
                register.out_value = clamp_i32(value.as_i64());
                Ok(())
            }
            (VarKind::Step, Some(Accessor::Activity)) | (VarKind::Step, None) => {
                write_bit(&mut self.steps, index, var, value.as_bool())
            }
            (VarKind::Step, Some(Accessor::Value)) => {
                write_u64(&mut self.step_ages, index, var, value.as_i64())
            }
            (VarKind::System, _) => write_bit(&mut self.system, index, var, value.as_bool()),
            (VarKind::Led, _) => write_bit(&mut self.leds, index, var, value.as_bool()),
            _ => Err(StoreError::NotWritable(var.clone())),
        }
    }

    /// Mutable access to a timer block by index.
    pub fn timer_mut(&mut self, index: usize) -> Option<&mut TimerIec> {
        self.timers.get_mut(index)
    }

    /// Mutable access to a counter block by index.
    pub fn counter_mut(&mut self, index: usize) -> Option<&mut Counter> {
        self.counters.get_mut(index)
    }

    /// Mutable access to a register block by index.
    pub fn register_mut(&mut self, index: usize) -> Option<&mut RegisterState> {
        self.registers.get_mut(index)
    }

    /// Number of physical digital channels; `%I` and `%Q` share the numbering.
    pub fn digital_channels(&self) -> usize {
        self.phys_in.len().max(self.phys_out.len())
    }

    /// Number of physical analog channels; `%IW` and `%QW` share the numbering.
    pub fn analog_channels(&self) -> usize {
        self.phys_in_words.len().max(self.phys_out_words.len())
    }

    /// Grows the store up to the index of `var` when needed and returns the
    /// resolved index.
    ///
    /// This is what lets a project address `%Q7` or `%TM12` even though the
    /// store was created empty.
    pub fn ensure(&mut self, var: &VarRef) -> Result<usize, StoreError> {
        let index = self
            .resolve_index(var)
            .ok_or_else(|| self.bad_index(var, "the index cannot be resolved"))?;
        if index >= MAX_GROWTH {
            return Err(StoreError::OutOfRange(var.clone()));
        }
        let needed = index + 1;
        match var.kind {
            VarKind::MemBit => grow(&mut self.mem_bits, needed, false),
            VarKind::MemWord => grow(&mut self.mem_words, needed, 0),
            VarKind::PhysIn => grow(&mut self.phys_in, needed, false),
            VarKind::PhysOut => grow(&mut self.phys_out, needed, false),
            VarKind::PhysInWord => grow(&mut self.phys_in_words, needed, 0),
            VarKind::PhysOutWord => grow(&mut self.phys_out_words, needed, 0),
            VarKind::TimerIec => grow(&mut self.timers, needed, TimerIec::default()),
            VarKind::Counter => grow(&mut self.counters, needed, Counter::default()),
            VarKind::Register => grow(&mut self.registers, needed, RegisterState::default()),
            VarKind::Step => {
                grow(&mut self.steps, needed, false);
                grow(&mut self.step_ages, needed, 0);
            }
            VarKind::System => grow(&mut self.system, needed, false),
            VarKind::Led => grow(&mut self.leds, needed, false),
        }
        Ok(index)
    }

    /// Writes one bit of a word variable, leaving the other bits alone.
    fn set_word_bit(
        &mut self,
        var: &VarRef,
        index: usize,
        bit: u8,
        value: bool,
    ) -> Result<(), StoreError> {
        if index >= MAX_GROWTH {
            return Err(StoreError::OutOfRange(var.clone()));
        }
        match var.kind {
            VarKind::MemWord => write_bit_in(&mut self.mem_words, index, bit, value, var),
            VarKind::PhysInWord => write_bit_in(&mut self.phys_in_words, index, bit, value, var),
            VarKind::PhysOutWord => write_bit_in(&mut self.phys_out_words, index, bit, value, var),
            _ => Err(StoreError::NotWritable(var.clone())),
        }
    }

    /// Resolves the effective index of `var`, following `index_expr`.
    fn resolve_index(&self, var: &VarRef) -> Option<usize> {
        let index = match &var.index_expr {
            None => var.index,
            Some(index_var) => {
                let value = self.get(index_var)?.as_i64();
                if value < 0 || value > MAX_GROWTH as i64 {
                    return None;
                }
                value as u32
            }
        };
        usize::try_from(index).ok()
    }

    /// Builds the [`StoreError::BadIndex`] for `var`.
    fn bad_index(&self, var: &VarRef, reason: &str) -> StoreError {
        StoreError::BadIndex(var.clone(), reason.to_owned())
    }
}

impl VarSource for VarStore {
    fn get(&self, var: &VarRef) -> Option<Value> {
        VarStore::get(self, var)
    }
}

/// Reads bit `bit` of `word`; bits above 31 read as zero.
fn word_bit(word: i32, bit: u8) -> bool {
    if usize::from(bit) >= 32 {
        return false;
    }
    (word >> bit) & 1 != 0
}

/// Returns `word` with bit `bit` set to `value`; bits above 31 are ignored.
fn write_word_bit(word: i32, bit: u8, value: bool) -> i32 {
    if usize::from(bit) >= 32 {
        return word;
    }
    let mask = 1_i32 << bit;
    if value {
        word | mask
    } else {
        word & !mask
    }
}

/// Grows `slot` to at least `needed` entries, filling with `filler`.
fn grow<T: Clone>(slot: &mut Vec<T>, needed: usize, filler: T) {
    if slot.len() < needed {
        slot.resize(needed, filler);
    }
}

/// Returns a mutable timer slot, growing the vector up to [`MAX_GROWTH`].
fn timer_slot<'a>(
    slots: &'a mut Vec<TimerIec>,
    index: usize,
    var: &VarRef,
) -> Result<Option<&'a mut TimerIec>, StoreError> {
    if index >= MAX_GROWTH {
        return Err(StoreError::OutOfRange(var.clone()));
    }
    grow(slots, index + 1, TimerIec::default());
    Ok(slots.get_mut(index))
}

/// Returns a mutable counter slot, growing the vector up to [`MAX_GROWTH`].
fn counter_slot<'a>(
    slots: &'a mut Vec<Counter>,
    index: usize,
    var: &VarRef,
) -> Result<Option<&'a mut Counter>, StoreError> {
    if index >= MAX_GROWTH {
        return Err(StoreError::OutOfRange(var.clone()));
    }
    grow(slots, index + 1, Counter::default());
    Ok(slots.get_mut(index))
}

/// Returns a mutable register slot, growing the vector up to [`MAX_GROWTH`].
fn register_slot<'a>(
    slots: &'a mut Vec<RegisterState>,
    index: usize,
    var: &VarRef,
) -> Result<Option<&'a mut RegisterState>, StoreError> {
    if index >= MAX_GROWTH {
        return Err(StoreError::OutOfRange(var.clone()));
    }
    grow(slots, index + 1, RegisterState::default());
    Ok(slots.get_mut(index))
}

/// Writes a bit into `slot`, growing it up to [`MAX_GROWTH`] entries.
fn write_bit(
    slot: &mut Vec<bool>,
    index: usize,
    var: &VarRef,
    value: bool,
) -> Result<(), StoreError> {
    if index >= MAX_GROWTH {
        return Err(StoreError::OutOfRange(var.clone()));
    }
    grow(slot, index + 1, false);
    match slot.get_mut(index) {
        Some(cell) => {
            *cell = value;
            Ok(())
        }
        None => Err(StoreError::OutOfRange(var.clone())),
    }
}

/// Writes a word into `slot`, growing it up to [`MAX_GROWTH`] entries.
fn write_word(
    slot: &mut Vec<i32>,
    index: usize,
    var: &VarRef,
    value: i64,
) -> Result<(), StoreError> {
    if index >= MAX_GROWTH {
        return Err(StoreError::OutOfRange(var.clone()));
    }
    grow(slot, index + 1, 0);
    match slot.get_mut(index) {
        Some(cell) => {
            *cell = clamp_i32(value);
            Ok(())
        }
        None => Err(StoreError::OutOfRange(var.clone())),
    }
}

/// Writes one bit of the word at `index`, growing the vector on demand.
fn write_bit_in(
    slots: &mut Vec<i32>,
    index: usize,
    bit: u8,
    value: bool,
    var: &VarRef,
) -> Result<(), StoreError> {
    if index >= MAX_GROWTH {
        return Err(StoreError::OutOfRange(var.clone()));
    }
    grow(slots, index + 1, 0);
    match slots.get_mut(index) {
        Some(word) => {
            *word = write_word_bit(*word, bit, value);
            Ok(())
        }
        None => Err(StoreError::OutOfRange(var.clone())),
    }
}

/// Writes a millisecond age into `slot`, growing it on demand.
fn write_u64(
    slot: &mut Vec<u64>,
    index: usize,
    var: &VarRef,
    value: i64,
) -> Result<(), StoreError> {
    if index >= MAX_GROWTH {
        return Err(StoreError::OutOfRange(var.clone()));
    }
    grow(slot, index + 1, 0);
    match slot.get_mut(index) {
        Some(cell) => {
            *cell = clamp_i64_to_u64(value);
            Ok(())
        }
        None => Err(StoreError::OutOfRange(var.clone())),
    }
}

/// Clamps an `i64` into the `i32` range.
fn clamp_i32(value: i64) -> i32 {
    value.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

/// Clamps an `i64` into the `u64` range.
fn clamp_i64_to_u64(value: i64) -> u64 {
    value.max(0) as u64
}

/// Clamps a `u64` into the `i32` range so that it can be read as a word.
fn clamp_u64_to_i32(value: u64) -> i32 {
    value.min(i32::MAX as u64) as i32
}

/// Result of a single scan.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ScanReport {
    /// Number of completed scans, including this one.
    pub cycles: u64,
    /// Diagnostics produced while evaluating the sections.
    pub diagnostics: Vec<Diagnostic>,
    /// `true` when the mad-loop guard aborted the scan.
    pub stopped_for_mad_loop: bool,
    /// Number of `CoilJump` blocks that were taken.
    pub jumps: u64,
    /// Number of `CoilCall` blocks that were executed.
    pub calls: u64,
}

/// The deterministic ladder-logic interpreter.
#[derive(Debug, Clone)]
pub struct ScanEngine {
    project: Project,
    store: VarStore,
    /// Precomputed cell index, rebuilt whenever the project shape changes.
    cache: RungCache,
    /// Previous bit value of every element, used by edge-sensing elements.
    edge_prev: Vec<bool>,
    /// Timestamp of the previous scan, or [`None`] before the first one.
    last_scan_ms: Option<u64>,
    /// Completed scans.
    cycles: u64,
    /// Public edge helper for callers that detect their own edges.
    edge_bank: EdgeBank,
}

impl ScanEngine {
    /// Creates an engine with a default-sized store and indexes `project`.
    pub fn new(project: Project) -> Self {
        Self::with_store(project, VarStore::with_default_sizes())
    }

    /// Creates an engine backed by an existing store.
    pub fn with_store(project: Project, store: VarStore) -> Self {
        let mut engine = Self {
            project,
            store,
            cache: RungCache::default(),
            edge_prev: Vec::new(),
            last_scan_ms: None,
            cycles: 0,
            edge_bank: EdgeBank::new(),
        };
        engine.refresh();
        engine
    }

    /// The project being executed.
    pub fn project(&self) -> &Project {
        &self.project
    }

    /// The project being executed, mutably.
    ///
    /// The cell index is rebuilt automatically on the next scan, so a caller may
    /// edit the rungs between scans.
    pub fn project_mut(&mut self) -> &mut Project {
        &mut self.project
    }

    /// The variable store.
    pub fn store(&self) -> &VarStore {
        &self.store
    }

    /// The variable store, mutably.
    pub fn store_mut(&mut self) -> &mut VarStore {
        &mut self.store
    }

    /// Number of completed scans.
    pub fn cycles(&self) -> u64 {
        self.cycles
    }

    /// The previous-state helper, for callers that detect their own edges.
    pub fn edge_bank(&self) -> &EdgeBank {
        &self.edge_bank
    }

    /// Rebuilds the cell index, the subroutine table and the edge history.
    ///
    /// Called by [`ScanEngine::new`] and automatically whenever the project
    /// shape changes; a caller that edits the project through
    /// [`ScanEngine::project_mut`] does not have to call it explicitly.
    pub fn refresh(&mut self) {
        self.cache = RungCache::build(&self.project);
        self.edge_prev = vec![false; self.cache.element_count];
        self.ensure_used_variables();
    }

    /// Grows the store so that every variable the project uses exists.
    fn ensure_used_variables(&mut self) {
        for rung in &self.project.rungs {
            for element in &rung.elements {
                if let Some(var) = element.var.clone() {
                    let _ = self.store.ensure(&var);
                }
            }
        }
    }

    /// Runs at most one scan at simulated time `now_ms` and returns a report.
    ///
    /// `delta_ms` is `now_ms - last_scan_ms`: zero on the first scan, clamped to
    /// [`MAX_DELTA_MS`]. The engine never reads the clock itself.
    pub fn scan_once(&mut self, now_ms: u64) -> ScanReport {
        if self.cache.shape != shape_of(&self.project) {
            self.refresh();
        }
        if self.edge_prev.len() != self.cache.element_count {
            self.edge_prev = vec![false; self.cache.element_count];
        }

        let delta_ms = match self.last_scan_ms {
            Some(previous) => now_ms.saturating_sub(previous).min(MAX_DELTA_MS),
            None => 0,
        };
        self.last_scan_ms = Some(now_ms);
        self.cycles = self.cycles.saturating_add(1);

        let ScanEngine {
            project,
            store,
            cache,
            edge_prev,
            ..
        } = self;

        let mut machine = Machine {
            store,
            cache,
            edge_prev,
            output_prev: Vec::new(),
            output_next: Vec::new(),
            diagnostics: Vec::new(),
            delta_ms,
            frame_abandoned: false,
            period_ms: u64::from(project.scan.period_ms),
            section_index: 0,
            rung_index: 0,
            jumps: 0,
            calls: 0,
            mad_loop: false,
            call_depth: 0,
        };

        for index in 0..project.sections.len() {
            let Some(section) = project.sections.get(index) else {
                continue;
            };
            match section.language {
                SectionLanguage::Ladder => {
                    if section.subroutine.is_none() && !machine.mad_loop {
                        machine.run_section(project, index);
                    }
                    // A failed call may have abandoned the section; each section
                    // starts from a clean slate.
                    machine.frame_abandoned = false;
                }
                SectionLanguage::Sfc => {
                    machine.section_index = index;
                    machine.rung_index = 0;
                    machine.error(
                        "SL-W002",
                        format!("SFC section `{}` is skipped until M4", section.name),
                    );
                }
            }
        }

        ScanReport {
            cycles: self.cycles,
            diagnostics: machine.diagnostics,
            stopped_for_mad_loop: machine.mad_loop,
            jumps: machine.jumps,
            calls: machine.calls,
        }
    }
}

/// Mutable state threaded through a scan.
///
/// The borrow of the [`Project`] is kept separate from this structure so that a
/// section may be read while the store is written, and so that subroutines can
/// re-enter `run_section` without cloning the project.
struct Machine<'a> {
    store: &'a mut VarStore,
    cache: &'a RungCache,
    edge_prev: &'a mut Vec<bool>,
    /// Outputs of the column being evaluated.
    output_next: Vec<bool>,
    /// Outputs of the previous column, which `state_on_left` reads.
    output_prev: Vec<bool>,
    diagnostics: Vec<Diagnostic>,
    /// Milliseconds since the previous scan, already clamped.
    delta_ms: u64,
    /// `true` when the current section must abandon its remaining rungs because
    /// a subroutine call failed inside it.
    frame_abandoned: bool,
    /// Configured scan period of the project, in milliseconds.
    period_ms: u64,
    section_index: usize,
    rung_index: usize,
    jumps: u64,
    calls: u64,
    mad_loop: bool,
    call_depth: usize,
}

/// What evaluating a rung did to the surrounding section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RungOutcome {
    /// The rung ran to its right-hand end; the next rung follows.
    Next,
    /// A jump moved the program counter; the next rung is `index`.
    JumpTo(usize),
    /// The mad-loop guard tripped; the whole scan must stop.
    StopScan,
    /// A failed call abandoned the rest of the rung; the next rung follows.
    AbandonRung,
}

/// How a call block ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CallOutcome {
    /// The subroutine ran to completion.
    Done,
    /// The rung must be abandoned (stack overflow or mad loop).
    Abandon,
}

impl Machine<'_> {
    /// Records a diagnostic carrying the current section and rung.
    fn report(&mut self, severity: Severity, code: &'static str, message: String) {
        self.diagnostics.push(
            Diagnostic::new(severity, code, message)
                .with_section(self.section_index)
                .with_rung(self.rung_index),
        );
    }

    /// Records an error diagnostic.
    fn error(&mut self, code: &'static str, message: String) {
        self.report(Severity::Error, code, message);
    }

    /// Runs one section (or subroutine).
    ///
    /// Returns `true` when the whole scan must stop because of the mad-loop
    /// guard.
    fn run_section(&mut self, project: &Project, section_index: usize) -> bool {
        let Some(section) = project.sections.get(section_index) else {
            return false;
        };
        let rung_count = section.rungs.len();
        let mut position = 0usize;
        while position < rung_count && !self.mad_loop {
            self.rung_index = position;
            match self.run_rung(project, section, position) {
                RungOutcome::Next => position += 1,
                RungOutcome::AbandonRung => {
                    // A failed call abandons the rest of the rung and marks the
                    // frame, so the caller abandons its own rung in turn.
                    self.frame_abandoned = true;
                    break;
                }
                RungOutcome::JumpTo(target) => position = target,
                RungOutcome::StopScan => return true,
            }
        }
        self.mad_loop
    }

    /// Runs the rung at `position` inside `section`.
    fn run_rung(&mut self, project: &Project, section: &Section, position: usize) -> RungOutcome {
        let Some(rung_id) = section.rungs.get(position).copied() else {
            return RungOutcome::Next;
        };
        let Some(rung) = project.rung(rung_id) else {
            return RungOutcome::Next;
        };
        // The cell index follows the order of `Project::rungs`, while a section
        // lists its rungs by id and may reference them in any order.
        let Some(grid_index) = self.cache.rung_index.get(&rung_id).copied() else {
            return RungOutcome::Next;
        };
        let Some(grid) = self.cache.grids.get(grid_index) else {
            return RungOutcome::Next;
        };

        let rows = self.cache.max_rows;
        reset(&mut self.output_prev, rows, false);
        reset(&mut self.output_next, rows, false);

        let mut column = 0usize;
        while column < grid.columns {
            std::mem::swap(&mut self.output_prev, &mut self.output_next);
            // Horizontal flow is implicit: an empty cell in a *live* row (a row
            // that holds at least one element) conducts. It carries exactly what
            // a `Connection` cell would carry, which is `state_on_left` — the OR
            // of the previous column over the vertical block at this column. That
            // matters wherever a row is joined to its neighbour: the shared power
            // must reach the empty cells of the merge column too, not just the
            // cells that hold an element. A row with no elements at all is inert:
            // it breaks the flow and can never inject power into a vertical link,
            // and an empty cell in column 0 is *not* the rail (a branch that taps
            // in mid-rung must not be fed from the rail — that is what `SL-W001`
            // warns about).
            for row in 0..rows {
                let value = if grid.live_row(row) {
                    self.state_on_left(grid, column, row)
                } else {
                    false
                };
                if let Some(cell) = self.output_next.get_mut(row) {
                    *cell = value;
                }
            }
            let mut row = 0usize;
            while row < rows {
                let cell = grid.cells.get(column).and_then(|column| column.get(row));
                let Some(Some(id)) = cell else {
                    row += 1;
                    continue;
                };
                if let Some(block) = grid.blocks.get(row).and_then(Option::as_ref) {
                    if block.id == *id {
                        self.evaluate_block(rung, grid, block);
                        row = row.saturating_add(usize::from(block.span).max(1));
                        continue;
                    }
                }
                let Some(local) = grid.local_index(*id) else {
                    row += 1;
                    continue;
                };
                let Some(element) = rung.elements.get(local) else {
                    row += 1;
                    continue;
                };
                let input = self.state_on_left(grid, column, row);
                let output = self.evaluate_element(element, *id, input);
                if let Some(cell) = self.output_next.get_mut(row) {
                    *cell = output;
                }
                if input {
                    match element.kind {
                        ElementKind::CoilJump => {
                            if let Some(outcome) = self.take_jump(project, section, element) {
                                return outcome;
                            }
                        }
                        ElementKind::CoilCall => match self.take_call(project, section, element) {
                            CallOutcome::Done => {}
                            CallOutcome::Abandon => return RungOutcome::AbandonRung,
                        },
                        _ => {}
                    }
                }
                row += 1;
            }
            column += 1;
        }
        RungOutcome::Next
    }

    /// Resolves and takes a jump coil, or reports why it could not be taken.
    fn take_jump(
        &mut self,
        project: &Project,
        section: &Section,
        element: &PlacedElement,
    ) -> Option<RungOutcome> {
        let Some(parameter) = element.params.first() else {
            self.error("SL-E004", "jump coil has no target".to_owned());
            return None;
        };
        match resolve_jump(project, section, parameter) {
            Some(target) => {
                self.jumps = self.jumps.saturating_add(1);
                if self.jumps > MAD_LOOP_LIMIT {
                    self.error(
                        "SL-E006",
                        format!(
                            "more than {MAD_LOOP_LIMIT} jumps in one scan; the section is abandoned"
                        ),
                    );
                    self.mad_loop = true;
                    return Some(RungOutcome::StopScan);
                }
                Some(RungOutcome::JumpTo(target))
            }
            None => {
                self.error(
                    "SL-E005",
                    format!("jump target `{parameter}` does not exist"),
                );
                None
            }
        }
    }

    /// Executes a subroutine requested by a call coil.
    fn take_call(
        &mut self,
        project: &Project,
        section: &Section,
        element: &PlacedElement,
    ) -> CallOutcome {
        let Some(parameter) = element.params.first() else {
            self.error("SL-E004", "call coil has no subroutine number".to_owned());
            return CallOutcome::Done;
        };
        let target = parameter.trim();
        let number = match target.parse::<u32>() {
            Ok(number) => number,
            Err(_) => {
                self.error(
                    "SL-E007",
                    format!("call target `{target}` is not a subroutine number"),
                );
                return CallOutcome::Done;
            }
        };
        let Some(index) = self.cache.subroutines.get(&number).copied() else {
            self.error(
                "SL-E007",
                format!(
                    "section {} calls undefined subroutine {number}",
                    section.name
                ),
            );
            return CallOutcome::Done;
        };
        if self.call_depth >= MAX_CALL_DEPTH {
            self.error(
                "SL-E008",
                format!("subroutine {number} nests deeper than {MAX_CALL_DEPTH} frames"),
            );
            return CallOutcome::Abandon;
        }
        self.calls = self.calls.saturating_add(1);
        self.call_depth += 1;
        let previous_rung = self.rung_index;
        let previous_section = self.section_index;
        self.section_index = index;
        let stopped = self.run_section(project, index);
        self.section_index = previous_section;
        self.rung_index = previous_rung;
        self.call_depth = self.call_depth.saturating_sub(1);
        // `frame_abandoned` is set by the subroutine (or by an earlier failed
        // call inside it) and propagates up one frame at a time, because each
        // caller sees it and abandons its own rung in turn.
        if stopped || self.frame_abandoned {
            CallOutcome::Abandon
        } else {
            CallOutcome::Done
        }
    }

    /// Evaluates a function block occupying several rows of one column.
    fn evaluate_block(&mut self, rung: &Rung, grid: &RungGrid, block: &BlockCell) {
        let Some(local) = grid.local_index(block.id) else {
            return;
        };
        let Some(element) = rung.elements.get(local) else {
            return;
        };
        // A block is placed on its first row, so this is also where a missing or
        // mismatched block variable has to be reported: the other rows of the
        // block carry no element of their own.
        match element.var.as_ref() {
            None => {
                self.error(
                    "SL-E004",
                    format!("{:?} block has no variable", element.kind),
                );
                return;
            }
            Some(var) => {
                let expected = match element.kind {
                    ElementKind::Timer { .. } => VarKind::TimerIec,
                    ElementKind::Counter { .. } => VarKind::Counter,
                    ElementKind::Register { .. } => VarKind::Register,
                    _ => var.kind,
                };
                if var.kind != expected {
                    self.error(
                        "SL-E003",
                        format!(
                            "{:?} block needs a `{}` variable, found `{var}`",
                            element.kind,
                            expected.mnemonic()
                        ),
                    );
                    return;
                }
            }
        }
        let mut inputs = [false; 4];
        for (offset, input) in inputs.iter_mut().enumerate().take(usize::from(block.span)) {
            *input = self.state_on_left_grid(block, block.row as usize + offset);
        }

        let output = match element.kind {
            ElementKind::Timer { mode } => self.update_timer(element, block, mode, inputs[0]),
            ElementKind::Counter { kind } => self.update_counter(element, block, kind, inputs),
            ElementKind::Register { mode } => self.update_register(element, block, mode, inputs),
            _ => false,
        };

        if let Some(cell) = self.output_next.get_mut(block.row as usize) {
            *cell = output;
        }
    }

    /// Runs one scan of a timer block.
    fn update_timer(
        &mut self,
        element: &PlacedElement,
        block: &BlockCell,
        mode: TimerMode,
        enable: bool,
    ) -> bool {
        let base = self.block_time_base(element);
        let preset = self.block_preset_units(element, base);
        let delta_ms = self.delta_ms;
        match self.store.timer_mut(block.index) {
            Some(timer) => {
                timer.mode = mode;
                timer.configure(preset, base);
                timer.update(delta_ms, enable)
            }
            None => {
                self.error(
                    "SL-E003",
                    format!("timer {} is out of range", element_text(element)),
                );
                false
            }
        }
    }

    /// Runs one scan of a counter block.
    fn update_counter(
        &mut self,
        element: &PlacedElement,
        block: &BlockCell,
        kind: CounterKind,
        inputs: [bool; 4],
    ) -> bool {
        let preset = self.block_preset(element);
        let [reset, load, up, down] = inputs;
        match self.store.counter_mut(block.index) {
            Some(counter) => {
                counter.kind = kind;
                counter.preset = clamp_i32(preset as i64).clamp(0, COUNTER_MAX);
                counter.emulate(up, down, reset, load);
                counter.done()
            }
            None => {
                self.error(
                    "SL-E003",
                    format!("counter {} is out of range", element_text(element)),
                );
                false
            }
        }
    }

    /// Runs one scan of a register block.
    fn update_register(
        &mut self,
        element: &PlacedElement,
        block: &BlockCell,
        mode: RegisterMode,
        inputs: [bool; 4],
    ) -> bool {
        let capacity = self.block_capacity(element);
        let [reset, push, pop, _] = inputs;
        match self.store.register_mut(block.index) {
            Some(register) => {
                register.mode = mode;
                register.set_capacity(capacity);
                register.update(reset, push, pop);
                register.is_empty()
            }
            None => {
                self.error(
                    "SL-E003",
                    format!("register {} is out of range", element_text(element)),
                );
                true
            }
        }
    }

    /// Reads the preset of a block in time-base units.
    ///
    /// A literal parameter is a duration in milliseconds, so `300` on a timer
    /// with the default 100 ms base is three units. A parameter that names a
    /// variable is read through the store and is used as it stands, which lets a
    /// project hold presets in whatever unit it prefers.
    fn block_preset_units(&mut self, element: &PlacedElement, base: TimeBase) -> u64 {
        let Some(parameter) = element.params.first().map(|text| text.trim().to_owned()) else {
            return 0;
        };
        if parameter.is_empty() {
            return 0;
        }
        // A suffixed literal is a duration in the suffixed unit, so `3s` is
        // 3000 ms before it is converted back into time-base units.
        let millis = match parse_suffixed_millis(&parameter) {
            Some(value) => value.max(0) as u64,
            None => match self.parameter_value(&parameter, element) {
                Some(value) => value.max(0) as u64,
                None => 0,
            },
        };
        let units = millis / base.millis();
        if units == 0 && millis > 0 {
            1
        } else {
            units
        }
    }

    /// Reads the preset parameter of a block as a plain integer.
    ///
    /// This is the raw parameter value, used by counters (whose presets are
    /// counts rather than durations) and by the register capacity.
    fn block_preset(&mut self, element: &PlacedElement) -> u64 {
        let Some(parameter) = element
            .params
            .first()
            .map(|parameter| parameter.trim().to_owned())
        else {
            return 0;
        };
        if parameter.is_empty() {
            return 0;
        }
        if let Some(value) = parse_suffixed_number(&parameter) {
            return value.max(0) as u64;
        }
        match self.parameter_value(&parameter, element) {
            Some(value) => value.max(0) as u64,
            None => 0,
        }
    }

    /// Derives the timer time base from the preset parameter.
    ///
    /// A plain decimal literal (or a literal with an `s`/`m` suffix) selects the
    /// unit directly, so that `100` means ten seconds at the default base; an
    /// unsuffixed variable is a duration in milliseconds and picks the largest
    /// base that divides it exactly.
    fn block_time_base(&mut self, element: &PlacedElement) -> TimeBase {
        let Some(parameter) = element.params.first() else {
            return TimeBase::Millis100;
        };
        let parameter = parameter.trim();
        if let Some(suffix) = parameter.chars().last() {
            if (suffix == 's' || suffix == 'S') && parse_suffixed_number(parameter).is_some() {
                return TimeBase::Second;
            }
            if (suffix == 'm' || suffix == 'M') && parse_suffixed_number(parameter).is_some() {
                return TimeBase::Minute60;
            }
        }
        if parameter.parse::<i64>().is_ok() {
            return TimeBase::Millis100;
        }
        let millis = self.parameter_value(parameter, element).unwrap_or(0);
        select_time_base(millis, self.period_ms)
    }

    /// Reads the capacity parameter of a register block.
    ///
    /// A missing capacity keeps the default of [`DEFAULT_REGISTER_CAPACITY`].
    fn block_capacity(&mut self, element: &PlacedElement) -> usize {
        let Some(parameter) = element
            .params
            .first()
            .map(|parameter| parameter.trim().to_owned())
        else {
            return DEFAULT_REGISTER_CAPACITY;
        };
        if parameter.is_empty() {
            return DEFAULT_REGISTER_CAPACITY;
        }
        match self.parameter_value(&parameter, element) {
            Some(value) => value.clamp(0, MAX_GROWTH as i64) as usize,
            None => DEFAULT_REGISTER_CAPACITY,
        }
    }

    /// Reads a block parameter once: a literal, or a variable.
    fn parameter_value(&mut self, parameter: &str, element: &PlacedElement) -> Option<i64> {
        let parameter = parameter.trim();
        if parameter.is_empty() {
            return None;
        }
        if let Ok(literal) = parameter.parse::<i64>() {
            return Some(literal);
        }
        let var = match parameter.parse::<VarRef>() {
            Ok(var) => var,
            Err(error) => {
                self.error(
                    "SL-E002",
                    format!(
                        "cannot read the block parameter `{parameter}` of `{}`: {error}",
                        element_text(element)
                    ),
                );
                return None;
            }
        };
        match self.store.get(&var) {
            Some(value) => Some(value.as_i64()),
            None => {
                self.error(
                    "SL-E001",
                    format!(
                        "block parameter `{var}` of `{}` is unknown",
                        element_text(element)
                    ),
                );
                None
            }
        }
    }

    /// Evaluates one single-cell element and returns its output.
    fn evaluate_element(&mut self, element: &PlacedElement, id: u32, input: bool) -> bool {
        match element.kind {
            ElementKind::ContactNo
            | ElementKind::ContactNc
            | ElementKind::ContactRising
            | ElementKind::ContactFalling => self.evaluate_contact(element, id, input),
            ElementKind::Connection => input,
            ElementKind::CoilOut
            | ElementKind::CoilOutNeg
            | ElementKind::CoilSet
            | ElementKind::CoilReset => {
                self.evaluate_coil(element, input);
                input
            }
            ElementKind::CoilJump | ElementKind::CoilCall => input,
            ElementKind::Compare => self.evaluate_compare(element, input),
            ElementKind::Operate => {
                self.evaluate_operate(element, input);
                input
            }
            // Blocks are evaluated as a unit by `evaluate_block`; reaching here
            // means the block table and the cell index disagree, which cannot
            // happen through `refresh`.
            ElementKind::Timer { .. }
            | ElementKind::Counter { .. }
            | ElementKind::Register { .. } => input,
        }
    }

    /// Evaluates one contact, consuming its edge when it is edge sensing.
    fn evaluate_contact(&mut self, element: &PlacedElement, id: u32, input: bool) -> bool {
        let Some(var) = element.var.as_ref() else {
            self.error(
                "SL-E004",
                format!("{:?} element has no variable", element.kind),
            );
            return false;
        };
        let bit = self.read_bit(var, element);
        let state = match element.kind {
            ElementKind::ContactNo => bit,
            ElementKind::ContactNc => !bit,
            ElementKind::ContactRising => {
                let previous = self.edge_prev.get(id as usize).copied().unwrap_or(false);
                set_edge(self.edge_prev, id as usize, bit);
                bit && !previous
            }
            ElementKind::ContactFalling => {
                let previous = self.edge_prev.get(id as usize).copied().unwrap_or(false);
                set_edge(self.edge_prev, id as usize, bit);
                !bit && previous
            }
            _ => bit,
        };
        state && input
    }

    /// Reads the bit value of an element's variable, reporting failures.
    ///
    /// A word variable is a kind mismatch, not an unknown variable: the two
    /// cases carry different diagnostic codes and are checked separately.
    fn read_bit(&mut self, var: &VarRef, element: &PlacedElement) -> bool {
        if !var.is_bit() {
            self.error(
                "SL-E003",
                format!(
                    "`{var}` is a word but the {:?} element needs a bit",
                    element.kind
                ),
            );
            return false;
        }
        match self.store.get(var) {
            Some(value) => value.as_bool(),
            None => {
                self.error(
                    "SL-E001",
                    format!(
                        "variable `{var}` of the {:?} element is unknown",
                        element.kind
                    ),
                );
                false
            }
        }
    }

    /// Applies an output coil.
    fn evaluate_coil(&mut self, element: &PlacedElement, input: bool) {
        let Some(var) = element.var.as_ref() else {
            self.error(
                "SL-E004",
                format!("{:?} coil has no variable", element.kind),
            );
            return;
        };
        if !var.is_bit() {
            self.error(
                "SL-E003",
                format!(
                    "`{var}` is a word but the {:?} coil needs a bit",
                    element.kind
                ),
            );
            return;
        }
        let value = match element.kind {
            ElementKind::CoilOut => Some(input),
            ElementKind::CoilOutNeg => Some(!input),
            ElementKind::CoilSet => input.then_some(true),
            ElementKind::CoilReset => input.then_some(false),
            _ => None,
        };
        if let Some(value) = value {
            if let Err(error) = self.store.set(var, Value::Bit(value)) {
                self.error("SL-E003", format!("cannot drive `{var}`: {error}"));
            }
        }
    }

    /// Evaluates a `Compare` block: `output = state && input`.
    fn evaluate_compare(&mut self, element: &PlacedElement, input: bool) -> bool {
        let expression = match element.params.len() {
            0 => {
                self.error("SL-E002", "compare block has no expression".to_owned());
                return false;
            }
            1 => element.params.first().cloned().unwrap_or_default(),
            3 => element
                .params
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(" "),
            count => {
                self.error(
                    "SL-E002",
                    format!("compare block has {count} parameters; expected 1 or 3"),
                );
                return false;
            }
        };
        match parse(&expression).and_then(|expr| eval(&expr, self.store)) {
            Ok(value) => value.as_bool() && input,
            Err(error) => {
                self.error(
                    "SL-E002",
                    format!("cannot evaluate `{expression}`: {error}"),
                );
                false
            }
        }
    }

    /// Evaluates an `Operate` block: `if input { write(target, expr) }`.
    fn evaluate_operate(&mut self, element: &PlacedElement, input: bool) {
        if !input {
            return;
        }
        let Some(target) = element.params.first() else {
            self.error("SL-E004", "operate block has no target".to_owned());
            return;
        };
        if element.params.len() < 2 {
            self.error(
                "SL-E002",
                format!("operate block `{target}` has no expression"),
            );
            return;
        }
        let expression = element
            .params
            .iter()
            .skip(1)
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(" ");
        let expression = expression.trim();
        let expression = expression.strip_prefix('=').unwrap_or(expression).trim();
        let target = target.trim();
        let var = match target.parse::<VarRef>() {
            Ok(var) => var,
            Err(error) => {
                self.error(
                    "SL-E003",
                    format!("operate target `{target}` is not a variable: {error}"),
                );
                return;
            }
        };
        let value = match parse(expression).and_then(|expr| eval(&expr, self.store)) {
            Ok(value) => value,
            Err(error) => {
                self.error(
                    "SL-E002",
                    format!("cannot evaluate `{expression}`: {error}"),
                );
                return;
            }
        };
        if let Err(error) = self.store.set(&var, value) {
            self.error("SL-E003", format!("cannot write `{var}`: {error}"));
        }
    }

    /// `state_on_left(col, row)` from `docs/SEMANTICS.md` §2.
    fn state_on_left(&self, grid: &RungGrid, col: usize, row: usize) -> bool {
        if col == 0 {
            // The rail feeds the left side of the cells that sit in column 0.
            // An empty cell in column 0 touches nothing, so a row with no
            // element in column 0 has no path to the rail (see `SL-W001`).
            return grid.cell_exists(0, row);
        }
        let mut result = self.output_prev.get(row).copied().unwrap_or(false);
        let mut y = row;
        while y > 0 && grid.linked_up(col, y) {
            y -= 1;
            result |= self.output_prev.get(y).copied().unwrap_or(false);
        }
        let mut y = row + 1;
        while y < self.cache.max_rows && grid.linked_up(col, y) {
            result |= self.output_prev.get(y).copied().unwrap_or(false);
            y += 1;
        }
        result
    }

    /// `state_on_left` for a block input row, evaluated at the block's column.
    ///
    /// A block never sits in column 0 in a well-formed project (the rail would
    /// feed all of its input rows at once), but the rail rule is applied here
    /// too for consistency.
    fn state_on_left_grid(&self, block: &BlockCell, row: usize) -> bool {
        if block.column == 0 {
            return block.cell_at(row);
        }
        let mut result = self.output_prev.get(row).copied().unwrap_or(false);
        let mut y = row;
        while y > 0 && block.linked_up(y) {
            y -= 1;
            result |= self.output_prev.get(y).copied().unwrap_or(false);
        }
        let mut y = row + 1;
        while y < self.cache.max_rows && block.linked_up(y) {
            result |= self.output_prev.get(y).copied().unwrap_or(false);
            y += 1;
        }
        result
    }
}

/// Reads an integer parameter that may carry an `s`/`S` or `m`/`M` suffix.
///
/// The suffix is only recognised, not applied: this returns the bare number, so
/// `3s` yields `3`. Use [`parse_suffixed_millis`] to get a duration.
fn parse_suffixed_number(parameter: &str) -> Option<i64> {
    let parameter = parameter.trim();
    let digits = parameter.trim_end_matches(['s', 'S', 'm', 'M']);
    if digits.len() == parameter.len() {
        return None;
    }
    digits.trim().parse::<i64>().ok()
}

/// Reads a duration that may carry an `s`/`S` (second) or `m`/`M` (minute)
/// suffix and returns it in milliseconds.
///
/// A parameter with no suffix is returned as it stands, so it is interpreted as
/// milliseconds by the caller.
fn parse_suffixed_millis(parameter: &str) -> Option<i64> {
    let value = parse_suffixed_number(parameter)?;
    let suffix = parameter.trim().chars().last()?;
    let factor = match suffix {
        's' | 'S' => 1_000,
        'm' | 'M' => 60_000,
        _ => 1,
    };
    value.checked_mul(factor)
}

/// Chooses the unit in which a literal preset is counted.
///
/// The preset is a duration in milliseconds and is displayed in whatever unit
/// keeps the numbers readable: whole minutes, whole seconds, or otherwise the
/// 100 ms base. A base is only usable when one scan can actually advance it
/// (`delta_ms` is clamped at [`MAX_DELTA_MS`]) and when it is no larger than the
/// configured scan period, because a base smaller than a scan period would make
/// the timer skip units in the presence of jitter.
fn select_time_base(preset_ms: i64, _period_ms: u64) -> TimeBase {
    let millis = u64::try_from(preset_ms).unwrap_or(0);
    for base in [TimeBase::Minute60, TimeBase::Second, TimeBase::Millis100] {
        let duration = base.millis();
        if duration <= MAX_DELTA_MS && millis % duration == 0 {
            return base;
        }
    }
    TimeBase::Millis100
}

/// Resets a boolean buffer to `value`, reusing its capacity.
fn reset(buffer: &mut Vec<bool>, len: usize, value: bool) {
    buffer.clear();
    buffer.resize(len, value);
}

/// Writes `value` into `slot` at `index` when the index is in bounds.
fn set_edge(slot: &mut [bool], index: usize, value: bool) {
    if let Some(cell) = slot.get_mut(index) {
        *cell = value;
    }
}

/// Renders an element's variable, or its kind when it has none.
fn element_text(element: &PlacedElement) -> String {
    match element.var.as_ref() {
        Some(var) => var.to_string(),
        None => format!("{:?}", element.kind),
    }
}

/// Number of rows a function block occupies, one for a single-cell element.
fn block_span(kind: ElementKind) -> usize {
    match kind {
        ElementKind::Counter { .. } => 4,
        ElementKind::Register { .. } => 3,
        _ => 1,
    }
}

/// A function block occupying several rows of one column.
#[derive(Debug, Clone, PartialEq, Eq)]
struct BlockCell {
    /// Index of the element inside its rung.
    id: u32,
    /// Column the block sits in.
    column: usize,
    /// Row of the block's first input.
    row: u8,
    /// Number of input rows the block reads.
    span: u8,
    /// Variable index of the block instance (`%TM<n>`, `%C<n>`, `%R<n>`).
    index: usize,
    /// Vertical link table of the block's rung, indexed `[column][row]`.
    links: Vec<Vec<bool>>,
    /// Number of rows in the block's rung.
    max_rows: usize,
    /// `true` for every row that holds at least one element, so that the rail
    /// rule can be applied to a block sitting in column 0.
    occupied_rows: Vec<bool>,
}

impl BlockCell {
    /// `true` when the cell at `(column, row)` declares a link with the cell
    /// above it, in the same column.
    fn linked_up(&self, row: usize) -> bool {
        if row == 0 || row >= self.max_rows {
            return false;
        }
        self.links
            .get(self.column)
            .and_then(|column| column.get(row))
            .copied()
            .unwrap_or(false)
    }

    /// `true` when `row` holds a cell, i.e. when the rail reaches its left side.
    fn cell_at(&self, row: usize) -> bool {
        self.occupied_rows.get(row).copied().unwrap_or(false)
    }
}

/// Cell index of a single rung.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct RungGrid {
    /// `cells[col][row]` is the element index at that cell, or [`None`].
    cells: Vec<Vec<Option<u32>>>,
    /// `live_rows[row]` is `true` when the row holds at least one element and
    /// therefore conducts through its empty cells.
    live_rows: Vec<bool>,
    /// `links[col][row]` is the `connected_with_top` flag of the cell at
    /// `(col, row)`. Links are per column: a wire drawn under one column must
    /// not merge the rows of the neighbouring columns.
    links: Vec<Vec<bool>>,
    /// Multi-row blocks, indexed by their first row.
    blocks: Vec<Option<BlockCell>>,
    /// Number of columns, including the implicit wire column `0`.
    columns: usize,
    /// Number of rows in this rung.
    max_rows: usize,
    /// Index of this rung's first element in the flat element numbering used by
    /// the cell index and the edge history.
    element_base: usize,
}

impl RungGrid {
    /// Translates a global element id into an index in this rung's elements.
    fn local_index(&self, id: u32) -> Option<usize> {
        let id = usize::try_from(id).ok()?;
        id.checked_sub(self.element_base)
    }

    /// `true` when the cell at `(col, row)` declares a link with the cell above
    /// it, in the same column.
    fn linked_up(&self, col: usize, row: usize) -> bool {
        if row == 0 || row >= self.max_rows {
            return false;
        }
        self.links
            .get(col)
            .and_then(|column| column.get(row))
            .copied()
            .unwrap_or(false)
    }

    /// `true` when `row` holds at least one element, i.e. when the row is live
    /// and its empty cells behave as wire.
    fn live_row(&self, row: usize) -> bool {
        self.live_rows.get(row).copied().unwrap_or(false)
    }

    /// `true` when `(col, row)` holds an element.
    fn cell_exists(&self, col: usize, row: usize) -> bool {
        self.cells
            .get(col)
            .and_then(|column| column.get(row))
            .is_some_and(Option::is_some)
    }
}

/// Identity of a project shape, used to detect structural edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Shape {
    sections: usize,
    rungs: usize,
    elements: usize,
}

/// Computes the [`Shape`] of a project.
fn shape_of(project: &Project) -> Shape {
    Shape {
        sections: project.sections.len(),
        rungs: project.rungs.len(),
        elements: project.rungs.iter().map(|rung| rung.elements.len()).sum(),
    }
}

/// Precomputed index over the project's rungs and cells.
#[derive(Debug, Clone, Default)]
struct RungCache {
    /// Shape the cache was built from.
    shape: Shape,
    /// One grid per rung of the project, in `Project::rungs` order.
    grids: Vec<RungGrid>,
    /// Rung id to index in [`RungCache::grids`].
    rung_index: BTreeMap<u32, usize>,
    /// Subroutine number to section index.
    subroutines: BTreeMap<u32, usize>,
    /// Total number of elements, i.e. the length of the edge history.
    element_count: usize,
    /// Highest row used by any rung, at least one.
    max_rows: usize,
}

impl RungCache {
    /// Builds the index for `project`.
    fn build(project: &Project) -> Self {
        let mut subroutines: BTreeMap<u32, usize> = BTreeMap::new();
        for (index, section) in project.sections.iter().enumerate() {
            if let Some(number) = section.subroutine {
                subroutines.insert(number, index);
            }
        }

        let mut cache = Self {
            shape: shape_of(project),
            grids: Vec::with_capacity(project.rungs.len()),
            rung_index: BTreeMap::new(),
            subroutines,
            element_count: 0,
            max_rows: 1,
        };

        let mut element_base = 0usize;
        for (grid_index, rung) in project.rungs.iter().enumerate() {
            cache.rung_index.insert(rung.id, grid_index);
            let base = element_base;
            element_base += rung.elements.len();
            cache.element_count += rung.elements.len();
            let max_rows = rung
                .elements
                .iter()
                .map(|element| usize::from(element.row) + block_span(element.kind))
                .max()
                .map_or(1, |row| row.max(1));
            let max_col = rung
                .elements
                .iter()
                .map(|element| usize::from(element.col))
                .max()
                .map_or(0, |col| col + 1);

            let mut cells: Vec<Vec<Option<u32>>> = vec![vec![None; max_rows]; max_col];
            let mut links: Vec<Vec<bool>> = vec![vec![false; max_rows]; max_col];
            let mut occupied_rows = vec![false; max_rows];
            let mut live_rows = vec![false; max_rows];
            let mut blocks: Vec<Option<BlockCell>> = vec![None; max_rows];
            for (local, element) in rung.elements.iter().enumerate() {
                let id = base + local;
                let row = usize::from(element.row);
                let span = block_span(element.kind);
                if let Some(occupied) = occupied_rows.get_mut(row) {
                    *occupied = true;
                }
                if let Some(live) = live_rows.get_mut(row) {
                    *live = true;
                }
                if let Some(cell) = cells
                    .get_mut(usize::from(element.col))
                    .and_then(|column| column.get_mut(row))
                {
                    // A function block is evaluated as a unit from its first
                    // row, so it takes that cell and the plain elements behave as
                    // before: the first one placed wins. A duplicate is reported
                    // by `lint` and ignored by the engine.
                    if cell.is_none() || element.kind.is_block() {
                        *cell = u32::try_from(id).ok();
                    }
                }
                if element.connected_with_top && row > 0 {
                    if let Some(link) = links
                        .get_mut(usize::from(element.col))
                        .and_then(|column| column.get_mut(row))
                    {
                        *link = true;
                    }
                }
                if !element.kind.is_block() {
                    continue;
                }
                // A block without a variable is still a block: it must be
                // evaluated (and diagnosed) from its first row rather than
                // falling through to the single-cell path.
                let index = element
                    .var
                    .as_ref()
                    .and_then(|var| usize::try_from(var.index).ok())
                    .unwrap_or(0);
                if let Some(slot) = blocks.get_mut(row) {
                    *slot = Some(BlockCell {
                        id: u32::try_from(id).unwrap_or(0),
                        column: usize::from(element.col),
                        row: element.row,
                        span: u8::try_from(span).unwrap_or(1),
                        index,
                        links: links.clone(),
                        max_rows,
                        occupied_rows: occupied_rows.clone(),
                    });
                }
            }

            cache.max_rows = cache.max_rows.max(max_rows);
            cache.grids.push(RungGrid {
                cells,
                live_rows,
                links,
                blocks,
                columns: max_col,
                max_rows,
                element_base: base,
            });
        }
        cache
    }
}

/// Resolves a jump target inside `section`.
///
/// A parameter that parses as an integer is a rung index; otherwise it is
/// matched against the labels of the section's rungs.
fn resolve_jump(project: &Project, section: &Section, parameter: &str) -> Option<usize> {
    let target = parameter.trim();
    if let Ok(index) = target.parse::<usize>() {
        return (index < section.rungs.len()).then_some(index);
    }
    if target.is_empty() {
        return None;
    }
    section
        .rungs
        .iter()
        .enumerate()
        .find(|(_, id)| project.rung(**id).is_some_and(|rung| rung.label == target))
        .map(|(position, _)| position)
}

/// Structural checks that do not need a scan.
///
/// `lint` reports, per `docs/SEMANTICS.md` §5:
///
/// * `SL-E009` — two elements placed on the same `(col, row)` cell,
/// * `SL-E005` — a `CoilJump` whose target is neither a rung index nor a label,
/// * `SL-E007` — a `CoilCall` to a section that is not a subroutine,
/// * `SL-E004` — an element missing its required variable,
/// * `SL-E003` — an element/variable kind mismatch,
/// * `SL-W001` — a live row with no element in column 0, i.e. no path to the
///   left rail,
/// * `SL-W002` — an SFC section, which the engine skips until M4.
///
/// Diagnostics carry the section and rung indices. Out-of-range variable
/// indices are *not* reported here because a store may legitimately be grown on
/// demand; the scan reports them as `SL-E003` when they cannot be addressed.
pub fn lint(project: &Project) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let subroutines: Vec<u32> = project
        .sections
        .iter()
        .filter_map(|section| section.subroutine)
        .collect();

    for (index, section) in project.sections.iter().enumerate() {
        if section.language == SectionLanguage::Sfc {
            diagnostics.push(
                Diagnostic::new(
                    Severity::Warning,
                    "SL-W002",
                    format!("SFC section `{}` is skipped until M4", section.name),
                )
                .with_section(index),
            );
        }
        for (position, rung_id) in section.rungs.iter().enumerate() {
            let Some(rung) = project.rung(*rung_id) else {
                diagnostics.push(context(
                    Diagnostic::new(
                        Severity::Error,
                        "SL-E011",
                        format!(
                            "section `{}` references rung {rung_id}, which does not exist",
                            section.name
                        ),
                    ),
                    index,
                    position,
                ));
                continue;
            };
            let mut seen: Vec<(u8, u8)> = Vec::new();
            let mut live_rows: Vec<u8> = Vec::new();
            let mut has_left_rail: Vec<u8> = Vec::new();

            for element in &rung.elements {
                if seen.contains(&(element.col, element.row)) {
                    diagnostics.push(context(
                        Diagnostic::new(
                            Severity::Error,
                            "SL-E009",
                            format!(
                                "two elements are placed on cell (col {}, row {})",
                                element.col, element.row
                            ),
                        ),
                        index,
                        position,
                    ));
                } else {
                    seen.push((element.col, element.row));
                }
                if !live_rows.contains(&element.row) {
                    live_rows.push(element.row);
                }
                if element.col == 0 && !has_left_rail.contains(&element.row) {
                    has_left_rail.push(element.row);
                }

                for (code, message) in check_element(project, section, element, &subroutines) {
                    diagnostics.push(context(
                        Diagnostic::new(Severity::Error, code, message),
                        index,
                        position,
                    ));
                }
            }

            for row in live_rows {
                if !has_left_rail.contains(&row) {
                    diagnostics.push(context(
                        Diagnostic::new(
                            Severity::Warning,
                            "SL-W001",
                            format!(
                                "row {row} of rung {} is live but has no path to the left rail",
                                rung.id
                            ),
                        ),
                        index,
                        position,
                    ));
                }
            }
        }
    }

    diagnostics
}

/// Attaches section and rung context to a diagnostic.
fn context(diagnostic: Diagnostic, section: usize, rung: usize) -> Diagnostic {
    diagnostic.with_section(section).with_rung(rung)
}

/// Checks one element and returns the `(code, message)` pairs it raises.
fn check_element(
    project: &Project,
    section: &Section,
    element: &PlacedElement,
    subroutines: &[u32],
) -> Vec<(&'static str, String)> {
    let mut found = Vec::new();
    let needs_variable = !matches!(
        element.kind,
        ElementKind::Compare
            | ElementKind::Operate
            | ElementKind::Connection
            | ElementKind::CoilJump
            | ElementKind::CoilCall
    );
    if needs_variable && element.var.is_none() {
        found.push((
            "SL-E004",
            format!("{:?} element has no variable", element.kind),
        ));
    }
    if matches!(element.kind, ElementKind::CoilJump | ElementKind::CoilCall)
        && element.params.is_empty()
    {
        found.push((
            "SL-E004",
            format!("{:?} element has no target", element.kind),
        ));
    }
    if let Some(var) = element.var.as_ref() {
        match element.kind {
            ElementKind::ContactNo
            | ElementKind::ContactNc
            | ElementKind::ContactRising
            | ElementKind::ContactFalling
            | ElementKind::CoilOut
            | ElementKind::CoilOutNeg
            | ElementKind::CoilSet
            | ElementKind::CoilReset
            | ElementKind::CoilJump
            | ElementKind::CoilCall => {
                if !var.is_bit() {
                    found.push((
                        "SL-E003",
                        format!(
                            "`{var}` is a word but the {:?} element needs a bit",
                            element.kind
                        ),
                    ));
                }
            }
            ElementKind::Timer { .. } => {
                if var.kind != VarKind::TimerIec {
                    found.push((
                        "SL-E003",
                        format!("timer element needs a `%TM` variable, found `{var}`"),
                    ));
                }
            }
            ElementKind::Counter { .. } => {
                if var.kind != VarKind::Counter {
                    found.push((
                        "SL-E003",
                        format!("counter element needs a `%C` variable, found `{var}`"),
                    ));
                }
            }
            ElementKind::Register { .. } => {
                if var.kind != VarKind::Register {
                    found.push((
                        "SL-E003",
                        format!("register element needs a `%R` variable, found `{var}`"),
                    ));
                }
            }
            ElementKind::Compare | ElementKind::Operate | ElementKind::Connection => {}
        }
    }

    match element.kind {
        ElementKind::CoilJump => {
            let Some(parameter) = element.params.first() else {
                found.push(("SL-E004", "jump coil has no target".to_owned()));
                return found;
            };
            let target = parameter.trim();
            let resolved = target.parse::<usize>().map_or_else(
                |_| {
                    !target.is_empty()
                        && section
                            .rungs
                            .iter()
                            .any(|id| project.rung(*id).is_some_and(|rung| rung.label == target))
                },
                |index| index < section.rungs.len(),
            );
            if !resolved {
                found.push((
                    "SL-E005",
                    format!("jump target `{target}` does not exist in this section"),
                ));
            }
        }
        ElementKind::CoilCall => {
            let Some(parameter) = element.params.first() else {
                found.push(("SL-E004", "call coil has no subroutine number".to_owned()));
                return found;
            };
            match parameter.trim().parse::<u32>() {
                Ok(number) => {
                    if !subroutines.contains(&number) {
                        found.push((
                            "SL-E007",
                            format!("call to undefined or non-subroutine section {number}"),
                        ));
                    }
                }
                Err(_) => found.push((
                    "SL-E007",
                    format!("call target `{parameter}` is not a subroutine number"),
                )),
            }
        }
        ElementKind::Operate => {
            if let Some(target) = element.params.first() {
                if target.trim().parse::<VarRef>().is_err() {
                    found.push((
                        "SL-E003",
                        format!("operate target `{target}` is not a variable"),
                    ));
                }
            }
        }
        _ => {}
    }

    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Section;

    fn var(text: &str) -> VarRef {
        text.parse::<VarRef>().expect("test variable parses")
    }

    fn element(kind: ElementKind, var_text: Option<&str>, col: u8, row: u8) -> PlacedElement {
        PlacedElement {
            kind,
            var: var_text.map(var),
            col,
            row,
            connected_with_top: false,
            params: Vec::new(),
        }
    }

    fn element_with(
        kind: ElementKind,
        var_text: Option<&str>,
        col: u8,
        row: u8,
        params: &[&str],
    ) -> PlacedElement {
        PlacedElement {
            params: params.iter().map(|text| (*text).to_owned()).collect(),
            ..element(kind, var_text, col, row)
        }
    }

    /// The same building block as `element`, wired to the cell above it.
    fn linked(kind: ElementKind, var_text: Option<&str>, col: u8, row: u8) -> PlacedElement {
        PlacedElement {
            connected_with_top: true,
            ..element(kind, var_text, col, row)
        }
    }

    fn rung(id: u32, elements: Vec<PlacedElement>) -> Rung {
        Rung {
            elements,
            ..Rung::new(id)
        }
    }

    fn project(sections: Vec<Section>, rungs: Vec<Rung>) -> Project {
        Project {
            sections,
            rungs,
            ..Project::new("scan test")
        }
    }

    /// A project with one main section holding every rung, in order.
    fn single_section(rungs: Vec<Rung>) -> Project {
        let ids = rungs.iter().map(|rung| rung.id).collect();
        project(
            vec![Section {
                rungs: ids,
                ..Section::new(1, "Main")
            }],
            rungs,
        )
    }

    fn engine(rungs: Vec<Rung>) -> ScanEngine {
        ScanEngine::new(single_section(rungs))
    }

    fn set_bit(engine: &mut ScanEngine, text: &str, value: bool) {
        engine
            .store_mut()
            .set(&var(text), Value::Bit(value))
            .expect("bit can be set");
    }

    fn get(engine: &ScanEngine, text: &str) -> Option<Value> {
        engine.store().get(&var(text))
    }

    fn codes(report: &ScanReport) -> Vec<&'static str> {
        report
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code)
            .collect()
    }

    fn has(report: &ScanReport, code: &str) -> bool {
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == code)
    }

    // ---------------------------------------------------------------------
    // The store
    // ---------------------------------------------------------------------

    #[test]
    fn store_reads_and_writes_bits_and_words() {
        let mut store = VarStore::with_default_sizes();
        assert_eq!(store.get(&var("%M0")), Some(Value::Bit(false)));
        store
            .set(&var("%M0"), Value::Bit(true))
            .expect("memory bit is writable");
        assert_eq!(store.get(&var("%M0")), Some(Value::Bit(true)));

        store
            .set(&var("%MW5"), Value::Word(-7))
            .expect("memory word is writable");
        assert_eq!(store.get(&var("%MW5")), Some(Value::Word(-7)));

        // Bits are coerced from any non-zero value.
        store
            .set(&var("%Q0"), Value::Word(9))
            .expect("output bit is writable");
        assert_eq!(store.get(&var("%Q0")), Some(Value::Bit(true)));
    }

    #[test]
    fn store_grows_on_demand_and_reports_out_of_range() {
        let mut store = VarStore::new();
        assert_eq!(store.get(&var("%Q7")), None);
        store
            .set(&var("%Q7"), Value::Bit(true))
            .expect("writing grows the store");
        assert_eq!(store.get(&var("%Q7")), Some(Value::Bit(true)));
        assert_eq!(store.digital_channels(), 8);
        assert_eq!(store.analog_channels(), 0);

        // Beyond the growth limit nothing is allocated and the error is clean.
        let far = VarRef::new(VarKind::MemBit, MAX_GROWTH as u32 + 1);
        assert_eq!(
            store.set(&far, Value::Bit(true)),
            Err(StoreError::OutOfRange(far.clone()))
        );
        assert_eq!(store.get(&far), None);
    }

    #[test]
    fn store_reads_and_writes_word_bits_in_place() {
        let mut store = VarStore::with_default_sizes();
        store
            .set(&var("%MW0"), Value::Word(0))
            .expect("word is writable");
        store
            .set(&var("%MW0.3"), Value::Bit(true))
            .expect("a bit of a word is writable");
        assert_eq!(store.get(&var("%MW0")), Some(Value::Word(0b1000)));
        assert_eq!(store.get(&var("%MW0.3")), Some(Value::Bit(true)));
        assert_eq!(store.get(&var("%MW0.2")), Some(Value::Bit(false)));
        assert_eq!(store.get(&var("%MW0.31")), Some(Value::Bit(false)));

        // Clearing a bit leaves the others alone, including the sign bit.
        store
            .set(&var("%MW0"), Value::Word(-1))
            .expect("word is writable");
        store
            .set(&var("%MW0.0"), Value::Bit(false))
            .expect("clearing a bit keeps the rest");
        assert_eq!(store.get(&var("%MW0")), Some(Value::Word(-2)));

        // The same rule applies to physical words.
        store
            .set(&var("%QW1.5"), Value::Bit(true))
            .expect("physical word bit is writable");
        assert_eq!(store.get(&var("%QW1")), Some(Value::Word(32)));
        store
            .set(&var("%IW3.5"), Value::Bit(true))
            .expect("physical input word bit is writable");
        assert_eq!(store.get(&var("%IW3")), Some(Value::Word(32)));
    }

    #[test]
    fn store_resolves_indirect_indices_and_refuses_bad_ones() {
        let mut store = VarStore::with_default_sizes();
        store
            .set(&var("%MW4"), Value::Word(42))
            .expect("word is writable");
        store
            .set(&var("%MW0"), Value::Word(4))
            .expect("index word is writable");
        assert_eq!(store.get(&var("%MW[%MW0]")), Some(Value::Word(42)));
        store
            .set(&var("%MW[%MW0]"), Value::Word(43))
            .expect("an indirect write resolves the index");
        assert_eq!(store.get(&var("%MW4")), Some(Value::Word(43)));

        // A negative index is an error, never a panic.
        store
            .set(&var("%MW0"), Value::Word(-1))
            .expect("index word is writable");
        assert_eq!(store.get(&var("%MW[%MW0]")), None);
        assert!(matches!(
            store.set(&var("%MW[%MW0]"), Value::Word(1)),
            Err(StoreError::BadIndex(_, _))
        ));

        // An index variable that does not exist is an error too, and the
        // unresolved reference must not fall back to index zero.
        let unknown: VarRef = "%MW[%C99.D]".parse().expect("parses");
        assert_eq!(store.get(&unknown), None);
        assert!(matches!(
            store.set(&unknown, Value::Word(1)),
            Err(StoreError::BadIndex(_, _))
        ));

        // An indirect index that resolves in range is fine. The default store
        // holds 200 words, so `%MW42` exists.
        store
            .set(&var("%MW0"), Value::Word(42))
            .expect("index word is writable");
        assert_eq!(store.get(&var("%MW[%MW0]")), Some(Value::Word(0)));

        // With a small store the very same reference is out of range.
        let mut small = VarStore::new();
        small
            .set(&var("%MW0"), Value::Word(42))
            .expect("index word is writable");
        assert_eq!(small.get(&var("%MW[%MW0]")), None);
    }

    #[test]
    fn store_exposes_timer_and_step_accessors() {
        let mut store = VarStore::new();
        store
            .set(&var("%TM0.P"), Value::Word(30))
            .expect("preset is writable");
        store
            .set(&var("%TM0.V"), Value::Word(12))
            .expect("elapsed is writable");
        assert_eq!(store.get(&var("%TM0.P")), Some(Value::Word(30)));
        assert_eq!(store.get(&var("%TM0.V")), Some(Value::Word(12)));
        assert_eq!(store.get(&var("%TM0")), Some(Value::Bit(false)));
        assert_eq!(store.get(&var("%TM0.Q")), Some(Value::Bit(false)));

        store
            .set(&var("%TM0.Q"), Value::Bit(true))
            .expect("done is writable");
        assert_eq!(store.get(&var("%TM0")), Some(Value::Bit(true)));

        store
            .set(&var("%X2.A"), Value::Bit(true))
            .expect("activity is writable");
        store
            .set(&var("%X2.V"), Value::Word(250))
            .expect("step age is writable");
        assert_eq!(store.get(&var("%X2")), Some(Value::Bit(true)));
        assert_eq!(store.get(&var("%X2.V")), Some(Value::Word(250)));

        // Register status flags are not writable.
        assert!(matches!(
            store.set(&var("%R0.E"), Value::Bit(true)),
            Err(StoreError::NotWritable(_))
        ));
        assert!(matches!(
            store.set(&var("%R0.F"), Value::Bit(true)),
            Err(StoreError::NotWritable(_))
        ));
        assert!(matches!(
            store.set(&var("%R0.S"), Value::Word(3)),
            Err(StoreError::NotWritable(_))
        ));
    }

    #[test]
    fn store_exposes_counter_and_register_accessors() {
        let mut store = VarStore::new();
        store
            .set(&var("%C0.P"), Value::Word(4))
            .expect("preset is writable");
        store
            .set(&var("%C0.V"), Value::Word(4))
            .expect("value is writable");
        assert_eq!(store.get(&var("%C0.P")), Some(Value::Word(4)));
        assert_eq!(store.get(&var("%C0.V")), Some(Value::Word(4)));
        assert_eq!(store.get(&var("%C0.D")), Some(Value::Bit(true)));
        assert_eq!(store.get(&var("%C0")), Some(Value::Bit(true)));

        store
            .set(&var("%R0.I"), Value::Word(11))
            .expect("register input is writable");
        store
            .set(&var("%R0.O"), Value::Word(22))
            .expect("register output is writable");
        assert_eq!(store.get(&var("%R0.I")), Some(Value::Word(11)));
        assert_eq!(store.get(&var("%R0.O")), Some(Value::Word(22)));
        assert_eq!(store.get(&var("%R0.E")), Some(Value::Bit(true)));
        assert_eq!(store.get(&var("%R0")), Some(Value::Bit(true)));
        assert_eq!(store.get(&var("%R0.F")), Some(Value::Bit(false)));
        assert_eq!(store.get(&var("%R0.S")), Some(Value::Word(0)));
    }

    #[test]
    fn store_mut_helpers_reach_the_function_blocks() {
        let mut store = VarStore::with_default_sizes();
        assert!(store.timer_mut(0).is_some());
        assert!(store.counter_mut(0).is_some());
        assert!(store.register_mut(0).is_some());
        assert!(store.timer_mut(usize::MAX).is_none());
        assert!(store.counter_mut(usize::MAX).is_none());
        assert!(store.register_mut(usize::MAX).is_none());
    }

    // ---------------------------------------------------------------------
    // Function blocks in isolation
    // ---------------------------------------------------------------------

    #[test]
    fn ton_fires_exactly_at_its_preset_and_resets_when_the_input_drops() {
        let mut timer = TimerIec::new(TimerMode::On, 3, TimeBase::Millis100);
        // The first scan only establishes the time base.
        assert!(!timer.update(0, true));
        assert_eq!(timer.elapsed, 0);
        // One unit of the 100 ms base per 100 ms scan.
        assert!(!timer.update(100, true));
        assert_eq!(timer.elapsed, 1);
        assert!(!timer.update(100, true));
        assert_eq!(timer.elapsed, 2);
        // The third unit lands exactly on the preset.
        assert!(timer.update(100, true));
        assert_eq!(timer.elapsed, 3);
        // It stays done and does not count past the preset.
        assert!(timer.update(100, true));
        assert_eq!(timer.elapsed, 3);

        // Losing the input resets both the output and the elapsed counter.
        assert!(!timer.update(100, false));
        assert_eq!(timer.elapsed, 0);
        assert_eq!(timer.acc_ms, 0);
        // Re-enabling restarts from scratch and fires again after the preset.
        assert!(!timer.update(100, true));
        assert!(!timer.update(100, true));
        assert!(timer.update(100, true));
    }

    #[test]
    fn ton_quantizes_irregular_scan_periods_without_drifting() {
        let mut timer = TimerIec::new(TimerMode::On, 5, TimeBase::Millis100);
        // Exactly one unit is produced per scan and the milliseconds that do not
        // complete a unit are carried over, so an irregular scan period neither
        // loses time nor makes the timer run fast.
        assert!(!timer.update(0, true));
        assert!(!timer.update(40, true));
        assert_eq!(timer.acc_ms, 40);
        assert!(!timer.update(30, true));
        assert_eq!(timer.acc_ms, 70);
        assert!(!timer.update(130, true));
        assert_eq!(timer.elapsed, 2, "70 + 130 is worth two whole units");
        assert_eq!(timer.acc_ms, 0);
        // A short scan still spends the remainder of the previous one.
        assert!(!timer.update(40, true));
        assert_eq!(timer.elapsed, 2);
        assert_eq!(timer.acc_ms, 40);
        assert!(!timer.update(60, true));
        assert_eq!(timer.elapsed, 3);
        assert_eq!(timer.acc_ms, 0);
        // Two more units reach the preset exactly.
        assert!(!timer.update(100, true));
        assert_eq!(timer.elapsed, 4);
        assert!(timer.update(100, true));
        assert_eq!(timer.elapsed, 5);
        assert_eq!(timer.acc_ms, 0, "a completed timer banks no time");
    }

    #[test]
    fn tof_holds_the_output_through_the_off_delay() {
        let mut timer = TimerIec::new(TimerMode::Off, 2, TimeBase::Millis100);
        // The first scan only establishes the edge state.
        assert!(!timer.update(0, false));
        assert!(timer.update(10, true));
        // The output follows the input while it is on.
        assert!(timer.update(100, true));
        assert_eq!(timer.elapsed, 0);
        // The falling edge starts the off delay and produces its first unit.
        assert!(timer.update(100, false));
        assert_eq!(timer.elapsed, 1);
        // The delay keeps counting even though the edge is long gone, and drops
        // the output exactly at the preset.
        assert!(!timer.update(100, false));
        assert_eq!(timer.elapsed, 0);
        assert_eq!(timer.acc_ms, 0);
        // The output stays down while the input stays down.
        assert!(!timer.update(100, false));

        // A low input before the first observation is not a falling edge.
        let mut fresh = TimerIec::new(TimerMode::Off, 2, TimeBase::Millis100);
        assert!(!fresh.update(100, false));
        assert!(!fresh.update(100, false));
        assert_eq!(fresh.elapsed, 0);

        // A later high pulse re-arms the delay from scratch.
        let mut rearm = TimerIec::new(TimerMode::Off, 2, TimeBase::Millis100);
        assert!(rearm.update(0, true));
        assert!(rearm.update(100, false));
        assert_eq!(rearm.elapsed, 1);
        assert!(rearm.update(100, true));
        assert_eq!(rearm.elapsed, 0);
        assert!(rearm.update(100, true));
        assert!(rearm.update(100, false));
        assert_eq!(rearm.elapsed, 1);
    }

    #[test]
    fn tp_is_not_retriggerable_and_runs_for_exactly_the_preset() {
        let mut timer = TimerIec::new(TimerMode::Pulse, 2, TimeBase::Millis100);
        assert!(!timer.update(0, false));
        // The rising edge starts the one-shot.
        assert!(timer.update(10, true));
        assert_eq!(timer.elapsed, 0);
        // The next scan produces the first unit, which is not yet the preset.
        assert!(timer.update(100, true));
        assert_eq!(timer.elapsed, 1);
        // Exactly at the preset the pulse is over, but the output stays high and
        // the still-high input does not start a second pulse.
        assert!(timer.update(100, true));
        assert_eq!(timer.elapsed, 2);
        assert!(timer.update(100, true));
        assert!(timer.update(100, true));
        // Dropping the input ends the pulse.
        assert!(!timer.update(100, false));
        // A new rising edge starts a fresh pulse.
        assert!(timer.update(100, true));
        assert!(timer.update(100, true));
        assert!(timer.update(100, true));
        assert!(!timer.update(100, false));
    }

    #[test]
    fn counter_counts_up_down_wraps_and_reports_full_and_empty() {
        let mut up = Counter::new(CounterKind::Up, 3);
        up.emulate(true, false, false, false);
        assert_eq!(up.value, 1);
        up.emulate(true, false, false, false);
        assert_eq!(up.value, 1, "a level input only counts on its rising edge");
        for _ in 0..2 {
            up.emulate(false, false, false, false);
            up.emulate(true, false, false, false);
        }
        assert_eq!(up.value, 3);
        assert!(up.done());

        assert!(!up.is_full());
        assert!(!up.is_empty());
        // Loading the preset is level triggered.
        up.emulate(false, false, false, true);
        assert_eq!(up.value, 3);
        // A reset is not a wrap, so the wrap flags stay clear.
        up.emulate(false, false, true, false);
        assert_eq!(up.value, 0);
        assert!(!up.is_empty());
        assert!(!up.is_full());
    }

    #[test]
    fn counter_wraps_up_from_9999_into_zero_and_sets_full() {
        let mut counter = Counter::new(CounterKind::Up, 5);
        counter.value = COUNTER_MAX;
        counter.emulate(true, false, false, false);
        assert_eq!(counter.value, 0);
        assert!(counter.is_full());
        assert!(!counter.is_empty());

        // A simultaneous preset and reset ends at zero, as the reference does.
        counter.value = 500;
        counter.prev_up = false;
        counter.emulate(true, false, true, true);
        assert_eq!(counter.value, 0);
    }

    #[test]
    fn counter_wraps_down_from_zero_into_9999_and_sets_empty() {
        let mut counter = Counter::new(CounterKind::Down, 0);
        counter.value = 0;
        counter.emulate(false, true, false, false);
        assert_eq!(counter.value, COUNTER_MAX);
        assert!(counter.is_empty());
        assert!(!counter.is_full());
        assert!(!counter.done());

        // The down counter starts at its preset on the first update and counts
        // down one step per rising edge.
        let mut started = Counter::new(CounterKind::Down, 3);
        started.update(false, false, false, false);
        assert_eq!(started.value, 3);
        assert!(started.done());
        for expected in (0..3).rev() {
            started.update(false, true, false, false);
            assert_eq!(started.value, expected);
            started.update(false, false, false, false);
        }
        // `%C<n>.D` compares against the preset, which a down counter is moving
        // away from, so the done bit is clear at zero.
        assert_eq!(started.value, 0);
        assert!(!started.done());
    }

    #[test]
    fn counter_updown_honours_both_edges() {
        let mut counter = Counter::new(CounterKind::UpDown, 2);
        counter.emulate(true, false, false, false);
        counter.emulate(false, false, false, false);
        counter.emulate(true, false, false, false);
        assert_eq!(counter.value, 2);
        assert!(counter.done());
        counter.emulate(false, true, false, false);
        counter.emulate(false, false, false, false);
        counter.emulate(false, true, false, false);
        assert_eq!(counter.value, 0);
    }

    #[test]
    fn register_pushes_and_pops_in_fifo_and_lifo_order() {
        let mut fifo = RegisterState::new(RegisterMode::Fifo, 3);
        assert!(fifo.is_empty());
        for value in [11, 22, 33] {
            fifo.in_value = value;
            fifo.update(false, true, false);
            fifo.update(false, false, false);
        }
        assert_eq!(fifo.stored(), 3);
        assert_eq!(fifo.values.front(), Some(&11));
        assert!(fifo.is_full());

        fifo.update(false, false, true);
        assert_eq!(fifo.out_value, 11);
        fifo.update(false, false, false);
        fifo.update(false, false, true);
        assert_eq!(fifo.out_value, 22);

        fifo.update(true, false, false);
        assert!(fifo.is_empty());
        assert_eq!(fifo.out_value, 0);
        assert_eq!(fifo.stored(), 0);

        let mut lifo = RegisterState::new(RegisterMode::Lifo, 3);
        for value in [11, 22, 33] {
            lifo.in_value = value;
            lifo.update(false, true, false);
            lifo.update(false, false, false);
        }
        lifo.update(false, false, true);
        assert_eq!(lifo.out_value, 33);
        lifo.update(false, false, false);
        lifo.update(false, false, true);
        assert_eq!(lifo.out_value, 22);
    }

    #[test]
    fn register_full_and_empty_operations_are_no_ops() {
        let mut register = RegisterState::new(RegisterMode::Fifo, 2);
        assert!(register.push(1));
        assert!(register.push(2));
        assert!(register.is_full());
        assert!(!register.push(3));
        assert_eq!(register.stored(), 2);
        assert_eq!(register.pop(), Some(1));
        assert_eq!(register.pop(), Some(2));
        assert_eq!(register.pop(), None);
        assert!(register.is_empty());

        // Shrinking the capacity drops the oldest values.
        let mut shrinking = RegisterState::new(RegisterMode::Fifo, 4);
        for value in 1..=4 {
            shrinking.push(value);
        }
        shrinking.set_capacity(2);
        assert_eq!(shrinking.stored(), 2);
        assert_eq!(shrinking.pop(), Some(3));
    }

    #[test]
    fn edge_bank_helper_detects_edges_and_can_be_cleared() {
        let mut bank = EdgeBank::new();
        assert!(bank.is_empty());
        assert!(bank.rising(&var("%M0"), true));
        assert!(!bank.rising(&var("%M0"), true));
        assert!(bank.falling(&var("%M0"), false));
        assert_eq!(bank.len(), 1);
        bank.clear();
        assert!(bank.is_empty());
    }

    // ---------------------------------------------------------------------
    // Rung evaluation
    // ---------------------------------------------------------------------

    #[test]
    fn serial_contacts_and_output_coils() {
        let mut engine = engine(vec![rung(
            1,
            vec![
                element(ElementKind::ContactNo, Some("%I0"), 0, 0),
                element(ElementKind::ContactNo, Some("%I1"), 1, 0),
                element(ElementKind::CoilOut, Some("%Q0"), 2, 0),
            ],
        )]);
        engine.scan_once(0);
        assert_eq!(get(&engine, "%Q0"), Some(Value::Bit(false)));

        set_bit(&mut engine, "%I0", true);
        engine.scan_once(10);
        assert_eq!(get(&engine, "%Q0"), Some(Value::Bit(false)));

        set_bit(&mut engine, "%I1", true);
        engine.scan_once(20);
        assert_eq!(get(&engine, "%Q0"), Some(Value::Bit(true)));

        // An output coil is rewritten from its input every scan.
        set_bit(&mut engine, "%I0", false);
        engine.scan_once(30);
        assert_eq!(get(&engine, "%Q0"), Some(Value::Bit(false)));
    }

    #[test]
    fn normally_closed_contact_inverts_its_variable() {
        let mut engine = engine(vec![rung(
            1,
            vec![
                element(ElementKind::ContactNc, Some("%I0"), 0, 0),
                element(ElementKind::CoilOut, Some("%Q0"), 1, 0),
            ],
        )]);
        engine.scan_once(0);
        assert_eq!(get(&engine, "%Q0"), Some(Value::Bit(true)));
        set_bit(&mut engine, "%I0", true);
        engine.scan_once(10);
        assert_eq!(get(&engine, "%Q0"), Some(Value::Bit(false)));
    }

    #[test]
    fn set_and_reset_coils_are_level_triggered() {
        let mut engine = engine(vec![
            rung(
                1,
                vec![
                    element(ElementKind::ContactNo, Some("%I0"), 0, 0),
                    element(ElementKind::CoilSet, Some("%M0"), 1, 0),
                ],
            ),
            rung(
                2,
                vec![
                    element(ElementKind::ContactNo, Some("%I1"), 0, 0),
                    element(ElementKind::CoilReset, Some("%M0"), 1, 0),
                ],
            ),
        ]);
        set_bit(&mut engine, "%I0", true);
        engine.scan_once(0);
        assert_eq!(get(&engine, "%M0"), Some(Value::Bit(true)));

        // Removing the flow leaves the latch set.
        set_bit(&mut engine, "%I0", false);
        engine.scan_once(10);
        assert_eq!(get(&engine, "%M0"), Some(Value::Bit(true)));

        set_bit(&mut engine, "%I1", true);
        engine.scan_once(20);
        assert_eq!(get(&engine, "%M0"), Some(Value::Bit(false)));

        set_bit(&mut engine, "%I1", false);
        engine.scan_once(30);
        assert_eq!(get(&engine, "%M0"), Some(Value::Bit(false)));
    }

    #[test]
    fn serial_coils_chain_the_flow() {
        let mut engine = engine(vec![rung(
            1,
            vec![
                element(ElementKind::ContactNo, Some("%I0"), 0, 0),
                element(ElementKind::CoilOut, Some("%Q0"), 1, 0),
                element(ElementKind::CoilOutNeg, Some("%Q1"), 2, 0),
            ],
        )]);
        set_bit(&mut engine, "%I0", true);
        engine.scan_once(0);
        assert_eq!(get(&engine, "%Q0"), Some(Value::Bit(true)));
        // `output := input`, so the negated coil still sees the live flow.
        assert_eq!(get(&engine, "%Q1"), Some(Value::Bit(false)));
    }

    #[test]
    fn rising_and_falling_edges_are_consumed_in_one_scan() {
        let mut engine = engine(vec![
            rung(
                1,
                vec![
                    element(ElementKind::ContactRising, Some("%I0"), 0, 0),
                    element(ElementKind::CoilOut, Some("%Q0"), 1, 0),
                ],
            ),
            rung(
                2,
                vec![
                    element(ElementKind::ContactFalling, Some("%I0"), 0, 0),
                    element(ElementKind::CoilOut, Some("%Q1"), 1, 0),
                ],
            ),
        ]);
        engine.scan_once(0);
        assert_eq!(get(&engine, "%Q0"), Some(Value::Bit(false)));

        // The rising edge is seen by exactly one scan.
        set_bit(&mut engine, "%I0", true);
        engine.scan_once(10);
        assert_eq!(get(&engine, "%Q0"), Some(Value::Bit(true)));
        assert_eq!(get(&engine, "%Q1"), Some(Value::Bit(false)));
        engine.scan_once(20);
        assert_eq!(get(&engine, "%Q0"), Some(Value::Bit(false)));

        // So is the falling edge.
        set_bit(&mut engine, "%I0", false);
        engine.scan_once(30);
        assert_eq!(get(&engine, "%Q1"), Some(Value::Bit(true)));
        engine.scan_once(40);
        assert_eq!(get(&engine, "%Q1"), Some(Value::Bit(false)));
    }

    #[test]
    fn each_edge_cell_keeps_its_own_previous_state() {
        let mut engine = engine(vec![rung(
            1,
            vec![
                element(ElementKind::ContactRising, Some("%I0"), 0, 0),
                element(ElementKind::ContactRising, Some("%I0"), 1, 0),
                element(ElementKind::CoilOut, Some("%Q0"), 2, 0),
            ],
        )]);
        // The first scan establishes the state of both cells while the variable
        // is still low, so nothing fires.
        engine.scan_once(0);
        assert_eq!(get(&engine, "%Q0"), Some(Value::Bit(false)));
        // The rising edge fires the chain once.
        set_bit(&mut engine, "%I0", true);
        engine.scan_once(10);
        assert_eq!(get(&engine, "%Q0"), Some(Value::Bit(true)));
        // Both cells then remember the high value independently.
        engine.scan_once(20);
        assert_eq!(get(&engine, "%Q0"), Some(Value::Bit(false)));
        // A second rising edge fires the chain again.
        set_bit(&mut engine, "%I0", false);
        engine.scan_once(30);
        set_bit(&mut engine, "%I0", true);
        engine.scan_once(40);
        assert_eq!(get(&engine, "%Q0"), Some(Value::Bit(true)));
    }

    #[test]
    fn empty_cells_conduct_in_a_live_row_but_an_empty_row_is_inert() {
        let mut engine = engine(vec![
            rung(
                1,
                vec![
                    element(ElementKind::ContactNo, Some("%I0"), 0, 0),
                    // Column 1 is empty, but row 0 is live so it conducts.
                    element(ElementKind::CoilOut, Some("%Q0"), 2, 0),
                ],
            ),
            rung(
                2,
                vec![
                    // Row 1 of this rung has no cell at all, so the coil has no
                    // path to the rail.
                    element(ElementKind::CoilOut, Some("%Q1"), 1, 1),
                ],
            ),
        ]);
        set_bit(&mut engine, "%I0", true);
        engine.scan_once(0);
        assert_eq!(get(&engine, "%Q0"), Some(Value::Bit(true)));
        assert_eq!(get(&engine, "%Q1"), Some(Value::Bit(false)));
    }

    #[test]
    fn a_row_whose_leftmost_element_is_not_in_column_zero_is_unreachable() {
        let mut engine = engine(vec![rung(
            1,
            vec![
                element(ElementKind::ContactNo, Some("%I0"), 1, 0),
                element(ElementKind::CoilOut, Some("%Q0"), 2, 0),
            ],
        )]);
        set_bit(&mut engine, "%I0", true);
        engine.scan_once(0);
        assert_eq!(get(&engine, "%Q0"), Some(Value::Bit(false)));
    }

    #[test]
    fn parallel_branches_merge_through_a_vertical_link() {
        let mut engine = engine(vec![rung(
            1,
            vec![
                element(ElementKind::ContactNo, Some("%I0"), 0, 0),
                element(ElementKind::ContactNo, Some("%I1"), 0, 1),
                // The link lives in the column that merges the branches, so the
                // coil's own input is the OR of both rows. Links are per column:
                // a link placed one column earlier would not reach the coil.
                linked(ElementKind::Connection, None, 2, 1),
                element(ElementKind::CoilOut, Some("%Q0"), 2, 0),
            ],
        )]);

        // Both branches open: the merge point and the coil above it see nothing.
        engine.scan_once(0);
        assert_eq!(get(&engine, "%Q0"), Some(Value::Bit(false)));

        // Either branch energises the merge point, and the coil taps it through
        // its own upward link.
        for (first, second) in [(true, false), (false, true), (true, true)] {
            set_bit(&mut engine, "%I0", first);
            set_bit(&mut engine, "%I1", second);
            engine.scan_once(10);
            assert_eq!(
                get(&engine, "%Q0"),
                Some(Value::Bit(true)),
                "branch ({first}, {second}) must energise the coil"
            );
        }

        set_bit(&mut engine, "%I0", false);
        set_bit(&mut engine, "%I1", false);
        engine.scan_once(20);
        assert_eq!(get(&engine, "%Q0"), Some(Value::Bit(false)));
    }

    #[test]
    fn vertical_links_share_power_between_rows_of_the_same_column() {
        let mut engine = engine(vec![rung(
            1,
            vec![
                element(ElementKind::ContactNo, Some("%I0"), 0, 1),
                linked(ElementKind::Connection, None, 1, 1),
                linked(ElementKind::CoilOut, Some("%Q0"), 1, 0),
            ],
        )]);
        // Power injected in row 1 travels upwards through the link and drives
        // the coil placed above it.
        set_bit(&mut engine, "%I0", true);
        engine.scan_once(0);
        assert_eq!(get(&engine, "%Q0"), Some(Value::Bit(true)));

        // With the branch open the link carries nothing.
        set_bit(&mut engine, "%I0", false);
        engine.scan_once(10);
        assert_eq!(get(&engine, "%Q0"), Some(Value::Bit(false)));
    }

    // ---------------------------------------------------------------------
    // Compare and operate
    // ---------------------------------------------------------------------

    #[test]
    fn compare_accepts_one_expression_or_three_parameters() {
        let mut engine = engine(vec![
            rung(
                1,
                vec![
                    element_with(ElementKind::Compare, None, 0, 0, &["%MW0 > 5"]),
                    element(ElementKind::CoilOut, Some("%Q0"), 1, 0),
                ],
            ),
            rung(
                2,
                vec![
                    element_with(ElementKind::Compare, None, 0, 0, &["%MW1", "=", "7"]),
                    element(ElementKind::CoilOut, Some("%Q1"), 1, 0),
                ],
            ),
        ]);
        engine
            .store_mut()
            .set(&var("%MW0"), Value::Word(6))
            .expect("word is writable");
        engine
            .store_mut()
            .set(&var("%MW1"), Value::Word(7))
            .expect("word is writable");
        engine.scan_once(0);
        assert_eq!(get(&engine, "%Q0"), Some(Value::Bit(true)));
        assert_eq!(get(&engine, "%Q1"), Some(Value::Bit(true)));

        engine
            .store_mut()
            .set(&var("%MW0"), Value::Word(5))
            .expect("word is writable");
        engine
            .store_mut()
            .set(&var("%MW1"), Value::Word(8))
            .expect("word is writable");
        engine.scan_once(10);
        assert_eq!(get(&engine, "%Q0"), Some(Value::Bit(false)));
        assert_eq!(get(&engine, "%Q1"), Some(Value::Bit(false)));
    }

    #[test]
    fn compare_reports_a_bad_expression_without_panicking() {
        let mut engine = engine(vec![
            rung(
                1,
                vec![
                    element_with(ElementKind::Compare, None, 0, 0, &["%TM0.V >= 3"]),
                    element(ElementKind::CoilOut, Some("%Q0"), 1, 0),
                ],
            ),
            rung(
                2,
                vec![
                    element_with(ElementKind::Compare, None, 0, 0, &["1 / 0 = 1"]),
                    element(ElementKind::CoilOut, Some("%Q1"), 1, 0),
                ],
            ),
        ]);
        engine
            .store_mut()
            .set(&var("%TM0.V"), Value::Word(4))
            .expect("elapsed is writable");
        let report = engine.scan_once(0);
        assert_eq!(get(&engine, "%Q0"), Some(Value::Bit(true)));
        assert_eq!(get(&engine, "%Q1"), Some(Value::Bit(false)));
        assert!(has(&report, "SL-E002"), "division by zero is SL-E002");
    }

    #[test]
    fn operate_writes_only_when_the_rung_is_live() {
        let mut engine = engine(vec![rung(
            1,
            vec![
                element(ElementKind::ContactNo, Some("%I0"), 0, 0),
                element_with(ElementKind::Operate, None, 1, 0, &["%MW0", "%MW1 + 2"]),
                element(ElementKind::CoilOut, Some("%Q0"), 2, 0),
            ],
        )]);
        engine
            .store_mut()
            .set(&var("%MW1"), Value::Word(1))
            .expect("word is writable");
        engine.scan_once(0);
        assert_eq!(
            get(&engine, "%MW0"),
            Some(Value::Word(0)),
            "the rung was dead"
        );

        set_bit(&mut engine, "%I0", true);
        engine.scan_once(10);
        assert_eq!(get(&engine, "%MW0"), Some(Value::Word(3)));
        assert_eq!(get(&engine, "%Q0"), Some(Value::Bit(true)));
    }

    #[test]
    fn operate_ignores_a_leading_equals_and_parses_the_expression() {
        let mut engine = engine(vec![rung(
            1,
            vec![element_with(
                ElementKind::Operate,
                None,
                0,
                0,
                &["%MW0", "=", "%MW1", "*", "3"],
            )],
        )]);
        engine
            .store_mut()
            .set(&var("%MW1"), Value::Word(4))
            .expect("word is writable");
        engine.scan_once(0);
        assert_eq!(get(&engine, "%MW0"), Some(Value::Word(12)));
    }

    #[test]
    fn operate_reports_division_by_zero_without_panicking() {
        let mut engine = engine(vec![rung(
            1,
            vec![element_with(
                ElementKind::Operate,
                None,
                0,
                0,
                &["%MW0", "7 / 0"],
            )],
        )]);
        let report = engine.scan_once(0);
        assert!(has(&report, "SL-E002"));
        assert_eq!(get(&engine, "%MW0"), Some(Value::Word(0)));
    }

    #[test]
    fn operate_reports_a_bad_target_and_a_missing_expression() {
        let mut engine = engine(vec![rung(
            1,
            vec![
                element_with(
                    ElementKind::Operate,
                    None,
                    0,
                    0,
                    &["not-a-variable", "1 + 1"],
                ),
                element_with(ElementKind::Operate, None, 1, 0, &["%MW0"]),
            ],
        )]);
        let report = engine.scan_once(0);
        assert!(has(&report, "SL-E003"), "the target is not a variable");
        assert!(has(&report, "SL-E002"), "there is no expression");
    }

    // ---------------------------------------------------------------------
    // Function blocks driven through the engine
    // ---------------------------------------------------------------------

    #[test]
    fn engine_drives_a_ton_from_the_rail() {
        let mut engine = engine(vec![rung(
            1,
            vec![
                element(ElementKind::ContactNo, Some("%I0"), 0, 0),
                element_with(
                    ElementKind::Timer {
                        mode: TimerMode::On,
                    },
                    Some("%TM0"),
                    1,
                    0,
                    &["300"],
                ),
                element(ElementKind::CoilOut, Some("%Q0"), 2, 0),
            ],
        )]);
        engine.scan_once(0);
        // The preset parameter is a duration in milliseconds; with a 100 ms time
        // base it is three units, and `%TM0.P` reports units just like `%TM0.V`.
        assert_eq!(get(&engine, "%TM0.P"), Some(Value::Word(3)));
        assert_eq!(get(&engine, "%TM0.V"), Some(Value::Word(0)));

        set_bit(&mut engine, "%I0", true);
        for (now, units) in [(100u64, 1i32), (200, 2)] {
            engine.scan_once(now);
            assert_eq!(get(&engine, "%TM0.V"), Some(Value::Word(units)));
            assert_eq!(get(&engine, "%TM0"), Some(Value::Bit(false)));
        }
        engine.scan_once(300);
        assert_eq!(get(&engine, "%TM0.V"), Some(Value::Word(3)));
        assert_eq!(get(&engine, "%TM0"), Some(Value::Bit(true)));
        assert_eq!(get(&engine, "%Q0"), Some(Value::Bit(true)));

        // Dropping the input resets the timer immediately.
        set_bit(&mut engine, "%I0", false);
        engine.scan_once(400);
        assert_eq!(get(&engine, "%TM0.V"), Some(Value::Word(0)));
        assert_eq!(get(&engine, "%TM0"), Some(Value::Bit(false)));
    }

    #[test]
    fn engine_reads_a_timer_preset_from_a_variable() {
        let mut engine = engine(vec![rung(
            1,
            vec![
                element(ElementKind::ContactNo, Some("%I0"), 0, 0),
                element_with(
                    ElementKind::Timer {
                        mode: TimerMode::On,
                    },
                    Some("%TM0"),
                    1,
                    0,
                    &["%MW0"],
                ),
                element(ElementKind::CoilOut, Some("%Q0"), 2, 0),
            ],
        )]);
        // A preset variable is a duration in milliseconds; 300 ms is three units
        // of the 100 ms base.
        engine
            .store_mut()
            .set(&var("%MW0"), Value::Word(300))
            .expect("preset word is writable");
        set_bit(&mut engine, "%I0", true);
        engine.scan_once(0);
        assert_eq!(get(&engine, "%TM0.P"), Some(Value::Word(3)));
        for (now, units) in [(100u64, 1i32), (200, 2)] {
            engine.scan_once(now);
            assert_eq!(get(&engine, "%TM0.V"), Some(Value::Word(units)));
            assert_eq!(get(&engine, "%TM0"), Some(Value::Bit(false)));
        }
        engine.scan_once(300);
        assert_eq!(get(&engine, "%TM0.V"), Some(Value::Word(3)));
        assert_eq!(get(&engine, "%TM0"), Some(Value::Bit(true)));

        // The preset is re-read on every scan, and the output stays on while the
        // timer is done even if the new preset is below the elapsed value.
        engine
            .store_mut()
            .set(&var("%MW0"), Value::Word(100))
            .expect("preset word is writable");
        engine.scan_once(400);
        assert_eq!(get(&engine, "%TM0.P"), Some(Value::Word(1)));
        assert_eq!(get(&engine, "%TM0.V"), Some(Value::Word(3)));
        assert_eq!(get(&engine, "%TM0"), Some(Value::Bit(true)));
        // Dropping the input resets the timer, which then fires after the new,
        // shorter preset.
        set_bit(&mut engine, "%I0", false);
        engine.scan_once(500);
        assert_eq!(get(&engine, "%TM0"), Some(Value::Bit(false)));
        set_bit(&mut engine, "%I0", true);
        engine.scan_once(600);
        assert_eq!(get(&engine, "%TM0"), Some(Value::Bit(true)));
    }

    #[test]
    fn engine_timer_accepts_a_suffix_for_a_whole_second_base() {
        let mut engine = engine(vec![rung(
            1,
            vec![
                element(ElementKind::ContactNo, Some("%I0"), 0, 0),
                element_with(
                    ElementKind::Timer {
                        mode: TimerMode::On,
                    },
                    Some("%TM0"),
                    1,
                    0,
                    &["3s"],
                ),
                element(ElementKind::CoilOut, Some("%Q0"), 2, 0),
            ],
        )]);
        // One unit is a whole second, so `3s` is three units and the elapsed
        // value is reported in seconds.
        set_bit(&mut engine, "%I0", true);
        engine.scan_once(0);
        assert_eq!(get(&engine, "%TM0.P"), Some(Value::Word(3)));
        assert_eq!(get(&engine, "%TM0.V"), Some(Value::Word(0)));
        // The next gap is clamped to MAX_DELTA_MS, which is one whole second at
        // this base.
        engine.scan_once(2500);
        assert_eq!(get(&engine, "%TM0.V"), Some(Value::Word(1)));
        assert_eq!(get(&engine, "%TM0"), Some(Value::Bit(false)));
        // Two more whole-second scans reach the preset.
        engine.scan_once(3500);
        assert_eq!(get(&engine, "%TM0.V"), Some(Value::Word(2)));
        assert_eq!(get(&engine, "%TM0"), Some(Value::Bit(false)));
        engine.scan_once(4500);
        assert_eq!(get(&engine, "%TM0.V"), Some(Value::Word(3)));
        assert_eq!(get(&engine, "%TM0"), Some(Value::Bit(true)));
        assert_eq!(get(&engine, "%Q0"), Some(Value::Bit(true)));
    }

    #[test]
    fn engine_counter_counts_up_and_reports_done() {
        let mut engine = engine(vec![rung(
            1,
            vec![
                element(ElementKind::ContactNo, Some("%I0"), 0, 2),
                element_with(
                    ElementKind::Counter {
                        kind: CounterKind::Up,
                    },
                    Some("%C0"),
                    1,
                    0,
                    &["3"],
                ),
                element(ElementKind::CoilOut, Some("%Q0"), 2, 0),
            ],
        )]);
        for (now, pressed) in [
            (0u64, false),
            (10, true),
            (20, false),
            (30, true),
            (40, false),
            (50, true),
        ] {
            set_bit(&mut engine, "%I0", pressed);
            engine.scan_once(now);
        }
        assert_eq!(get(&engine, "%C0.V"), Some(Value::Word(3)));
        assert_eq!(get(&engine, "%C0"), Some(Value::Bit(true)));
        assert_eq!(get(&engine, "%Q0"), Some(Value::Bit(true)));
    }

    #[test]
    fn engine_counter_reset_and_preset_rows_are_level_triggered() {
        let mut engine = engine(vec![rung(
            1,
            vec![
                element(ElementKind::ContactNo, Some("%M0"), 0, 0),
                element(ElementKind::ContactNo, Some("%I2"), 1, 0),
                element(ElementKind::ContactNo, Some("%M0"), 0, 1),
                element(ElementKind::ContactNo, Some("%I1"), 1, 1),
                element(ElementKind::ContactNo, Some("%M0"), 0, 2),
                element(ElementKind::ContactNo, Some("%I0"), 1, 2),
                element_with(
                    ElementKind::Counter {
                        kind: CounterKind::Up,
                    },
                    Some("%C0"),
                    2,
                    0,
                    &["2"],
                ),
            ],
        )]);
        set_bit(&mut engine, "%M0", true);
        set_bit(&mut engine, "%I0", true);
        engine.scan_once(0);
        assert_eq!(get(&engine, "%C0.V"), Some(Value::Word(1)));

        // Loading the preset moves the count straight to it.
        set_bit(&mut engine, "%I1", true);
        engine.scan_once(10);
        assert_eq!(get(&engine, "%C0.V"), Some(Value::Word(2)));
        assert_eq!(get(&engine, "%C0"), Some(Value::Bit(true)));

        // Reset wins over the edges in the same scan.
        set_bit(&mut engine, "%I2", true);
        engine.scan_once(20);
        assert_eq!(get(&engine, "%C0.V"), Some(Value::Word(0)));
        assert_eq!(get(&engine, "%C0"), Some(Value::Bit(false)));
    }

    #[test]
    fn engine_register_pushes_and_pops() {
        let mut engine = engine(vec![rung(
            1,
            vec![
                element(ElementKind::ContactNo, Some("%M0"), 0, 0),
                element(ElementKind::ContactNo, Some("%I2"), 1, 0),
                element(ElementKind::ContactNo, Some("%M0"), 0, 1),
                element(ElementKind::ContactNo, Some("%I0"), 1, 1),
                element(ElementKind::ContactNo, Some("%M0"), 0, 2),
                element(ElementKind::ContactNo, Some("%I1"), 1, 2),
                element_with(
                    ElementKind::Register {
                        mode: RegisterMode::Fifo,
                    },
                    Some("%R0"),
                    2,
                    0,
                    &["3"],
                ),
                element(ElementKind::CoilOut, Some("%Q0"), 3, 0),
            ],
        )]);
        set_bit(&mut engine, "%M0", true);
        // The block output is the empty flag, which is true while nothing is
        // stored.
        engine.scan_once(0);
        assert_eq!(get(&engine, "%Q0"), Some(Value::Bit(true)));
        assert_eq!(get(&engine, "%R0.S"), Some(Value::Word(0)));

        engine
            .store_mut()
            .set(&var("%R0.I"), Value::Word(77))
            .expect("register input is writable");
        set_bit(&mut engine, "%I0", true);
        engine.scan_once(10);
        assert_eq!(get(&engine, "%R0.S"), Some(Value::Word(1)));
        assert_eq!(get(&engine, "%Q0"), Some(Value::Bit(false)));

        set_bit(&mut engine, "%I0", false);
        set_bit(&mut engine, "%I1", true);
        engine.scan_once(20);
        assert_eq!(get(&engine, "%R0.S"), Some(Value::Word(0)));
        assert_eq!(get(&engine, "%R0.O"), Some(Value::Word(77)));

        // Popping an empty buffer is a no-op, not an error.
        let report = engine.scan_once(30);
        assert_eq!(get(&engine, "%R0.S"), Some(Value::Word(0)));
        assert!(!has(&report, "SL-E001"));
    }

    #[test]
    fn a_counter_block_occupies_four_rows_and_reads_each_of_them() {
        // The count-up row is row 2; a connection in row 3 must not leak into
        // it and the block must not be evaluated once per row.
        let mut engine = engine(vec![rung(
            1,
            vec![
                element(ElementKind::ContactNo, Some("%I0"), 0, 2),
                linked(ElementKind::Connection, None, 1, 3),
                element_with(
                    ElementKind::Counter {
                        kind: CounterKind::Up,
                    },
                    Some("%C0"),
                    2,
                    0,
                    &["5"],
                ),
            ],
        )]);
        set_bit(&mut engine, "%I0", true);
        engine.scan_once(0);
        assert_eq!(get(&engine, "%C0.V"), Some(Value::Word(1)));
        engine.scan_once(10);
        assert_eq!(
            get(&engine, "%C0.V"),
            Some(Value::Word(1)),
            "one count per scan at most"
        );
    }

    // ---------------------------------------------------------------------
    // Jumps and calls
    // ---------------------------------------------------------------------

    /// Three rungs; the first can jump over the second one.
    fn jump_project() -> Project {
        single_section(vec![
            rung(
                1,
                vec![
                    element(ElementKind::ContactNo, Some("%M0"), 0, 0),
                    element(ElementKind::ContactNo, Some("%I0"), 1, 0),
                    element_with(ElementKind::CoilJump, None, 2, 0, &["2"]),
                    element(ElementKind::CoilOut, Some("%Q0"), 3, 0),
                ],
            ),
            rung(
                2,
                vec![
                    element(ElementKind::ContactNo, Some("%M0"), 0, 0),
                    element(ElementKind::CoilOut, Some("%Q1"), 1, 0),
                ],
            ),
            rung(
                3,
                vec![
                    element(ElementKind::ContactNo, Some("%M0"), 0, 0),
                    element(ElementKind::CoilOut, Some("%Q2"), 1, 0),
                ],
            ),
        ])
    }

    #[test]
    fn a_jump_aborts_the_rest_of_its_rung_and_continues_at_the_target() {
        let mut engine = ScanEngine::new(jump_project());
        set_bit(&mut engine, "%M0", true);
        set_bit(&mut engine, "%I0", true);
        let report = engine.scan_once(0);
        // The target is rung index 2, so `%Q0` (after the jump) and `%Q1` (in
        // the skipped rung 2) are never driven, while `%Q2` runs.
        assert_eq!(get(&engine, "%Q0"), Some(Value::Bit(false)));
        assert_eq!(get(&engine, "%Q1"), Some(Value::Bit(false)));
        assert_eq!(get(&engine, "%Q2"), Some(Value::Bit(true)));
        assert_eq!(report.jumps, 1);
        assert!(report.diagnostics.is_empty(), "no diagnostics expected");

        // Without the jump the whole section runs: the rung that was skipped
        // drives `%Q1` again.
        set_bit(&mut engine, "%I0", false);
        let report = engine.scan_once(10);
        assert_eq!(get(&engine, "%Q1"), Some(Value::Bit(true)));
        assert_eq!(get(&engine, "%Q2"), Some(Value::Bit(true)));
        assert_eq!(report.jumps, 0);
    }

    #[test]
    fn a_jump_target_may_be_a_rung_label() {
        let mut project = jump_project();
        if let Some(rung) = project.rung_mut(3) {
            rung.label = "last".to_owned();
        }
        if let Some(element) = project
            .rung_mut(1)
            .and_then(|rung| rung.elements.get_mut(2))
        {
            element.params = vec!["last".to_owned()];
        }
        let mut engine = ScanEngine::new(project);
        set_bit(&mut engine, "%M0", true);
        set_bit(&mut engine, "%I0", true);
        let report = engine.scan_once(0);
        assert_eq!(get(&engine, "%Q0"), Some(Value::Bit(false)));
        assert_eq!(
            get(&engine, "%Q1"),
            Some(Value::Bit(false)),
            "rung 2 skipped"
        );
        assert_eq!(get(&engine, "%Q2"), Some(Value::Bit(true)));
        assert_eq!(report.jumps, 1);

        // A label that matches nothing is reported and the jump is ignored.
        let mut broken = jump_project();
        if let Some(element) = broken.rung_mut(1).and_then(|rung| rung.elements.get_mut(2)) {
            element.params = vec!["nowhere".to_owned()];
        }
        let mut engine = ScanEngine::new(broken);
        set_bit(&mut engine, "%M0", true);
        set_bit(&mut engine, "%I0", true);
        let report = engine.scan_once(0);
        assert!(has(&report, "SL-E005"));
        // The jump was ignored, so the rest of the rung still ran.
        assert_eq!(get(&engine, "%Q0"), Some(Value::Bit(true)));
        assert_eq!(report.jumps, 0);
    }

    #[test]
    fn an_out_of_range_jump_index_is_reported_and_ignored() {
        let mut project = jump_project();
        if let Some(element) = project
            .rung_mut(1)
            .and_then(|rung| rung.elements.get_mut(2))
        {
            element.params = vec!["9".to_owned()];
        }
        let mut engine = ScanEngine::new(project);
        set_bit(&mut engine, "%M0", true);
        set_bit(&mut engine, "%I0", true);
        let report = engine.scan_once(0);
        assert!(has(&report, "SL-E005"));
        assert_eq!(get(&engine, "%Q0"), Some(Value::Bit(true)));
        assert_eq!(report.jumps, 0);
    }

    #[test]
    fn a_mad_loop_is_detected_and_stops_the_scan() {
        let mut engine = engine(vec![
            rung(
                1,
                vec![
                    element(ElementKind::ContactNo, Some("%M0"), 0, 0),
                    element_with(ElementKind::CoilJump, None, 1, 0, &["0"]),
                ],
            ),
            rung(
                2,
                vec![
                    element(ElementKind::ContactNo, Some("%M0"), 0, 0),
                    element(ElementKind::CoilOut, Some("%Q0"), 1, 0),
                ],
            ),
        ]);
        set_bit(&mut engine, "%M0", true);
        let report = engine.scan_once(0);
        assert!(report.stopped_for_mad_loop);
        assert!(has(&report, "SL-E006"));
        assert!(report.jumps > MAD_LOOP_LIMIT);
        // The section after the runaway one is not evaluated.
        assert_eq!(get(&engine, "%Q0"), Some(Value::Bit(false)));
    }

    /// A project whose main section calls subroutine 7 once per call coil.
    fn call_project(calls: usize) -> Project {
        let mut main_elements = vec![element(ElementKind::ContactNo, Some("%M0"), 0, 0)];
        for index in 0..calls {
            let col = u8::try_from(index + 1).unwrap_or(1);
            main_elements.push(element_with(ElementKind::CoilCall, None, col, 0, &["7"]));
        }
        let main = Section {
            rungs: vec![1],
            ..Section::new(1, "Main")
        };
        let subroutine = Section {
            rungs: vec![2],
            subroutine: Some(7),
            ..Section::new(2, "Sub")
        };
        project(
            vec![main, subroutine],
            vec![
                rung(1, main_elements),
                rung(
                    2,
                    vec![
                        element(ElementKind::ContactNo, Some("%M0"), 0, 0),
                        element_with(ElementKind::Operate, None, 1, 0, &["%MW0", "%MW0 + 1"]),
                    ],
                ),
            ],
        )
    }

    #[test]
    fn a_subroutine_runs_once_per_call_coil() {
        let mut engine = ScanEngine::new(call_project(1));
        set_bit(&mut engine, "%M0", true);
        let report = engine.scan_once(0);
        assert_eq!(get(&engine, "%MW0"), Some(Value::Word(1)));
        assert_eq!(report.calls, 1);
        assert!(report.diagnostics.is_empty());

        // A second call in the same scan executes the subroutine again.
        let mut twice = ScanEngine::new(call_project(2));
        set_bit(&mut twice, "%M0", true);
        let report = twice.scan_once(0);
        assert_eq!(get(&twice, "%MW0"), Some(Value::Word(2)));
        assert_eq!(report.calls, 2);

        // The subroutine never runs on its own.
        let mut idle = ScanEngine::new(call_project(0));
        set_bit(&mut idle, "%M0", true);
        idle.scan_once(0);
        assert_eq!(get(&idle, "%MW0"), Some(Value::Word(0)));
    }

    #[test]
    fn a_call_to_an_undefined_section_is_reported_and_ignored() {
        let main = Section {
            rungs: vec![1],
            ..Section::new(1, "Main")
        };
        let other = Section {
            rungs: vec![2],
            ..Section::new(2, "NotASubroutine")
        };
        let mut engine = ScanEngine::new(project(
            vec![main, other],
            vec![
                rung(
                    1,
                    vec![
                        element(ElementKind::ContactNo, Some("%M0"), 0, 0),
                        element_with(ElementKind::CoilCall, None, 1, 0, &["7"]),
                        element(ElementKind::CoilOut, Some("%Q0"), 2, 0),
                    ],
                ),
                rung(
                    2,
                    vec![
                        element(ElementKind::ContactNo, Some("%M0"), 0, 0),
                        element(ElementKind::CoilOut, Some("%Q1"), 1, 0),
                    ],
                ),
            ],
        ));
        set_bit(&mut engine, "%M0", true);
        let report = engine.scan_once(0);
        assert!(has(&report, "SL-E007"));
        // The call was ignored and the rest of the rung still ran.
        assert_eq!(get(&engine, "%Q0"), Some(Value::Bit(true)));
        assert_eq!(report.calls, 0);
    }

    #[test]
    fn call_depth_overflow_is_reported_and_abandons_the_rung() {
        // More subroutines than the frame limit, each calling the next.
        let subroutines = 30u32;
        let mut sections = vec![Section {
            rungs: vec![1],
            ..Section::new(1, "Main")
        }];
        for number in 0..subroutines {
            sections.push(Section {
                rungs: vec![number + 2],
                subroutine: Some(number),
                ..Section::new(number + 2, format!("Sub{number}"))
            });
        }
        let mut rungs = vec![rung(
            1,
            vec![
                element(ElementKind::ContactNo, Some("%M0"), 0, 0),
                element_with(ElementKind::CoilCall, None, 1, 0, &["0"]),
                element(ElementKind::CoilOut, Some("%Q0"), 2, 0),
            ],
        )];
        for number in 0..subroutines {
            rungs.push(rung(
                number + 2,
                vec![
                    element(ElementKind::ContactNo, Some("%M0"), 0, 0),
                    element_with(
                        ElementKind::CoilCall,
                        None,
                        1,
                        0,
                        &[&format!("{}", number + 1)],
                    ),
                ],
            ));
        }
        let mut engine = ScanEngine::new(project(sections, rungs));
        set_bit(&mut engine, "%M0", true);
        let report = engine.scan_once(0);
        assert!(has(&report, "SL-E008"), "{:?}", codes(&report));
        assert_eq!(report.calls, MAX_CALL_DEPTH as u64);
        // The frame that overflowed abandoned its rung, and every caller up the
        // chain abandoned its own rung too, so the coil after the call in the
        // main rung was never reached.
        assert_eq!(
            get(&engine, "%Q0"),
            Some(Value::Bit(false)),
            "calls={} diags={:?}",
            report.calls,
            codes(&report)
        );
    }

    // ---------------------------------------------------------------------
    // Scan structure
    // ---------------------------------------------------------------------

    #[test]
    fn a_section_referencing_a_missing_rung_is_skipped() {
        let main = Section {
            rungs: vec![99, 1],
            ..Section::new(1, "Main")
        };
        let mut engine = ScanEngine::new(project(
            vec![main],
            vec![rung(
                1,
                vec![
                    element(ElementKind::ContactNo, Some("%M0"), 0, 0),
                    element(ElementKind::CoilOut, Some("%Q0"), 1, 0),
                ],
            )],
        ));
        set_bit(&mut engine, "%M0", true);
        let report = engine.scan_once(0);
        assert_eq!(
            get(&engine, "%Q0"),
            Some(Value::Bit(true)),
            "M0={:?} diags={:?}",
            get(&engine, "%M0"),
            codes(&report)
        );
    }

    #[test]
    fn sfc_sections_are_skipped_with_a_warning() {
        let sfc = Section {
            language: SectionLanguage::Sfc,
            rungs: vec![1],
            ..Section::new(2, "Chart")
        };
        let mut engine = ScanEngine::new(project(
            vec![sfc],
            vec![rung(
                1,
                vec![
                    element(ElementKind::ContactNo, Some("%M0"), 0, 0),
                    element(ElementKind::CoilOut, Some("%Q0"), 1, 0),
                ],
            )],
        ));
        set_bit(&mut engine, "%M0", true);
        let report = engine.scan_once(0);
        assert!(has(&report, "SL-W002"));
        assert_eq!(get(&engine, "%Q0"), Some(Value::Bit(false)));
    }

    #[test]
    fn main_sections_run_in_declaration_order() {
        let first = Section {
            rungs: vec![1],
            ..Section::new(1, "First")
        };
        let second = Section {
            rungs: vec![2],
            ..Section::new(2, "Second")
        };
        let mut engine = ScanEngine::new(project(
            vec![first, second],
            vec![
                rung(
                    1,
                    vec![
                        element(ElementKind::ContactNo, Some("%M0"), 0, 0),
                        element(ElementKind::CoilOut, Some("%M1"), 1, 0),
                    ],
                ),
                rung(
                    2,
                    vec![
                        // Reads the bit written by the first section.
                        element(ElementKind::ContactNo, Some("%M1"), 0, 0),
                        element(ElementKind::CoilOut, Some("%Q0"), 1, 0),
                    ],
                ),
            ],
        ));
        set_bit(&mut engine, "%M0", true);
        engine.scan_once(0);
        assert_eq!(get(&engine, "%Q0"), Some(Value::Bit(true)));
    }

    #[test]
    fn the_first_scan_charges_no_elapsed_time() {
        let mut engine = engine(vec![rung(
            1,
            vec![
                element(ElementKind::ContactNo, Some("%M0"), 0, 0),
                element_with(
                    ElementKind::Timer {
                        mode: TimerMode::On,
                    },
                    Some("%TM0"),
                    1,
                    0,
                    &["2000"],
                ),
            ],
        )]);
        set_bit(&mut engine, "%M0", true);
        // A huge first timestamp must not charge the timer a huge delta.
        engine.scan_once(1_000_000);
        assert_eq!(get(&engine, "%TM0.V"), Some(Value::Word(0)));
        // The next scan is clamped to MAX_DELTA_MS, which is ten units of the
        // 100 ms base and not the whole day that actually elapsed.
        engine.scan_once(9_000_000);
        assert_eq!(get(&engine, "%TM0.V"), Some(Value::Word(10)));
        assert_eq!(get(&engine, "%TM0"), Some(Value::Bit(false)));
        // Each further clamped scan is worth exactly ten units, and the preset
        // of twenty units is reached without ever seeing the real gap.
        engine.scan_once(9_001_000);
        assert_eq!(get(&engine, "%TM0.V"), Some(Value::Word(20)));
        assert_eq!(get(&engine, "%TM0"), Some(Value::Bit(true)));
    }

    #[test]
    fn a_huge_scan_gap_is_clamped_before_it_reaches_a_block() {
        let mut engine = engine(vec![rung(
            1,
            vec![
                element(ElementKind::ContactNo, Some("%M0"), 0, 0),
                element_with(
                    ElementKind::Timer {
                        mode: TimerMode::On,
                    },
                    Some("%TM0"),
                    1,
                    0,
                    &["30000"],
                ),
            ],
        )]);
        set_bit(&mut engine, "%M0", true);
        engine.scan_once(0);
        // One scan may only charge MAX_DELTA_MS milliseconds of wall clock time,
        // which is ten units of the 100 ms base, so a clock that jumps by a day
        // advances the timer by ten units and not by 864000.
        engine.scan_once(86_400_000);
        assert_eq!(get(&engine, "%TM0.V"), Some(Value::Word(10)));
        assert_eq!(get(&engine, "%TM0"), Some(Value::Bit(false)));

        // The preset is 300 units, so it takes thirty clamped scans to fill it:
        // a clock that jumps by a whole day cannot fill it in one step.
        let mut now = 86_400_000u64;
        for units in 2..=30i32 {
            now += MAX_DELTA_MS;
            engine.scan_once(now);
            assert_eq!(get(&engine, "%TM0.V"), Some(Value::Word(units * 10)));
            assert_eq!(
                get(&engine, "%TM0"),
                Some(Value::Bit(units == 30)),
                "done after {units} clamped scans"
            );
        }
    }

    #[test]
    fn a_project_edit_is_picked_up_on_the_next_scan() {
        let mut engine = engine(vec![rung(
            1,
            vec![
                element(ElementKind::ContactNo, Some("%M0"), 0, 0),
                element(ElementKind::CoilOut, Some("%Q0"), 1, 0),
            ],
        )]);
        set_bit(&mut engine, "%M0", true);
        engine.scan_once(0);
        assert_eq!(get(&engine, "%Q0"), Some(Value::Bit(true)));

        // Swap the element: the cell index must be rebuilt automatically.
        let project = engine.project_mut();
        if let Some(rung) = project.rung_mut(1) {
            rung.elements = vec![
                element(ElementKind::ContactNo, Some("%M0"), 0, 0),
                element(ElementKind::CoilOut, Some("%Q1"), 1, 0),
            ];
        }
        engine.scan_once(10);
        assert_eq!(get(&engine, "%Q1"), Some(Value::Bit(true)));
        assert_eq!(get(&engine, "%Q0"), Some(Value::Bit(true)), "stale output");
    }

    // ---------------------------------------------------------------------
    // Diagnostics raised by the engine
    // ---------------------------------------------------------------------

    #[test]
    fn a_word_used_as_a_contact_reports_a_kind_mismatch() {
        let mut engine = engine(vec![rung(
            1,
            vec![
                element(ElementKind::ContactNo, Some("%MW0"), 0, 0),
                element(ElementKind::ContactNo, Some("%MW0"), 1, 0),
                element(ElementKind::CoilOut, Some("%Q0"), 2, 0),
            ],
        )]);
        let report = engine.scan_once(0);
        assert!(has(&report, "SL-E003"), "{:?}", codes(&report));
        assert_eq!(get(&engine, "%Q0"), Some(Value::Bit(false)));
    }

    #[test]
    fn a_word_used_as_a_coil_reports_a_kind_mismatch() {
        let mut engine = engine(vec![rung(
            1,
            vec![
                element(ElementKind::ContactNo, Some("%I0"), 0, 0),
                element(ElementKind::CoilOut, Some("%MW1"), 1, 0),
            ],
        )]);
        set_bit(&mut engine, "%I0", true);
        let report = engine.scan_once(0);
        assert!(has(&report, "SL-E003"), "{:?}", codes(&report));
        assert_eq!(get(&engine, "%MW1"), Some(Value::Word(0)));
    }

    #[test]
    fn an_element_without_its_variable_reports_sl_e004() {
        let mut engine = engine(vec![rung(
            1,
            vec![element(ElementKind::ContactNo, None, 0, 0)],
        )]);
        let report = engine.scan_once(0);
        assert!(has(&report, "SL-E004"));
    }

    #[test]
    fn a_block_without_its_variable_reports_sl_e004_once_per_block() {
        let mut engine = ScanEngine::new(project(
            vec![Section {
                rungs: vec![1, 2, 3],
                ..Section::new(1, "Main")
            }],
            vec![
                rung(
                    1,
                    vec![
                        element(ElementKind::ContactNo, Some("%M0"), 0, 0),
                        element_with(
                            ElementKind::Timer {
                                mode: TimerMode::On,
                            },
                            None,
                            1,
                            0,
                            &["100"],
                        ),
                    ],
                ),
                rung(
                    2,
                    vec![
                        element(ElementKind::ContactNo, Some("%M0"), 0, 0),
                        element_with(
                            ElementKind::Counter {
                                kind: CounterKind::Up,
                            },
                            None,
                            1,
                            0,
                            &["1"],
                        ),
                    ],
                ),
                rung(
                    3,
                    vec![
                        element(ElementKind::ContactNo, Some("%M0"), 0, 0),
                        element_with(
                            ElementKind::Register {
                                mode: RegisterMode::Fifo,
                            },
                            None,
                            1,
                            0,
                            &["1"],
                        ),
                    ],
                ),
            ],
        ));
        engine
            .store_mut()
            .set(&var("%M0"), Value::Bit(true))
            .expect("bit can be set");
        let report = engine.scan_once(0);
        assert_eq!(
            report
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == "SL-E004")
                .count(),
            3,
            "one per block: {:?}",
            codes(&report)
        );
    }

    #[test]
    fn a_block_with_a_variable_of_the_wrong_kind_reports_sl_e003() {
        let mut engine = engine(vec![rung(
            1,
            vec![
                element(ElementKind::ContactNo, Some("%M0"), 0, 0),
                element_with(
                    ElementKind::Timer {
                        mode: TimerMode::On,
                    },
                    Some("%C0"),
                    1,
                    0,
                    &["100"],
                ),
            ],
        )]);
        set_bit(&mut engine, "%M0", true);
        let report = engine.scan_once(0);
        assert!(has(&report, "SL-E003"));
    }

    // ---------------------------------------------------------------------
    // Lint
    // ---------------------------------------------------------------------

    #[test]
    fn lint_reports_duplicate_cells() {
        let project = single_section(vec![rung(
            1,
            vec![
                element(ElementKind::ContactNo, Some("%I0"), 1, 0),
                element(ElementKind::ContactNc, Some("%I1"), 1, 0),
            ],
        )]);
        let diagnostics = lint(&project);
        assert!(diagnostics.iter().any(|d| d.code == "SL-E009"));
    }

    #[test]
    fn lint_reports_unresolvable_jump_targets() {
        let project = single_section(vec![
            rung(
                1,
                vec![element_with(ElementKind::CoilJump, None, 0, 0, &["9"])],
            ),
            rung(
                2,
                vec![element_with(
                    ElementKind::CoilJump,
                    None,
                    0,
                    0,
                    &["missing"],
                )],
            ),
            rung(
                3,
                vec![
                    element_with(ElementKind::CoilJump, None, 0, 0, &["1"]),
                    element_with(ElementKind::CoilJump, None, 1, 0, &["9"]),
                    element(ElementKind::CoilJump, None, 2, 0),
                ],
            ),
        ]);
        let diagnostics = lint(&project);
        let jumps: Vec<&Diagnostic> = diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "SL-E005")
            .collect();
        assert_eq!(jumps.len(), 3, "two bad indices and one missing target");
        assert_eq!(jumps[0].rung, Some(0));
        assert_eq!(jumps[1].rung, Some(1));
    }

    #[test]
    fn lint_reports_calls_to_non_subroutines() {
        let main = Section {
            rungs: vec![1],
            ..Section::new(1, "Main")
        };
        let plain = Section {
            rungs: vec![2],
            ..Section::new(2, "Plain")
        };
        let subroutine = Section {
            rungs: vec![3],
            subroutine: Some(4),
            ..Section::new(3, "Sub")
        };
        let project = project(
            vec![main, plain, subroutine],
            vec![
                rung(
                    1,
                    vec![
                        element_with(ElementKind::CoilCall, None, 0, 0, &["4"]),
                        element_with(ElementKind::CoilCall, None, 1, 0, &["2"]),
                        element_with(ElementKind::CoilCall, None, 2, 0, &["7"]),
                        element_with(ElementKind::CoilCall, None, 3, 0, &["nope"]),
                    ],
                ),
                rung(2, vec![element(ElementKind::Connection, None, 0, 0)]),
                rung(3, vec![element(ElementKind::Connection, None, 0, 0)]),
            ],
        );
        let diagnostics = lint(&project);
        let calls: Vec<&Diagnostic> = diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "SL-E007")
            .collect();
        assert_eq!(
            calls.len(),
            3,
            "section 2 is not a subroutine, 7 is missing and `nope` is not a number"
        );
    }

    #[test]
    fn lint_reports_sfc_sections_and_missing_or_mismatched_variables() {
        let chart = Section {
            language: SectionLanguage::Sfc,
            rungs: vec![],
            ..Section::new(1, "Chart")
        };
        let main = Section {
            rungs: vec![1, 2],
            ..Section::new(2, "Main")
        };
        let project = project(
            vec![chart, main],
            vec![
                rung(1, vec![element(ElementKind::ContactNo, None, 0, 0)]),
                rung(
                    2,
                    vec![element_with(
                        ElementKind::Timer {
                            mode: TimerMode::On,
                        },
                        Some("%M0"),
                        0,
                        0,
                        &["1"],
                    )],
                ),
            ],
        );
        let diagnostics = lint(&project);
        assert!(diagnostics.iter().any(|d| d.code == "SL-W002"));
        assert!(diagnostics.iter().any(|d| d.code == "SL-E004"));
        assert!(diagnostics.iter().any(|d| d.code == "SL-E003"));
    }

    #[test]
    fn lint_reports_rows_that_cannot_reach_the_left_rail() {
        let project = single_section(vec![
            rung(1, vec![element(ElementKind::CoilOut, Some("%Q0"), 1, 0)]),
            rung(
                2,
                vec![
                    element(ElementKind::Connection, None, 0, 0),
                    element(ElementKind::CoilOut, Some("%Q1"), 1, 0),
                ],
            ),
        ]);
        let diagnostics = lint(&project);
        let warnings: Vec<&Diagnostic> = diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "SL-W001")
            .collect();
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].rung, Some(0));
    }

    #[test]
    fn lint_accepts_a_clean_project() {
        let project = single_section(vec![
            rung(
                1,
                vec![
                    element(ElementKind::ContactNo, Some("%I0"), 0, 0),
                    element(ElementKind::CoilOut, Some("%Q0"), 1, 0),
                ],
            ),
            rung(
                2,
                vec![
                    element(ElementKind::ContactNo, Some("%I0"), 0, 0),
                    element_with(ElementKind::CoilJump, None, 1, 0, &["0"]),
                ],
            ),
        ]);
        assert!(lint(&project).is_empty(), "{:?}", lint(&project));
    }

    // ---------------------------------------------------------------------
    // Determinism
    // ---------------------------------------------------------------------

    fn determinism_project() -> Project {
        let main = Section {
            rungs: vec![1, 2, 3],
            ..Section::new(1, "Main")
        };
        let subroutine = Section {
            rungs: vec![4],
            subroutine: Some(3),
            ..Section::new(2, "Sub")
        };
        project(
            vec![main, subroutine],
            vec![
                rung(
                    1,
                    vec![
                        element(ElementKind::ContactNo, Some("%I0"), 0, 0),
                        element(ElementKind::ContactRising, Some("%I1"), 1, 0),
                        element_with(
                            ElementKind::Timer {
                                mode: TimerMode::On,
                            },
                            Some("%TM0"),
                            2,
                            0,
                            &["300"],
                        ),
                        element(ElementKind::CoilOut, Some("%Q0"), 3, 0),
                    ],
                ),
                rung(
                    2,
                    vec![
                        element(ElementKind::ContactNo, Some("%M0"), 0, 0),
                        element_with(
                            ElementKind::Counter {
                                kind: CounterKind::Up,
                            },
                            Some("%C0"),
                            1,
                            0,
                            &["3"],
                        ),
                        element(ElementKind::CoilOut, Some("%M1"), 2, 0),
                    ],
                ),
                rung(
                    3,
                    vec![
                        element(ElementKind::ContactNo, Some("%M1"), 0, 0),
                        element_with(ElementKind::CoilCall, None, 1, 0, &["3"]),
                        element_with(ElementKind::Compare, None, 2, 0, &["%MW0 >= 3"]),
                        element(ElementKind::CoilOut, Some("%Q1"), 3, 0),
                    ],
                ),
                rung(
                    4,
                    vec![
                        element(ElementKind::ContactNo, Some("%M0"), 0, 0),
                        element_with(ElementKind::Operate, None, 1, 0, &["%MW0", "%MW0 + 1"]),
                    ],
                ),
            ],
        )
    }

    fn drive(engine: &mut ScanEngine) {
        engine
            .store_mut()
            .set(&var("%M0"), Value::Bit(true))
            .expect("bit can be set");
        for cycle in 0..60u64 {
            let now = cycle * 37;
            let first = cycle % 7 == 0;
            let second = cycle % 11 == 0;
            engine
                .store_mut()
                .set(&var("%I0"), Value::Bit(first))
                .expect("input is writable");
            engine
                .store_mut()
                .set(&var("%I1"), Value::Bit(second))
                .expect("input is writable");
            engine.scan_once(now);
        }
    }

    #[test]
    fn two_engines_scanning_the_same_inputs_stay_identical() {
        let mut first = ScanEngine::new(determinism_project());
        let mut second = ScanEngine::new(determinism_project());
        drive(&mut first);
        drive(&mut second);
        assert_eq!(first.store(), second.store());
        assert_eq!(first.project(), second.project());
        assert_eq!(first.cycles(), second.cycles());

        // And the state is exactly what a third, uninterrupted run produces.
        let mut third = ScanEngine::new(determinism_project());
        drive(&mut third);
        assert_eq!(first.store(), third.store());
    }

    #[test]
    fn a_scan_depends_only_on_the_store_and_now_ms() {
        let mut first = ScanEngine::new(determinism_project());
        let mut second = ScanEngine::new(determinism_project());
        first
            .store_mut()
            .set(&var("%M0"), Value::Bit(true))
            .expect("bit can be set");
        second
            .store_mut()
            .set(&var("%M0"), Value::Bit(true))
            .expect("bit can be set");
        for (cycle, now) in [(0u64, 0u64), (1, 10), (2, 5000), (3, 5010)] {
            first.scan_once(now);
            second.scan_once(now);
            assert_eq!(
                first.store(),
                second.store(),
                "cycle {cycle} diverged at now = {now}"
            );
        }
    }

    #[test]
    fn scan_report_counts_cycles_and_diagnostics() {
        let mut engine = engine(vec![rung(
            1,
            vec![element(ElementKind::ContactNo, None, 0, 0)],
        )]);
        let first = engine.scan_once(0);
        assert_eq!(first.cycles, 1);
        let second = engine.scan_once(10);
        assert_eq!(second.cycles, 2);
        assert_eq!(engine.cycles(), 2);
        assert!(!second.stopped_for_mad_loop);
        assert_eq!(second.jumps, 0);
        assert_eq!(second.calls, 0);
        assert!(has(&second, "SL-E004"));
    }
    #[test]
    fn a_series_element_in_the_merge_column_is_not_bypassed() {
        // The classic start/stop seal: %I0 starts, the normally-closed %I2 stops,
        // and %Q0 holds itself in through the row below. The vertical link sits
        // in the same column as the stop contact, so the contact's input is the
        // OR of both branches — and its output, not the raw branch power, is what
        // reaches the coil. Regression test: when links were stored per row
        // instead of per column, the merge leaked one column to the right and the
        // stop button could not break the seal.
        let mut engine = engine(vec![rung(
            1,
            vec![
                element(ElementKind::ContactNo, Some("%I0"), 0, 0),
                element(ElementKind::ContactNc, Some("%I2"), 1, 0),
                element(ElementKind::ContactNo, Some("%Q0"), 0, 1),
                linked(ElementKind::Connection, None, 1, 1),
                element(ElementKind::CoilOut, Some("%Q0"), 2, 0),
            ],
        )]);

        // Start pressed: the seal closes.
        set_bit(&mut engine, "%I0", true);
        engine.scan_once(0);
        assert_eq!(get(&engine, "%Q0"), Some(Value::Bit(true)));

        // Start released: the lamp stays on through its own contact.
        set_bit(&mut engine, "%I0", false);
        engine.scan_once(10);
        assert_eq!(
            get(&engine, "%Q0"),
            Some(Value::Bit(true)),
            "the seal holds without the start button"
        );

        // Stop pressed: the normally-closed contact opens and breaks the seal.
        set_bit(&mut engine, "%I2", true);
        engine.scan_once(20);
        assert_eq!(
            get(&engine, "%Q0"),
            Some(Value::Bit(false)),
            "the stop button must break the seal"
        );

        // Releasing stop does not restart the lamp: start is still open.
        set_bit(&mut engine, "%I2", false);
        engine.scan_once(30);
        assert_eq!(get(&engine, "%Q0"), Some(Value::Bit(false)));
    }

    #[test]
    fn a_vertical_link_only_merges_its_own_column() {
        // Row 0 and row 1 are joined in column 1, but column 2 carries a
        // normally-open contact in row 0. The contact must see only its own row's
        // power, so the coil stays off until that contact is closed.
        let mut engine = engine(vec![rung(
            1,
            vec![
                element(ElementKind::ContactNo, Some("%I0"), 0, 0),
                element(ElementKind::ContactNo, Some("%I1"), 0, 1),
                linked(ElementKind::Connection, None, 1, 1),
                element(ElementKind::ContactNo, Some("%I3"), 2, 0),
                element(ElementKind::CoilOut, Some("%Q0"), 3, 0),
            ],
        )]);

        // Only the lower branch is live, and %I3 is open: no path to the coil.
        set_bit(&mut engine, "%I1", true);
        engine.scan_once(0);
        assert_eq!(get(&engine, "%Q0"), Some(Value::Bit(false)));

        // Closing %I3, which sits to the right of the link, completes the path.
        set_bit(&mut engine, "%I3", true);
        engine.scan_once(10);
        assert_eq!(get(&engine, "%Q0"), Some(Value::Bit(true)));
    }
}
