//! Numeric tables of the ClassicLadder format, and the mapping between its
//! element/variable codes and the SoftLadder model.
//!
//! The tables are normative in `docs/COMPAT.md` §8.2 and §8.4. Everything here
//! is a pure function: decoding never panics, whatever numbers the document
//! holds, and encoding reports the features ClassicLadder cannot express so the
//! caller can raise `SL-W033`.

use softladder_core::{
    Accessor, CounterKind, ElementKind, RegisterMode, TimerMode, VarKind, VarRef,
};

/// `ELE_FREE` — the cell holds nothing.
pub(crate) const ELE_FREE: i64 = 0;
/// `ELE_INPUT` — normally-open contact.
pub(crate) const ELE_INPUT: i64 = 1;
/// `ELE_INPUT_NOT` — normally-closed contact.
pub(crate) const ELE_INPUT_NOT: i64 = 2;
/// `ELE_RISING_INPUT` — rising-edge contact.
pub(crate) const ELE_RISING_INPUT: i64 = 3;
/// `ELE_FALLING_INPUT` — falling-edge contact.
pub(crate) const ELE_FALLING_INPUT: i64 = 4;
/// `ELE_CONNECTION` — wire / vertical link carrier.
pub(crate) const ELE_CONNECTION: i64 = 9;
/// `ELE_TIMER` — deprecated timer block.
pub(crate) const ELE_TIMER: i64 = 10;
/// `ELE_MONOSTABLE` — deprecated monostable block.
pub(crate) const ELE_MONOSTABLE: i64 = 11;
/// `ELE_COUNTER` — counter block.
pub(crate) const ELE_COUNTER: i64 = 12;
/// `ELE_TIMER_IEC` — IEC timer block.
pub(crate) const ELE_TIMER_IEC: i64 = 13;
/// `ELE_REGISTER` — FIFO/LIFO register block.
pub(crate) const ELE_REGISTER: i64 = 14;
/// `ELE_COMPAR` — comparison block.
pub(crate) const ELE_COMPAR: i64 = 20;
/// `ELE_OUTPUT` — output coil.
pub(crate) const ELE_OUTPUT: i64 = 50;
/// `ELE_OUTPUT_NOT` — negated output coil.
pub(crate) const ELE_OUTPUT_NOT: i64 = 51;
/// `ELE_OUTPUT_SET` — latch coil.
pub(crate) const ELE_OUTPUT_SET: i64 = 52;
/// `ELE_OUTPUT_RESET` — unlatch coil.
pub(crate) const ELE_OUTPUT_RESET: i64 = 53;
/// `ELE_OUTPUT_JUMP` — jump coil.
pub(crate) const ELE_OUTPUT_JUMP: i64 = 54;
/// `ELE_OUTPUT_CALL` — subroutine call coil.
pub(crate) const ELE_OUTPUT_CALL: i64 = 55;
/// `ELE_OUTPUT_OPERATE` — assignment block.
pub(crate) const ELE_OUTPUT_OPERATE: i64 = 60;
/// `ELE_UNUSABLE` — body cell of a multi-cell block.
pub(crate) const ELE_UNUSABLE: i64 = 99;

/// Width of the reference rung matrix, in cells.
pub(crate) const RUNG_WIDTH: u8 = 12;
/// Height of the reference rung matrix, in cells.
pub(crate) const RUNG_HEIGHT: u8 = 8;

/// `VAR_MEM_BIT` — `%B<n>`, SoftLadder's `%M<n>`.
pub(crate) const VAR_MEM_BIT: i64 = 0;
/// `VAR_TIMER_DONE` — deprecated `%T<n>.D`.
pub(crate) const VAR_TIMER_DONE: i64 = 10;
/// `VAR_TIMER_IEC_DONE` — `%TM<n>.Q`.
pub(crate) const VAR_TIMER_IEC_DONE: i64 = 15;
/// `VAR_COUNTER_DONE` — `%C<n>.D`.
pub(crate) const VAR_COUNTER_DONE: i64 = 25;
/// `VAR_COUNTER_EMPTY` — `%C<n>.E`.
pub(crate) const VAR_COUNTER_EMPTY: i64 = 26;
/// `VAR_COUNTER_FULL` — `%C<n>.F`.
pub(crate) const VAR_COUNTER_FULL: i64 = 27;
/// `VAR_STEP_ACTIVITY` — `%X<n>.A`.
pub(crate) const VAR_STEP_ACTIVITY: i64 = 30;
/// `VAR_PHYS_INPUT` — `%I<n>`.
pub(crate) const VAR_PHYS_INPUT: i64 = 50;
/// `VAR_PHYS_OUTPUT` — `%Q<n>`.
pub(crate) const VAR_PHYS_OUTPUT: i64 = 60;
/// `VAR_USER_LED` — `%QLED<n>`.
pub(crate) const VAR_USER_LED: i64 = 65;
/// `VAR_SYSTEM` — `%S<n>`.
pub(crate) const VAR_SYSTEM: i64 = 70;
/// `VAR_REGISTER_EMPTY` — `%R<n>.E`.
pub(crate) const VAR_REGISTER_EMPTY: i64 = 80;
/// `VAR_REGISTER_FULL` — `%R<n>.F`.
pub(crate) const VAR_REGISTER_FULL: i64 = 81;
/// `VAR_MEM_WORD` — `%W<n>`, SoftLadder's `%MW<n>`.
pub(crate) const VAR_MEM_WORD: i64 = 200;
/// `VAR_STEP_TIME` — `%X<n>.V`.
pub(crate) const VAR_STEP_TIME: i64 = 220;
/// `VAR_TIMER_PRESET` — deprecated `%T<n>.P`.
pub(crate) const VAR_TIMER_PRESET: i64 = 230;
/// `VAR_TIMER_VALUE` — deprecated `%T<n>.V`.
pub(crate) const VAR_TIMER_VALUE: i64 = 231;
/// `VAR_MONOSTABLE_PRESET` — deprecated `%M<n>.P`.
pub(crate) const VAR_MONOSTABLE_PRESET: i64 = 240;
/// `VAR_MONOSTABLE_VALUE` — deprecated `%M<n>.V`.
pub(crate) const VAR_MONOSTABLE_VALUE: i64 = 241;
/// `VAR_COUNTER_PRESET` — `%C<n>.P`.
pub(crate) const VAR_COUNTER_PRESET: i64 = 250;
/// `VAR_COUNTER_VALUE` — `%C<n>.V`.
pub(crate) const VAR_COUNTER_VALUE: i64 = 251;
/// `VAR_TIMER_IEC_PRESET` — `%TM<n>.P`.
pub(crate) const VAR_TIMER_IEC_PRESET: i64 = 260;
/// `VAR_TIMER_IEC_VALUE` — `%TM<n>.V`.
pub(crate) const VAR_TIMER_IEC_VALUE: i64 = 261;
/// `VAR_PHYS_WORD_INPUT` — `%IW<n>`.
pub(crate) const VAR_PHYS_WORD_INPUT: i64 = 270;
/// `VAR_PHYS_WORD_OUTPUT` — `%QW<n>`.
pub(crate) const VAR_PHYS_WORD_OUTPUT: i64 = 280;
/// `VAR_REGISTER_IN_VALUE` — `%R<n>.I`.
pub(crate) const VAR_REGISTER_IN_VALUE: i64 = 300;
/// `VAR_REGISTER_OUT_VALUE` — `%R<n>.O`.
pub(crate) const VAR_REGISTER_OUT_VALUE: i64 = 301;
/// `VAR_REGISTER_NBR_VALUES` — `%R<n>.S`.
pub(crate) const VAR_REGISTER_NBR_VALUES: i64 = 302;

/// Base identifier from `timers_iec.csv`: 60 minutes.
pub(crate) const BASE_MINS: i64 = 0;
/// Base identifier from `timers_iec.csv`: one second.
pub(crate) const BASE_SECS: i64 = 1;
/// Base identifier from `timers_iec.csv`: 100 milliseconds.
pub(crate) const BASE_100MS: i64 = 2;

/// Mode identifier from `timers_iec.csv`: on-delay (TON).
pub(crate) const TIMER_MODE_ON: i64 = 0;
/// Mode identifier from `timers_iec.csv`: off-delay (TOF).
pub(crate) const TIMER_MODE_OFF: i64 = 1;
/// Mode identifier from `timers_iec.csv`: pulse (TP).
pub(crate) const TIMER_MODE_PULSE: i64 = 2;

/// Register mode from `registers.csv`: undefined (SoftLadder has no such mode).
pub(crate) const REGISTER_MODE_UNDEF: i64 = 0;
/// Register mode from `registers.csv`: FIFO.
pub(crate) const REGISTER_MODE_FIFO: i64 = 1;
/// Register mode from `registers.csv`: LIFO.
pub(crate) const REGISTER_MODE_LIFO: i64 = 2;

/// Durations of the three reference time bases, in milliseconds.
pub(crate) fn base_millis(base: i64) -> Option<i64> {
    match base {
        BASE_MINS => Some(60 * 60 * 1000),
        BASE_SECS => Some(1000),
        BASE_100MS => Some(100),
        _ => None,
    }
}

/// Splits a timer preset in the SoftLadder `params[0]` spelling into the
/// reference time base and the preset counted in that base.
///
/// `params[0]` is a duration in milliseconds, with an optional suffix that also
/// selects the time base (`docs/SEMANTICS.md` §3.5): `"500"` is 500 ms at the
/// 100 ms base, `"3s"` is 3 s and `"5m"` is 5 min. Returns `None` for anything
/// that is not a plain literal; a negative or unparsable value is reported as
/// no preset at all.
pub(crate) fn split_timer_preset(preset: &str) -> Option<(i64, i64)> {
    let text = preset.trim();
    let (digits, base) = match text.chars().last() {
        Some('s') | Some('S') => (&text[..text.len() - 1], BASE_SECS),
        Some('m') | Some('M') => (&text[..text.len() - 1], BASE_MINS),
        _ => (text, BASE_100MS),
    };
    let value: i64 = digits.trim().parse().ok()?;
    if value < 0 {
        return None;
    }
    let base_ms = base_millis(base)?;
    if base == BASE_100MS {
        // Match the scan engine: a positive duration shorter than one base
        // width still counts as a single unit.
        let units = value / base_ms;
        Some((base, if units == 0 && value > 0 { 1 } else { units }))
    } else {
        Some((base, value))
    }
}

/// Formats a preset and its reference time base as milliseconds, which is the
/// deprecated timer families' spelling (`preset * base_ms`).
pub(crate) fn timer_preset_millis(base: i64, preset: i64) -> String {
    base_millis(base)
        .and_then(|unit| unit.checked_mul(preset))
        .unwrap_or(0)
        .to_string()
}

/// Formats a preset and its reference time base as the SoftLadder `params[0]`
/// spelling, keeping both the value and the base so the pair survives a round
/// trip.
pub(crate) fn timer_preset_text(base: i64, preset: i64) -> String {
    match base {
        BASE_SECS => format!("{preset}s"),
        BASE_MINS => format!("{preset}m"),
        _ => base_millis(base)
            .and_then(|unit| unit.checked_mul(preset))
            .unwrap_or(0)
            .to_string(),
    }
}

/// Geometry of a multi-cell block: `(columns, rows)`, with the "alive" cell at
/// its top-right corner.
pub(crate) fn block_geometry(kind: &ElementKind) -> (u8, u8) {
    match kind {
        ElementKind::Timer { .. } => (2, 2),
        ElementKind::Counter { .. } => (2, 4),
        ElementKind::Register { .. } => (2, 3),
        ElementKind::Compare | ElementKind::Operate => (3, 1),
        _ => (1, 1),
    }
}

/// Outcome of decoding one ClassicLadder variable reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DecodedVar {
    /// A variable SoftLadder models directly.
    Plain(VarRef),
    /// A legacy variable SoftLadder models, but through a deprecated family
    /// (`%T<n>`, `%M<n>`): importing it deserves a `SL-W031` warning.
    Deprecated(VarRef),
    /// A variable family with no SoftLadder equivalent (`%SW<n>`, `%T<n>.R`).
    Unsupported,
}

/// Maps a ClassicLadder `(VarType, VarNum)` pair onto a SoftLadder variable.
pub(crate) fn decode_var(var_type: i64, var_num: i64) -> DecodedVar {
    let Some(index) = u32::try_from(var_num).ok() else {
        return DecodedVar::Unsupported;
    };
    let (kind, accessor, deprecated) = match var_type {
        VAR_MEM_BIT => (VarKind::MemBit, None, false),
        VAR_TIMER_DONE => (VarKind::TimerIec, Some(Accessor::Done), true),
        VAR_TIMER_IEC_DONE => (VarKind::TimerIec, Some(Accessor::Done), false),
        VAR_COUNTER_DONE => (VarKind::Counter, Some(Accessor::Done), false),
        VAR_COUNTER_EMPTY => (VarKind::Counter, Some(Accessor::Empty), false),
        VAR_COUNTER_FULL => (VarKind::Counter, Some(Accessor::Full), false),
        VAR_STEP_ACTIVITY => (VarKind::Step, Some(Accessor::Activity), false),
        VAR_PHYS_INPUT => (VarKind::PhysIn, None, false),
        VAR_PHYS_OUTPUT => (VarKind::PhysOut, None, false),
        VAR_USER_LED => (VarKind::Led, None, false),
        VAR_SYSTEM => (VarKind::System, None, false),
        VAR_REGISTER_EMPTY => (VarKind::Register, Some(Accessor::Empty), false),
        VAR_REGISTER_FULL => (VarKind::Register, Some(Accessor::Full), false),
        VAR_MEM_WORD => (VarKind::MemWord, None, false),
        VAR_STEP_TIME => (VarKind::Step, Some(Accessor::Value), false),
        VAR_TIMER_PRESET => (VarKind::TimerIec, Some(Accessor::Preset), true),
        VAR_TIMER_VALUE => (VarKind::TimerIec, Some(Accessor::Value), true),
        VAR_MONOSTABLE_PRESET => (VarKind::TimerIec, Some(Accessor::Preset), true),
        VAR_MONOSTABLE_VALUE => (VarKind::TimerIec, Some(Accessor::Value), true),
        VAR_COUNTER_PRESET => (VarKind::Counter, Some(Accessor::Preset), false),
        VAR_COUNTER_VALUE => (VarKind::Counter, Some(Accessor::Value), false),
        VAR_TIMER_IEC_PRESET => (VarKind::TimerIec, Some(Accessor::Preset), false),
        VAR_TIMER_IEC_VALUE => (VarKind::TimerIec, Some(Accessor::Value), false),
        VAR_PHYS_WORD_INPUT => (VarKind::PhysInWord, None, false),
        VAR_PHYS_WORD_OUTPUT => (VarKind::PhysOutWord, None, false),
        VAR_REGISTER_IN_VALUE => (VarKind::Register, Some(Accessor::In), false),
        VAR_REGISTER_OUT_VALUE => (VarKind::Register, Some(Accessor::Out), false),
        VAR_REGISTER_NBR_VALUES => (VarKind::Register, Some(Accessor::Count), false),
        // `VAR_WORD_SYSTEM` (290) and everything the reference has not defined:
        // SoftLadder has no storage class for it.
        _ => return DecodedVar::Unsupported,
    };
    let mut var = VarRef::new(kind, index);
    if let Some(accessor) = accessor {
        var = var.with_accessor(accessor);
    }
    if deprecated {
        DecodedVar::Deprecated(var)
    } else {
        DecodedVar::Plain(var)
    }
}

/// ClassicLadder `(VarType, VarNum)` pair for a SoftLadder variable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct EncodedVar {
    /// The `VarType` field of the cell or expression token.
    pub var_type: i64,
    /// The `VarNum` field.
    pub var_num: i64,
    /// Optional indirect index: `(IndexedVarType, IndexedVarNum)`.
    pub indexed: Option<(i64, i64)>,
}

/// Maps a SoftLadder variable onto the ClassicLadder numeric space.
///
/// Returns a human-readable reason when the reference cannot express the
/// variable (a bit accessor, a nested indirect index, an unknown storage
/// class), so the caller can raise `SL-W033`.
pub(crate) fn encode_var(var: &VarRef) -> Result<EncodedVar, String> {
    let (var_type, var_num) = encode_plain(var)?;
    let indexed = match &var.index_expr {
        None => None,
        Some(inner) => {
            if inner.index_expr.is_some() {
                return Err(format!("`{var}` has a nested indirect index"));
            }
            Some(encode_plain(inner)?)
        }
    };
    Ok(EncodedVar {
        var_type,
        var_num,
        indexed,
    })
}

/// Numeric pair for a variable with no indirect index.
fn encode_plain(var: &VarRef) -> Result<(i64, i64), String> {
    let num = i64::from(var.index);
    let accessor = var.effective_accessor();
    let var_type = match (var.kind, accessor) {
        (VarKind::MemBit, None) => VAR_MEM_BIT,
        (VarKind::PhysIn, None) => VAR_PHYS_INPUT,
        (VarKind::PhysOut, None) => VAR_PHYS_OUTPUT,
        (VarKind::Led, None) => VAR_USER_LED,
        (VarKind::System, None) => VAR_SYSTEM,
        (VarKind::MemWord, None) => VAR_MEM_WORD,
        (VarKind::PhysInWord, None) => VAR_PHYS_WORD_INPUT,
        (VarKind::PhysOutWord, None) => VAR_PHYS_WORD_OUTPUT,
        (VarKind::TimerIec, Some(Accessor::Done)) => VAR_TIMER_IEC_DONE,
        (VarKind::TimerIec, Some(Accessor::Preset)) => VAR_TIMER_IEC_PRESET,
        (VarKind::TimerIec, Some(Accessor::Value)) => VAR_TIMER_IEC_VALUE,
        (VarKind::Counter, Some(Accessor::Done)) => VAR_COUNTER_DONE,
        (VarKind::Counter, Some(Accessor::Empty)) => VAR_COUNTER_EMPTY,
        (VarKind::Counter, Some(Accessor::Full)) => VAR_COUNTER_FULL,
        (VarKind::Counter, Some(Accessor::Preset)) => VAR_COUNTER_PRESET,
        (VarKind::Counter, Some(Accessor::Value)) => VAR_COUNTER_VALUE,
        (VarKind::Register, Some(Accessor::Empty)) => VAR_REGISTER_EMPTY,
        (VarKind::Register, Some(Accessor::Full)) => VAR_REGISTER_FULL,
        (VarKind::Register, Some(Accessor::In)) => VAR_REGISTER_IN_VALUE,
        (VarKind::Register, Some(Accessor::Out)) => VAR_REGISTER_OUT_VALUE,
        (VarKind::Register, Some(Accessor::Count)) => VAR_REGISTER_NBR_VALUES,
        (VarKind::Step, Some(Accessor::Activity)) => VAR_STEP_ACTIVITY,
        (VarKind::Step, Some(Accessor::Value)) => VAR_STEP_TIME,
        _ => {
            return Err(format!(
                "`{var}` has no ClassicLadder spelling (bit access or unknown sub-value)"
            ))
        }
    };
    Ok((var_type, num))
}

/// SoftLadder timer mode for a `timers_iec.csv` mode field.
pub(crate) fn timer_mode(mode: i64) -> TimerMode {
    match mode {
        TIMER_MODE_OFF => TimerMode::Off,
        TIMER_MODE_PULSE => TimerMode::Pulse,
        _ => TimerMode::On,
    }
}

/// `timers_iec.csv` mode field for a SoftLadder timer mode.
pub(crate) fn timer_mode_code(mode: TimerMode) -> i64 {
    match mode {
        TimerMode::On => TIMER_MODE_ON,
        TimerMode::Off => TIMER_MODE_OFF,
        TimerMode::Pulse => TIMER_MODE_PULSE,
    }
}

/// SoftLadder register mode for a `registers.csv` mode field.
///
/// The reference's "undefined" mode has no SoftLadder equivalent; FIFO is used
/// and the caller warns (`SL-W030`).
pub(crate) fn register_mode(mode: i64) -> RegisterMode {
    match mode {
        REGISTER_MODE_LIFO => RegisterMode::Lifo,
        _ => RegisterMode::Fifo,
    }
}

/// `registers.csv` mode field for a SoftLadder register mode.
pub(crate) fn register_mode_code(mode: RegisterMode) -> i64 {
    match mode {
        RegisterMode::Fifo => REGISTER_MODE_FIFO,
        RegisterMode::Lifo => REGISTER_MODE_LIFO,
    }
}

/// SoftLadder counter kind imported from an `ELE_COUNTER` block.
///
/// The reference always honours both edges; the SoftLadder model has richer
/// kinds, but nothing in the file records which one the author wanted.
pub(crate) const IMPORTED_COUNTER_KIND: CounterKind = CounterKind::UpDown;
