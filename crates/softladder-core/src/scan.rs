//! Deterministic scan engine, variable store and function-block state.
//!
//! The engine never reads the clock: the caller passes the current simulated
//! time in milliseconds to [`ScanEngine::scan_once`]. Together with the fact
//! that rungs are evaluated in declaration order and that no iteration order
//! depends on hashing, this makes a scan a pure function of
//! `(project, store, now_ms)` and therefore reproducible.
//!
//! # Power flow
//!
//! Each rung is evaluated right-to-left as a series path: contacts `AND` their
//! value into the path, blocks such as `Timer`/`Counter` pass the path through
//! once their output is true, and the resulting rung flow drives every coil on
//! the rung. Rows (`PlacedElement::row`) are treated as parallel branches and
//! `OR`ed together.
//!
//! `Connection` elements — the horizontal and vertical links ClassicLadder
//! draws between branches — do not influence the result yet; see the
//! `TODO(M1)` comments in this module for the planned per-column power-flow
//! propagation.

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

use crate::diag::{Diagnostic, Severity};
use crate::expr::{compare_values, eval, parse, Value, VarSource};
use crate::model::{
    CounterKind, ElementKind, PlacedElement, Project, Rung, SectionLanguage, TimerMode,
};
use crate::vars::{VarKind, VarRef};

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

/// Error returned when a value cannot be stored into a variable.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StoreError {
    /// The variable index is outside its storage class.
    #[error("variable `{0}` is out of range")]
    OutOfRange(VarRef),
    /// The indirect index expression could not be resolved.
    #[error("cannot resolve the index of `{0}`: {1}")]
    BadIndex(VarRef, String),
}

/// Complete variable state of a running SoftLadder program.
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
    /// Register words (`%R`).
    pub registers: Vec<i32>,
    /// Sequential step activity bits (`%X`).
    pub steps: Vec<bool>,
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
    /// Creates a store with the ClassicLadder default sizes.
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
            registers: vec![0; DEFAULT_REGISTERS],
            steps: vec![false; DEFAULT_STEPS],
            system: vec![false; DEFAULT_SYSTEM_BITS],
            leds: vec![false; DEFAULT_LEDS],
        }
    }

    /// Reads a variable, resolving indirect indices and bit selectors.
    ///
    /// Returns [`None`] when the variable does not exist in this store.
    pub fn get(&self, var: &VarRef) -> Option<Value> {
        let index = self.resolve_index(var).ok()?;
        let value = match var.kind {
            VarKind::MemBit => Value::Bit(*self.mem_bits.get(index)?),
            VarKind::MemWord => Value::Word(*self.mem_words.get(index)?),
            VarKind::PhysIn => Value::Bit(*self.phys_in.get(index)?),
            VarKind::PhysOut => Value::Bit(*self.phys_out.get(index)?),
            VarKind::PhysInWord => Value::Word(*self.phys_in_words.get(index)?),
            VarKind::PhysOutWord => Value::Word(*self.phys_out_words.get(index)?),
            VarKind::TimerIec => Value::Bit(self.timers.get(index)?.done),
            VarKind::TimerIecValue => Value::Word(self.timers.get(index)?.elapsed_ms as i32),
            VarKind::Counter => Value::Bit(self.counters.get(index)?.done()),
            VarKind::CounterValue => Value::Word(self.counters.get(index)?.value),
            VarKind::Register => Value::Word(*self.registers.get(index)?),
            VarKind::Step => Value::Bit(*self.steps.get(index)?),
            VarKind::System => Value::Bit(*self.system.get(index)?),
            VarKind::Led => Value::Bit(*self.leds.get(index)?),
        };
        match var.bit {
            Some(bit) if bit < 64 => Some(Value::Bit((value.as_i64() >> bit) & 1 != 0)),
            Some(_) => None,
            None => Some(value),
        }
    }

    /// Writes a variable, resolving indirect indices.
    ///
    /// Values are coerced to the storage class of the variable: bits use the
    /// truthiness of the value and words use its integer view.
    pub fn set(&mut self, var: &VarRef, value: Value) -> Result<(), StoreError> {
        let index = self.resolve_index(var)?;
        match var.kind {
            VarKind::MemBit => write_bit(&mut self.mem_bits, index, var, value.as_bool()),
            VarKind::MemWord => write_word(&mut self.mem_words, index, var, value.as_i64()),
            VarKind::PhysIn => write_bit(&mut self.phys_in, index, var, value.as_bool()),
            VarKind::PhysOut => write_bit(&mut self.phys_out, index, var, value.as_bool()),
            VarKind::PhysInWord => write_word(&mut self.phys_in_words, index, var, value.as_i64()),
            VarKind::PhysOutWord => {
                write_word(&mut self.phys_out_words, index, var, value.as_i64())
            }
            VarKind::TimerIec => match self.timers.get_mut(index) {
                Some(timer) => {
                    timer.done = value.as_bool();
                    Ok(())
                }
                None => Err(StoreError::OutOfRange(var.clone())),
            },
            VarKind::TimerIecValue => match self.timers.get_mut(index) {
                Some(timer) => {
                    timer.elapsed_ms = clamp_u32(value.as_i64());
                    Ok(())
                }
                None => Err(StoreError::OutOfRange(var.clone())),
            },
            VarKind::Counter => match self.counters.get_mut(index) {
                Some(counter) => {
                    counter.set_done(value.as_bool());
                    Ok(())
                }
                None => Err(StoreError::OutOfRange(var.clone())),
            },
            VarKind::CounterValue => match self.counters.get_mut(index) {
                Some(counter) => {
                    counter.value = clamp_i32(value.as_i64());
                    Ok(())
                }
                None => Err(StoreError::OutOfRange(var.clone())),
            },
            VarKind::Register => write_word(&mut self.registers, index, var, value.as_i64()),
            VarKind::Step => write_bit(&mut self.steps, index, var, value.as_bool()),
            VarKind::System => write_bit(&mut self.system, index, var, value.as_bool()),
            VarKind::Led => write_bit(&mut self.leds, index, var, value.as_bool()),
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

    /// Number of physical digital channels; `%I` and `%Q` share the numbering.
    pub fn digital_channels(&self) -> usize {
        self.phys_in.len().max(self.phys_out.len())
    }

    /// Number of physical analog channels; `%IW` and `%QW` share the numbering.
    pub fn analog_channels(&self) -> usize {
        self.phys_in_words.len().max(self.phys_out_words.len())
    }

    /// Resolves the effective index of `var`, following `index_expr`.
    fn resolve_index(&self, var: &VarRef) -> Result<usize, StoreError> {
        match &var.index_expr {
            None => Ok(var.index as usize),
            Some(index_var) => {
                let value = self
                    .get(index_var)
                    .ok_or_else(|| {
                        StoreError::BadIndex(
                            var.clone(),
                            format!("index variable `{index_var}` is unknown"),
                        )
                    })?
                    .as_i64();
                if value < 0 {
                    Err(StoreError::BadIndex(
                        var.clone(),
                        format!("index {value} is negative"),
                    ))
                } else {
                    Ok(value as usize)
                }
            }
        }
    }
}

impl VarSource for VarStore {
    fn get(&self, var: &VarRef) -> Option<Value> {
        VarStore::get(self, var)
    }
}

fn write_bit(slot: &mut [bool], index: usize, var: &VarRef, value: bool) -> Result<(), StoreError> {
    match slot.get_mut(index) {
        Some(cell) => {
            *cell = value;
            Ok(())
        }
        None => Err(StoreError::OutOfRange(var.clone())),
    }
}

fn write_word(slot: &mut [i32], index: usize, var: &VarRef, value: i64) -> Result<(), StoreError> {
    match slot.get_mut(index) {
        Some(cell) => {
            *cell = clamp_i32(value);
            Ok(())
        }
        None => Err(StoreError::OutOfRange(var.clone())),
    }
}

fn clamp_i32(value: i64) -> i32 {
    value.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

fn clamp_u32(value: i64) -> u32 {
    value.clamp(0, i64::from(u32::MAX)) as u32
}

/// State of an IEC 61131-3 timer block.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimerIec {
    /// Preset duration in milliseconds.
    pub preset_ms: u32,
    /// Time accumulated in the current phase, in milliseconds.
    pub elapsed_ms: u32,
    /// Timer flavour.
    pub mode: TimerMode,
    /// Whether the timer output is currently true.
    pub done: bool,
    /// Previous scan timestamp, used to accumulate elapsed time.
    last_now_ms: Option<u64>,
    /// Input value seen during the previous scan, used by the pulse timer.
    prev_input: bool,
}

impl Default for TimerIec {
    fn default() -> Self {
        Self {
            preset_ms: 0,
            elapsed_ms: 0,
            mode: TimerMode::On,
            done: false,
            last_now_ms: None,
            prev_input: false,
        }
    }
}

impl TimerIec {
    /// Creates a timer with the given flavour and preset.
    pub fn new(mode: TimerMode, preset_ms: u32) -> Self {
        Self {
            mode,
            preset_ms,
            ..Self::default()
        }
    }

    /// Runs one scan of the timer and returns its output.
    ///
    /// `now_ms` is the caller-supplied scan timestamp and `input` is the power
    /// flow reaching the block. The first call only establishes the time base,
    /// so it accumulates zero milliseconds.
    pub fn update(&mut self, now_ms: u64, input: bool) -> bool {
        let delta = match self.last_now_ms {
            Some(previous) if now_ms > previous => {
                (now_ms - previous).min(u64::from(u32::MAX)) as u32
            }
            _ => 0,
        };
        self.last_now_ms = Some(now_ms);

        match self.mode {
            TimerMode::On => {
                if input {
                    if !self.done {
                        self.elapsed_ms = self.elapsed_ms.saturating_add(delta);
                        if self.elapsed_ms >= self.preset_ms {
                            self.elapsed_ms = self.preset_ms;
                            self.done = true;
                        }
                    }
                } else {
                    self.elapsed_ms = 0;
                    self.done = false;
                }
            }
            TimerMode::Off => {
                if input {
                    self.elapsed_ms = 0;
                    self.done = true;
                } else if self.done {
                    self.elapsed_ms = self.elapsed_ms.saturating_add(delta);
                    if self.elapsed_ms >= self.preset_ms {
                        self.elapsed_ms = self.preset_ms;
                        self.done = false;
                    }
                }
            }
            TimerMode::Pulse => {
                if input && !self.prev_input && !self.done {
                    self.done = true;
                    self.elapsed_ms = 0;
                }
                if self.done {
                    self.elapsed_ms = self.elapsed_ms.saturating_add(delta);
                    if self.elapsed_ms >= self.preset_ms {
                        self.elapsed_ms = self.preset_ms;
                        self.done = false;
                    }
                }
            }
        }

        self.prev_input = input;
        self.done
    }
}

/// State of a counter block.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Counter {
    /// Value at which the counter reports done.
    pub preset: i32,
    /// Current count.
    pub value: i32,
    /// Counter flavour.
    pub kind: CounterKind,
    prev_count_up: bool,
    prev_count_down: bool,
    prev_reset: bool,
    prev_load: bool,
    started: bool,
}

impl Default for Counter {
    fn default() -> Self {
        Self {
            preset: 0,
            value: 0,
            kind: CounterKind::Up,
            prev_count_up: false,
            prev_count_down: false,
            prev_reset: false,
            prev_load: false,
            started: false,
        }
    }
}

impl Counter {
    /// Creates a counter with the given flavour and preset.
    pub fn new(kind: CounterKind, preset: i32) -> Self {
        Self {
            kind,
            preset,
            ..Self::default()
        }
    }

    /// Runs one scan of the counter and returns its new value.
    ///
    /// Counting and reset/load are edge triggered; `reset` wins over `load`,
    /// which in turn wins over counting.
    pub fn update(&mut self, count_up: bool, count_down: bool, reset: bool, load: bool) -> i32 {
        if self.kind == CounterKind::Down && !self.started {
            self.value = self.preset;
            self.started = true;
        }

        if reset && !self.prev_reset {
            self.value = 0;
        } else if load && !self.prev_load {
            self.value = self.preset;
        } else if count_up && !self.prev_count_up && !reset && !load {
            self.value = self.value.saturating_add(1);
        } else if count_down && !self.prev_count_down && !reset && !load {
            self.value = self.value.saturating_sub(1);
        }

        self.prev_count_up = count_up;
        self.prev_count_down = count_down;
        self.prev_reset = reset;
        self.prev_load = load;
        self.value
    }

    /// `true` when the counter has reached its preset (or zero when counting
    /// down).
    pub fn done(&self) -> bool {
        match self.kind {
            CounterKind::Down => self.value <= 0,
            CounterKind::Up | CounterKind::UpDown => self.value >= self.preset,
        }
    }

    /// `true` when the count is zero.
    pub fn is_empty(&self) -> bool {
        self.value == 0
    }

    /// `true` when the count has reached or passed the preset.
    pub fn is_full(&self) -> bool {
        self.value >= self.preset
    }

    /// Overrides the done state, used when the monitor forces the done bit.
    pub fn set_done(&mut self, done: bool) {
        self.value = if done { self.preset } else { 0 };
    }

    /// Clears the count and the stored edge states.
    pub fn clear(&mut self) {
        self.value = 0;
        self.started = false;
        self.prev_count_up = false;
        self.prev_count_down = false;
        self.prev_reset = false;
        self.prev_load = false;
    }
}

/// Previous-state bank used to detect rising and falling edges.
///
/// Only lookups are performed on the map, never iteration, so the result of a
/// scan stays deterministic. The first observation of a variable counts as a
/// transition from `false`.
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

/// Result of a single scan.
#[derive(Debug, Clone, PartialEq)]
pub struct ScanReport {
    /// Number of completed scans, including this one.
    pub cycles: u64,
    /// Diagnostics produced while evaluating the rungs.
    pub diagnostics: Vec<Diagnostic>,
}

/// Collects diagnostics together with their section and rung context.
struct DiagSink<'a> {
    section: Option<usize>,
    rung: Option<usize>,
    diagnostics: &'a mut Vec<Diagnostic>,
}

impl<'a> DiagSink<'a> {
    fn new(
        section: Option<usize>,
        rung: Option<usize>,
        diagnostics: &'a mut Vec<Diagnostic>,
    ) -> Self {
        Self {
            section,
            rung,
            diagnostics,
        }
    }

    fn error(&mut self, code: &'static str, message: String) {
        let mut diagnostic = Diagnostic::new(Severity::Error, code, message);
        diagnostic.section = self.section;
        diagnostic.rung = self.rung;
        self.diagnostics.push(diagnostic);
    }
}

/// The deterministic ladder-logic interpreter.
#[derive(Debug, Clone)]
pub struct ScanEngine {
    project: Project,
    store: VarStore,
    edges: EdgeBank,
    cycles: u64,
}

impl ScanEngine {
    /// Creates an engine for `project` with a default-sized variable store.
    pub fn new(project: Project) -> Self {
        Self {
            project,
            store: VarStore::with_default_sizes(),
            edges: EdgeBank::new(),
            cycles: 0,
        }
    }

    /// Creates an engine that runs `project` against a caller-provided store.
    pub fn with_store(project: Project, store: VarStore) -> Self {
        Self {
            project,
            store,
            edges: EdgeBank::new(),
            cycles: 0,
        }
    }

    /// The project being executed.
    pub fn project(&self) -> &Project {
        &self.project
    }

    /// Mutable access to the project, for editors.
    pub fn project_mut(&mut self) -> &mut Project {
        &mut self.project
    }

    /// The variable store.
    pub fn store(&self) -> &VarStore {
        &self.store
    }

    /// Mutable access to the variable store, for drivers and the monitor.
    pub fn store_mut(&mut self) -> &mut VarStore {
        &mut self.store
    }

    /// Number of completed scans.
    pub fn cycles(&self) -> u64 {
        self.cycles
    }

    /// Runs one scan at simulated time `now_ms`.
    ///
    /// The function never panics: malformed elements, unknown variables and
    /// evaluation failures are reported through [`ScanReport::diagnostics`].
    pub fn scan_once(&mut self, now_ms: u64) -> ScanReport {
        let mut diagnostics = Vec::new();

        for (section_index, section) in self.project.sections.iter().enumerate() {
            if section.language == SectionLanguage::Sfc {
                let mut diagnostic = Diagnostic::new(
                    Severity::Warning,
                    "SL-W002",
                    "SFC sections are not executed until M4".to_owned(),
                );
                diagnostic.section = Some(section_index);
                diagnostics.push(diagnostic);
                continue;
            }

            for rung_id in &section.rungs {
                let Some(rung) = self.project.rungs.iter().find(|rung| rung.id == *rung_id) else {
                    let mut diagnostic = Diagnostic::new(
                        Severity::Warning,
                        "SL-W001",
                        format!("section references missing rung id {rung_id}"),
                    );
                    diagnostic.section = Some(section_index);
                    diagnostics.push(diagnostic);
                    continue;
                };
                let mut sink = DiagSink::new(
                    Some(section_index),
                    Some(rung.id as usize),
                    &mut diagnostics,
                );
                eval_rung(&mut self.store, &mut self.edges, rung, now_ms, &mut sink);
            }
        }

        self.cycles = self.cycles.saturating_add(1);
        ScanReport {
            cycles: self.cycles,
            diagnostics,
        }
    }
}

/// Evaluates one rung and returns its final power flow.
fn eval_rung(
    store: &mut VarStore,
    edges: &mut EdgeBank,
    rung: &Rung,
    now_ms: u64,
    sink: &mut DiagSink<'_>,
) -> bool {
    // TODO(M1): replace this row-OR approximation with true per-column
    // power-flow propagation, so that `Connection` elements and elements placed
    // after a branch point are evaluated exactly like ClassicLadder does.
    let mut rows: BTreeMap<u8, Vec<&PlacedElement>> = BTreeMap::new();
    let mut coils: Vec<&PlacedElement> = Vec::new();
    for element in &rung.elements {
        if element.kind.is_coil() {
            coils.push(element);
        } else {
            rows.entry(element.row).or_default().push(element);
        }
    }

    let mut rung_flow = false;
    for elements in rows.values_mut() {
        elements.sort_by_key(|element| element.col);
        let mut series = true;
        for element in elements.iter() {
            series = eval_path_element(store, edges, element, now_ms, series, sink);
        }
        rung_flow |= series;
    }

    // Coils are driven by the final flow of the rung.
    for coil in coils {
        apply_coil(store, coil, rung_flow, sink);
    }

    rung_flow
}

/// Evaluates one non-coil element and returns the updated series flow.
fn eval_path_element(
    store: &mut VarStore,
    edges: &mut EdgeBank,
    element: &PlacedElement,
    now_ms: u64,
    flow_in: bool,
    sink: &mut DiagSink<'_>,
) -> bool {
    match element.kind {
        ElementKind::ContactNo => flow_in & read_bit(store, element, sink),
        ElementKind::ContactNc => flow_in & !read_bit(store, element, sink),
        ElementKind::ContactRising => match element.var.as_ref() {
            Some(var) => {
                let value = read_bit(store, element, sink);
                flow_in & edges.rising(var, value)
            }
            None => {
                sink.error("SL-E004", "edge contact has no variable".to_owned());
                flow_in
            }
        },
        ElementKind::ContactFalling => match element.var.as_ref() {
            Some(var) => {
                let value = read_bit(store, element, sink);
                flow_in & edges.falling(var, value)
            }
            None => {
                sink.error("SL-E004", "edge contact has no variable".to_owned());
                flow_in
            }
        },
        ElementKind::Compare => flow_in & compare_element(store, element, sink),
        ElementKind::Operate => {
            if flow_in {
                operate_element(store, element, sink);
            }
            flow_in
        }
        ElementKind::Timer { mode } => timer_element(store, element, mode, now_ms, flow_in, sink),
        ElementKind::Counter { kind } => counter_element(store, element, kind, flow_in, sink),
        ElementKind::Register { .. } => {
            // TODO(M1): implement FIFO/LIFO register semantics.
            flow_in
        }
        ElementKind::Connection => {
            // TODO(M1): connections carry the branch topology; the skeleton
            // treats rows as branches and ignores the links.
            flow_in
        }
        ElementKind::CoilOut
        | ElementKind::CoilOutNeg
        | ElementKind::CoilSet
        | ElementKind::CoilReset
        | ElementKind::CoilJump
        | ElementKind::CoilCall => flow_in,
    }
}

/// Reads the bit value of an element's variable, reporting failures.
fn read_bit(store: &VarStore, element: &PlacedElement, sink: &mut DiagSink<'_>) -> bool {
    match element.var.as_ref() {
        Some(var) => match store.get(var) {
            Some(value) => value.as_bool(),
            None => {
                sink.error(
                    "SL-E001",
                    format!("variable `{var}` is not defined in the store"),
                );
                false
            }
        },
        None => {
            sink.error("SL-E004", "element has no variable".to_owned());
            false
        }
    }
}

/// Parses an integer parameter, either as a literal or as a variable value.
fn parse_numeric_param(store: &VarStore, element: &PlacedElement, index: usize) -> Option<i64> {
    let text = element.params.get(index)?;
    if let Ok(literal) = text.trim().parse::<i64>() {
        return Some(literal);
    }
    let var: VarRef = text.trim().parse().ok()?;
    store.get(&var).map(Value::as_i64)
}

fn timer_element(
    store: &mut VarStore,
    element: &PlacedElement,
    mode: TimerMode,
    now_ms: u64,
    flow_in: bool,
    sink: &mut DiagSink<'_>,
) -> bool {
    let Some(var) = element.var.as_ref() else {
        sink.error("SL-E004", "timer block has no variable".to_owned());
        return flow_in;
    };
    if var.kind != VarKind::TimerIec {
        sink.error(
            "SL-E003",
            format!("timer block must reference a `%TM` variable, got `{var}`"),
        );
        return flow_in;
    }
    let preset = parse_numeric_param(store, element, 0);
    let Some(timer) = store.timer_mut(var.index as usize) else {
        sink.error("SL-E003", format!("timer `{var}` is out of range"));
        return flow_in;
    };
    timer.mode = mode;
    if let Some(preset) = preset {
        timer.preset_ms = clamp_u32(preset);
    }
    timer.update(now_ms, flow_in)
}

fn counter_element(
    store: &mut VarStore,
    element: &PlacedElement,
    kind: CounterKind,
    flow_in: bool,
    sink: &mut DiagSink<'_>,
) -> bool {
    let Some(var) = element.var.as_ref() else {
        sink.error("SL-E004", "counter block has no variable".to_owned());
        return flow_in;
    };
    if var.kind != VarKind::Counter {
        sink.error(
            "SL-E003",
            format!("counter block must reference a `%C` variable, got `{var}`"),
        );
        return flow_in;
    }
    let preset = parse_numeric_param(store, element, 0);
    // TODO(M1): dedicated count-down, reset and load elements feed the
    // remaining inputs; the skeleton counts up while the rung has flow.
    let Some(counter) = store.counter_mut(var.index as usize) else {
        sink.error("SL-E003", format!("counter `{var}` is out of range"));
        return flow_in;
    };
    counter.kind = kind;
    if let Some(preset) = preset {
        counter.preset = clamp_i32(preset);
    }
    let count_up = matches!(kind, CounterKind::Up | CounterKind::UpDown) && flow_in;
    let count_down = kind == CounterKind::Down && flow_in;
    counter.update(count_up, count_down, false, false);
    counter.done()
}

fn compare_element(store: &VarStore, element: &PlacedElement, sink: &mut DiagSink<'_>) -> bool {
    let params = &element.params;
    if params.len() == 1 {
        let text = params.first().map(String::as_str).unwrap_or_default();
        return match parse(text) {
            Ok(expr) => match eval(&expr, store) {
                Ok(value) => value.as_bool(),
                Err(error) => {
                    sink.error("SL-E002", format!("cannot evaluate `{text}`: {error}"));
                    false
                }
            },
            Err(error) => {
                sink.error("SL-E002", format!("cannot parse `{text}`: {error}"));
                false
            }
        };
    }
    if params.len() < 3 {
        sink.error(
            "SL-E003",
            "compare block needs either one expression or `<lhs> <op> <rhs>`".to_owned(),
        );
        return false;
    }
    let lhs_text = params.first().map(String::as_str).unwrap_or_default();
    let op_text = params.get(1).map(String::as_str).unwrap_or_default();
    let rhs_text = params.get(2).map(String::as_str).unwrap_or_default();
    let lhs = parse(lhs_text).and_then(|expr| eval(&expr, store));
    let rhs = parse(rhs_text).and_then(|expr| eval(&expr, store));
    match (lhs, rhs) {
        (Ok(lhs), Ok(rhs)) => match compare_values(op_text, &lhs, &rhs) {
            Ok(result) => result,
            Err(error) => {
                sink.error(
                    "SL-E002",
                    format!("cannot compare `{lhs_text}` {op_text} `{rhs_text}`: {error}"),
                );
                false
            }
        },
        (Err(error), _) => {
            sink.error("SL-E002", format!("cannot evaluate `{lhs_text}`: {error}"));
            false
        }
        (_, Err(error)) => {
            sink.error("SL-E002", format!("cannot evaluate `{rhs_text}`: {error}"));
            false
        }
    }
}

fn operate_element(store: &mut VarStore, element: &PlacedElement, sink: &mut DiagSink<'_>) {
    let Some(target_text) = element.params.first() else {
        sink.error("SL-E002", "operate block has no target variable".to_owned());
        return;
    };
    let Ok(target) = target_text.trim().parse::<VarRef>() else {
        sink.error(
            "SL-E002",
            format!("operate target `{target_text}` is not a variable"),
        );
        return;
    };
    let mut expression = element
        .params
        .iter()
        .skip(1)
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(" ");
    if let Some(rest) = expression.strip_prefix('=') {
        expression = rest.trim().to_owned();
    }
    if expression.trim().is_empty() {
        sink.error(
            "SL-E002",
            format!("operate block for `{target}` has no expression"),
        );
        return;
    }
    let expr = match parse(&expression) {
        Ok(expr) => expr,
        Err(error) => {
            sink.error("SL-E002", format!("cannot parse `{expression}`: {error}"));
            return;
        }
    };
    match eval(&expr, store) {
        Ok(value) => {
            if let Err(error) = store.set(&target, value) {
                sink.error("SL-E002", format!("cannot store into `{target}`: {error}"));
            }
        }
        Err(error) => {
            sink.error(
                "SL-E002",
                format!("cannot evaluate `{expression}`: {error}"),
            );
        }
    }
}

fn apply_coil(store: &mut VarStore, element: &PlacedElement, flow: bool, sink: &mut DiagSink<'_>) {
    let Some(var) = element.var.as_ref() else {
        sink.error("SL-E004", "coil has no variable".to_owned());
        return;
    };
    let value = match element.kind {
        ElementKind::CoilOut => Some(flow),
        ElementKind::CoilOutNeg => Some(!flow),
        ElementKind::CoilSet => flow.then_some(true),
        ElementKind::CoilReset => flow.then_some(false),
        // TODO(M1): jump and call coils need the section/execution model.
        ElementKind::CoilJump | ElementKind::CoilCall => None,
        _ => None,
    };
    if let Some(value) = value {
        if let Err(error) = store.set(var, Value::Bit(value)) {
            sink.error("SL-E001", format!("cannot drive `{var}`: {error}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{PlacedElement, Project, Rung, Section};

    fn var(text: &str) -> VarRef {
        text.parse::<VarRef>().expect("test variable should parse")
    }

    fn project_with(rungs: Vec<Rung>) -> Project {
        let rung_ids = rungs.iter().map(|rung| rung.id).collect();
        let mut project = Project::new("test");
        project.sections.push(Section {
            rungs: rung_ids,
            ..Section::new(1, "Main")
        });
        project.rungs = rungs;
        project
    }

    fn element(
        kind: ElementKind,
        var: Option<&str>,
        col: u8,
        row: u8,
        params: &[&str],
    ) -> PlacedElement {
        PlacedElement {
            kind,
            var: var.map(self::var),
            col,
            row,
            params: params.iter().map(|param| (*param).to_owned()).collect(),
        }
    }

    fn engine_with(rung: Rung) -> ScanEngine {
        ScanEngine::new(project_with(vec![rung]))
    }

    #[test]
    fn ton_fires_after_its_preset() {
        let rung = Rung {
            elements: vec![
                element(ElementKind::ContactNo, Some("%I0"), 0, 0, &[]),
                element(
                    ElementKind::Timer {
                        mode: TimerMode::On,
                    },
                    Some("%TM0"),
                    1,
                    0,
                    &["3000"],
                ),
                element(ElementKind::CoilOut, Some("%Q0"), 2, 0, &[]),
            ],
            ..Rung::new(1)
        };
        let mut engine = engine_with(rung);
        engine
            .store_mut()
            .set(&var("%I0"), Value::Bit(true))
            .expect("input can be set");

        engine.scan_once(0);
        engine.scan_once(1500);
        assert_eq!(engine.store().get(&var("%TM0")), Some(Value::Bit(false)));
        assert_eq!(engine.store().get(&var("%Q0")), Some(Value::Bit(false)));
        assert_eq!(engine.store().get(&var("%TM0.V")), Some(Value::Word(1500)));

        engine.scan_once(3000);
        assert_eq!(engine.store().get(&var("%TM0")), Some(Value::Bit(true)));
        assert_eq!(engine.store().get(&var("%Q0")), Some(Value::Bit(true)));
    }

    #[test]
    fn tof_and_pulse_timers() {
        let mut tof = TimerIec::new(TimerMode::Off, 1000);
        assert!(tof.update(0, true));
        assert!(tof.update(500, true));
        assert!(tof.update(1000, false));
        assert!(tof.update(1400, false));
        assert!(!tof.update(1600, false));

        let mut pulse = TimerIec::new(TimerMode::Pulse, 100);
        assert!(!pulse.update(0, false));
        assert!(pulse.update(10, true));
        assert!(pulse.update(50, true));
        assert!(!pulse.update(200, true));
        assert!(!pulse.update(300, false));
    }

    #[test]
    fn counter_reaches_its_preset() {
        let rung = Rung {
            elements: vec![
                element(ElementKind::ContactNo, Some("%I1"), 0, 0, &[]),
                element(
                    ElementKind::Counter {
                        kind: CounterKind::Up,
                    },
                    Some("%C0"),
                    1,
                    0,
                    &["3"],
                ),
                element(ElementKind::CoilOut, Some("%M0"), 2, 0, &[]),
            ],
            ..Rung::new(1)
        };
        let mut engine = engine_with(rung);
        for (now_ms, pressed) in [
            (0u64, false),
            (10, true),
            (20, false),
            (30, true),
            (40, false),
            (50, true),
        ] {
            engine
                .store_mut()
                .set(&var("%I1"), Value::Bit(pressed))
                .expect("input can be set");
            engine.scan_once(now_ms);
        }
        assert_eq!(engine.store().get(&var("%C0.V")), Some(Value::Word(3)));
        assert_eq!(engine.store().get(&var("%C0")), Some(Value::Bit(true)));
        assert_eq!(engine.store().get(&var("%M0")), Some(Value::Bit(true)));
    }

    #[test]
    fn down_counter_starts_at_its_preset() {
        let mut counter = Counter::new(CounterKind::Down, 2);
        counter.update(false, false, false, false);
        assert!(!counter.done());
        counter.update(false, true, false, false);
        assert!(!counter.done());
        counter.update(false, false, false, false);
        counter.update(false, true, false, false);
        assert!(counter.done());
        assert_eq!(counter.value, 0);
    }

    #[test]
    fn two_contacts_in_series_and_the_rail() {
        let rung = Rung {
            elements: vec![
                element(ElementKind::ContactNo, Some("%I0"), 0, 0, &[]),
                element(ElementKind::ContactNo, Some("%I1"), 1, 0, &[]),
                element(ElementKind::CoilOut, Some("%Q0"), 2, 0, &[]),
            ],
            ..Rung::new(1)
        };
        let mut engine = engine_with(rung);
        engine
            .store_mut()
            .set(&var("%I0"), Value::Bit(true))
            .expect("input can be set");
        engine
            .store_mut()
            .set(&var("%I1"), Value::Bit(false))
            .expect("input can be set");
        engine.scan_once(0);
        assert_eq!(engine.store().get(&var("%Q0")), Some(Value::Bit(false)));

        engine
            .store_mut()
            .set(&var("%I1"), Value::Bit(true))
            .expect("input can be set");
        engine.scan_once(10);
        assert_eq!(engine.store().get(&var("%Q0")), Some(Value::Bit(true)));

        engine
            .store_mut()
            .set(&var("%I0"), Value::Bit(false))
            .expect("input can be set");
        engine.scan_once(20);
        assert_eq!(engine.store().get(&var("%Q0")), Some(Value::Bit(false)));
    }

    #[test]
    fn parallel_rows_are_ored() {
        // `%M0` (or an output coil) in the second row latches the rung: the
        // classic start/stop self-holding circuit.
        let rung = Rung {
            elements: vec![
                element(ElementKind::ContactNo, Some("%I0"), 0, 0, &[]),
                element(ElementKind::ContactNo, Some("%I1"), 1, 0, &[]),
                element(ElementKind::ContactNo, Some("%Q0"), 1, 1, &[]),
                element(ElementKind::CoilOut, Some("%Q0"), 2, 0, &[]),
            ],
            ..Rung::new(1)
        };
        let mut engine = engine_with(rung);
        engine
            .store_mut()
            .set(&var("%I1"), Value::Bit(true))
            .expect("input can be set");
        engine.scan_once(0);
        assert_eq!(engine.store().get(&var("%Q0")), Some(Value::Bit(false)));

        engine
            .store_mut()
            .set(&var("%I0"), Value::Bit(true))
            .expect("input can be set");
        engine.scan_once(10);
        assert_eq!(engine.store().get(&var("%Q0")), Some(Value::Bit(true)));

        // Releasing the button keeps the lamp on through the `%Q0` branch.
        engine
            .store_mut()
            .set(&var("%I0"), Value::Bit(false))
            .expect("input can be set");
        engine.scan_once(20);
        assert_eq!(engine.store().get(&var("%Q0")), Some(Value::Bit(true)));
    }

    #[test]
    fn set_and_reset_coils_latch() {
        let rung = Rung {
            elements: vec![
                element(ElementKind::ContactNo, Some("%I0"), 0, 0, &[]),
                element(ElementKind::CoilSet, Some("%M5"), 1, 0, &[]),
            ],
            ..Rung::new(1)
        };
        let mut engine = engine_with(rung);
        engine
            .store_mut()
            .set(&var("%I0"), Value::Bit(true))
            .expect("input can be set");
        engine.scan_once(0);
        engine
            .store_mut()
            .set(&var("%I0"), Value::Bit(false))
            .expect("input can be set");
        engine.scan_once(10);
        assert_eq!(engine.store().get(&var("%M5")), Some(Value::Bit(true)));
    }

    #[test]
    fn edge_contacts_detect_transitions() {
        let rung = Rung {
            elements: vec![
                element(ElementKind::ContactRising, Some("%I0"), 0, 0, &[]),
                element(ElementKind::CoilOut, Some("%Q0"), 1, 0, &[]),
            ],
            ..Rung::new(1)
        };
        let mut engine = engine_with(rung);
        engine
            .store_mut()
            .set(&var("%I0"), Value::Bit(false))
            .expect("input can be set");
        engine.scan_once(0);
        engine
            .store_mut()
            .set(&var("%I0"), Value::Bit(true))
            .expect("input can be set");
        engine.scan_once(10);
        assert_eq!(engine.store().get(&var("%Q0")), Some(Value::Bit(true)));
        engine.scan_once(20);
        assert_eq!(engine.store().get(&var("%Q0")), Some(Value::Bit(false)));
    }

    #[test]
    fn operate_divide_by_zero_reports_an_error_and_keeps_running() {
        let rung = Rung {
            elements: vec![
                element(ElementKind::Operate, None, 0, 0, &["%MW0", "=", "1 / 0"]),
                element(ElementKind::Operate, None, 1, 0, &["%MW1", "=", "2 + 3"]),
                element(ElementKind::CoilOut, Some("%Q0"), 2, 0, &[]),
            ],
            ..Rung::new(1)
        };
        let mut engine = engine_with(rung);
        let report = engine.scan_once(0);
        assert!(report.diagnostics.iter().any(
            |diagnostic| diagnostic.severity == Severity::Error && diagnostic.code == "SL-E002"
        ));
        assert_eq!(report.cycles, 1);
        assert_eq!(engine.store().get(&var("%MW0")), Some(Value::Word(0)));
        assert_eq!(engine.store().get(&var("%MW1")), Some(Value::Word(5)));
        assert_eq!(engine.store().get(&var("%Q0")), Some(Value::Bit(true)));
    }

    #[test]
    fn operate_is_gated_by_the_rung_flow() {
        let rung = Rung {
            elements: vec![
                element(ElementKind::ContactNo, Some("%I0"), 0, 0, &[]),
                element(ElementKind::Operate, None, 1, 0, &["%MW0", "%MW0 + 1"]),
            ],
            ..Rung::new(1)
        };
        let mut engine = engine_with(rung);
        engine.scan_once(0);
        assert_eq!(engine.store().get(&var("%MW0")), Some(Value::Word(0)));
        engine
            .store_mut()
            .set(&var("%I0"), Value::Bit(true))
            .expect("input can be set");
        engine.scan_once(10);
        engine.scan_once(20);
        assert_eq!(engine.store().get(&var("%MW0")), Some(Value::Word(2)));
    }

    #[test]
    fn compare_blocks_work_with_expressions_and_operators() {
        let rung = Rung {
            elements: vec![
                element(ElementKind::Compare, None, 0, 0, &["%MW0 > 3"]),
                element(ElementKind::Compare, None, 1, 0, &["%MW0", "<=", "10"]),
                element(ElementKind::CoilOut, Some("%Q0"), 2, 0, &[]),
            ],
            ..Rung::new(1)
        };
        let mut engine = engine_with(rung);
        engine
            .store_mut()
            .set(&var("%MW0"), Value::Word(5))
            .expect("word can be set");
        engine.scan_once(0);
        assert_eq!(engine.store().get(&var("%Q0")), Some(Value::Bit(true)));
        engine
            .store_mut()
            .set(&var("%MW0"), Value::Word(2))
            .expect("word can be set");
        engine.scan_once(10);
        assert_eq!(engine.store().get(&var("%Q0")), Some(Value::Bit(false)));
    }

    #[test]
    fn malformed_elements_do_not_panic() {
        let rung = Rung {
            elements: vec![
                element(ElementKind::ContactNo, None, 0, 0, &[]),
                element(ElementKind::ContactNo, Some("%M9999"), 1, 0, &[]),
                element(ElementKind::CoilOut, None, 2, 0, &[]),
                element(
                    ElementKind::Timer {
                        mode: TimerMode::On,
                    },
                    Some("%M0"),
                    3,
                    0,
                    &["not-a-number"],
                ),
                element(
                    ElementKind::Counter {
                        kind: CounterKind::Up,
                    },
                    Some("%M1"),
                    4,
                    0,
                    &["nonsense"],
                ),
                element(ElementKind::Compare, None, 5, 0, &["%MW0", "<"]),
                element(
                    ElementKind::Operate,
                    None,
                    6,
                    0,
                    &["not-a-variable", "=", "1"],
                ),
            ],
            ..Rung::new(1)
        };
        let mut engine = engine_with(rung);
        let report = engine.scan_once(0);
        assert!(!report.diagnostics.is_empty());
        assert!(report
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.severity != Severity::Info));
    }

    #[test]
    fn missing_rungs_and_sfc_sections_are_reported() {
        let mut project = Project::new("test");
        let mut section = Section::new(1, "Main");
        section.rungs.push(42);
        project.sections.push(section);
        let mut sfc = Section::new(2, "Sequence");
        sfc.language = SectionLanguage::Sfc;
        project.sections.push(sfc);

        let mut engine = ScanEngine::new(project);
        let report = engine.scan_once(0);
        assert!(report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "SL-W001"));
        assert!(report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "SL-W002"));
    }

    #[test]
    fn store_handles_indirect_indices_and_ranges() {
        let mut store = VarStore::with_default_sizes();
        store
            .set(&var("%MW0"), Value::Word(2))
            .expect("index word can be set");
        store
            .set(&var("%MW[%MW0]"), Value::Word(42))
            .expect("indirect write");
        assert_eq!(store.get(&var("%MW2")), Some(Value::Word(42)));
        assert_eq!(store.get(&var("%MW[%MW0]")), Some(Value::Word(42)));
        assert_eq!(store.get(&var("%MW0.1")), Some(Value::Bit(true)));
        assert_eq!(store.get(&var("%MW9999")), None);
        assert_eq!(
            store.set(&var("%MW9999"), Value::Word(1)),
            Err(StoreError::OutOfRange(var("%MW9999")))
        );
        store
            .set(&var("%MW1"), Value::Word(-1))
            .expect("index word can be set");
        assert!(matches!(
            store.set(&var("%MW[%MW1]"), Value::Word(1)),
            Err(StoreError::BadIndex(_, _))
        ));
    }

    #[test]
    fn edge_bank_tracks_state() {
        let mut edges = EdgeBank::new();
        assert!(edges.is_empty());
        assert!(edges.rising(&var("%M0"), true));
        assert!(!edges.rising(&var("%M0"), true));
        assert!(edges.falling(&var("%M0"), false));
        assert_eq!(edges.len(), 1);
        edges.clear();
        assert!(edges.is_empty());
    }

    #[test]
    fn scan_reports_increase_the_cycle_count() {
        let mut engine = ScanEngine::new(Project::new("empty"));
        assert_eq!(engine.scan_once(0).cycles, 1);
        assert_eq!(engine.scan_once(1).cycles, 2);
        assert_eq!(engine.cycles(), 2);
    }

    #[test]
    fn counters_expose_empty_and_full() {
        let mut counter = Counter::new(CounterKind::Up, 2);
        assert!(counter.is_empty());
        counter.update(true, false, false, false);
        assert!(!counter.is_empty());
        assert!(!counter.is_full());
        counter.update(false, false, false, false);
        counter.update(true, false, false, false);
        assert!(counter.is_full());
        assert!(counter.done());
        counter.set_done(false);
        assert_eq!(counter.value, 0);
        counter.clear();
        assert!(counter.is_empty());
    }
}
