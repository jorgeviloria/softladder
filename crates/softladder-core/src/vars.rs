//! Variable references (`%M0`, `%MW5`, `%I3`, …), their accessors and their
//! ClassicLadder aliases.
//!
//! SoftLadder addresses its data with the ClassicLadder `%`-notation. Two
//! families of prefixes are understood by [`VarRef::from_str`]:
//!
//! * the mnemonics used by modern projects — `%M`, `%MW`, `%I`, `%Q`, `%IW`,
//!   `%QW`, `%TM`, `%C`, `%R`, `%X`, `%S` and `%QLED`, and
//! * the historical short aliases that older ClassicLadder projects use for the
//!   very same storage — `%B` for `%M`, `%W` for `%MW`, and so on.
//!
//! The canonical spelling produced by [`std::fmt::Display`] is always the
//! modern mnemonic, so parsing `%B0` and printing it again yields `%M0`.
//!
//! Variables may be indexed indirectly — `%MW[%MW0]` reads or writes the word
//! whose index is held in `%MW0` — and structured variables expose sub-values
//! through an [`Accessor`]: `%TM0.Q` (timer output), `%TM0.V` (elapsed),
//! `%C1.D` (counter done), `%R0.I` (register input value), `%MW0.3` (bit 3 of a
//! word). The spellings follow ClassicLadder's own name table so that imported
//! projects keep working; see `docs/FORMAT.md` §"Accessors (schema v2)".

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// The system bit ClassicLadder uses for the shift and rotate carry (`%S8`).
///
/// `SHL`, `SHR`, `ROL` and `ROR` write the bit that left the operand here as they
/// evaluate; see [`crate::expr::EvalEffects`].
pub const SHIFT_CARRY_BIT: u32 = 8;

/// Storage class of a SoftLadder variable.
///
/// Kinds of variable the engine can address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum VarKind {
    /// Internal bit memory (`%M`). ClassicLadder alias: `%B`.
    MemBit,
    /// Internal word memory (`%MW`). ClassicLadder alias: `%W`.
    MemWord,
    /// Physical (hardware) input bit (`%I`).
    PhysIn,
    /// Physical (hardware) output bit (`%Q`).
    PhysOut,
    /// Physical input word (`%IW`).
    PhysInWord,
    /// Physical output word (`%QW`).
    PhysOutWord,
    /// IEC timer instance (`%TM`).
    TimerIec,
    /// Counter instance (`%C`).
    Counter,
    /// Register / FIFO–LIFO stack (`%R`).
    Register,
    /// Sequential step (`%X`).
    Step,
    /// System bit (`%S`).
    System,
    /// User LED / status lamp (`%QLED`).
    Led,
}

/// Sub-value selected inside a structured variable.
///
/// An accessor is only meaningful for the kinds listed in
/// [`VarKind::accepts_accessor`]; every other combination is a parse error, so
/// a malformed reference such as `%I3.P` can never reach the scan engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Accessor {
    /// `.V` — current value of a timer or counter, or the age of an SFC step.
    Value,
    /// `.P` — preset of a timer or counter.
    Preset,
    /// `.Q` / `.D` — output (timer) or done bit (counter). This is also the
    /// meaning of a bare `%TM0` / `%C0`.
    Done,
    /// `.E` — empty flag of a counter or register.
    Empty,
    /// `.F` — full flag of a counter or register.
    Full,
    /// `.I` — value pushed into a register.
    In,
    /// `.O` — value popped out of a register.
    Out,
    /// `.S` — number of values currently stored in a register.
    Count,
    /// `.A` — activity of an SFC step. Also the meaning of a bare `%X0`.
    Activity,
    /// `.n` — bit `n` (0–31) selected inside a word.
    Bit(u8),
}

impl Accessor {
    /// `true` when the accessor selects a single bit.
    pub fn is_bit(self) -> bool {
        matches!(
            self,
            Accessor::Done
                | Accessor::Empty
                | Accessor::Full
                | Accessor::Activity
                | Accessor::Bit(_)
        )
    }

    /// Letter used by the canonical spelling, or `None` for a bit selector.
    fn letter(self) -> Option<char> {
        match self {
            Accessor::Value => Some('V'),
            Accessor::Preset => Some('P'),
            Accessor::Done => Some('Q'),
            Accessor::Empty => Some('E'),
            Accessor::Full => Some('F'),
            Accessor::In => Some('I'),
            Accessor::Out => Some('O'),
            Accessor::Count => Some('S'),
            Accessor::Activity => Some('A'),
            Accessor::Bit(_) => None,
        }
    }
}

/// Canonical suffix letter for `accessor` on `kind`.
///
/// Timers spell their output `.Q` and counters their done bit `.D` (both parse
/// from either letter, for ClassicLadder compatibility), so the canonical
/// spelling has to look at the kind as well as the accessor.
fn accessor_letter(kind: VarKind, accessor: Accessor) -> Option<char> {
    match (kind, accessor) {
        (VarKind::TimerIec, Accessor::Done) => Some('Q'),
        (VarKind::Counter, Accessor::Done) => Some('D'),
        _ => accessor.letter(),
    }
}

/// Prefix spellings accepted when parsing, longest first.
const PREFIXES: &[(&str, VarKind)] = &[
    ("QLED", VarKind::Led),
    ("IW", VarKind::PhysInWord),
    ("QW", VarKind::PhysOutWord),
    ("MW", VarKind::MemWord),
    ("TM", VarKind::TimerIec),
    ("B", VarKind::MemBit),
    ("W", VarKind::MemWord),
    ("M", VarKind::MemBit),
    ("I", VarKind::PhysIn),
    ("Q", VarKind::PhysOut),
    ("C", VarKind::Counter),
    ("R", VarKind::Register),
    ("X", VarKind::Step),
    ("S", VarKind::System),
];

impl VarKind {
    /// Canonical mnemonic (without the leading `%`) used when displaying.
    pub fn mnemonic(self) -> &'static str {
        match self {
            VarKind::MemBit => "M",
            VarKind::MemWord => "MW",
            VarKind::PhysIn => "I",
            VarKind::PhysOut => "Q",
            VarKind::PhysInWord => "IW",
            VarKind::PhysOutWord => "QW",
            VarKind::TimerIec => "TM",
            VarKind::Counter => "C",
            VarKind::Register => "R",
            VarKind::Step => "X",
            VarKind::System => "S",
            VarKind::Led => "QLED",
        }
    }

    /// `true` when a variable of this kind holds a single bit by default.
    pub fn is_bit(self) -> bool {
        matches!(
            self,
            VarKind::MemBit
                | VarKind::PhysIn
                | VarKind::PhysOut
                | VarKind::TimerIec
                | VarKind::Counter
                | VarKind::Register
                | VarKind::Step
                | VarKind::System
                | VarKind::Led
        )
    }

    /// `true` when a variable of this kind holds a 32-bit integer by default.
    pub fn is_word(self) -> bool {
        !self.is_bit()
    }

    /// Accessor implied by a bare reference such as `%TM0` or `%X2`.
    ///
    /// Timers and counters imply their output/done bit (the reference engine
    /// treats `%TM0` and `%TM0.Q` as the same variable) and steps imply their
    /// activity. Everything else has no implied accessor.
    pub fn default_accessor(self) -> Option<Accessor> {
        match self {
            VarKind::TimerIec | VarKind::Counter => Some(Accessor::Done),
            VarKind::Step => Some(Accessor::Activity),
            _ => None,
        }
    }

    /// `true` when `accessor` may be applied to this kind.
    pub fn accepts_accessor(self, accessor: Accessor) -> bool {
        match accessor {
            Accessor::Value => matches!(self, VarKind::TimerIec | VarKind::Counter | VarKind::Step),
            Accessor::Preset => matches!(self, VarKind::TimerIec | VarKind::Counter),
            Accessor::Done => matches!(self, VarKind::TimerIec | VarKind::Counter),
            Accessor::Empty | Accessor::Full => {
                matches!(self, VarKind::Counter | VarKind::Register)
            }
            Accessor::In | Accessor::Out | Accessor::Count => matches!(self, VarKind::Register),
            Accessor::Activity => matches!(self, VarKind::Step),
            Accessor::Bit(_) => matches!(
                self,
                VarKind::MemWord | VarKind::PhysInWord | VarKind::PhysOutWord
            ),
        }
    }
}

/// A reference to a single SoftLadder variable.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct VarRef {
    /// Storage class of the referenced variable.
    pub kind: VarKind,
    /// Index inside the storage class.
    pub index: u32,
    /// Indirect index: when present, [`VarRef::index`] is zero and the real
    /// index is read from this variable at access time (`%MW[%MW0]`).
    pub index_expr: Option<Box<VarRef>>,
    /// Sub-value selected inside the variable (`%TM0.Q`, `%R0.I`, `%MW0.3`).
    /// `None` means the variable itself, which for timers, counters and steps
    /// still denotes their default bit (see [`VarKind::default_accessor`]).
    pub accessor: Option<Accessor>,
}

impl VarRef {
    /// Creates a direct reference to `index` inside `kind`.
    pub fn new(kind: VarKind, index: u32) -> Self {
        Self {
            kind,
            index,
            index_expr: None,
            accessor: None,
        }
    }

    /// Returns a copy of this reference with `accessor` selected.
    pub fn with_accessor(mut self, accessor: Accessor) -> Self {
        self.accessor = Some(accessor);
        self
    }

    /// Returns a copy of this reference with bit `bit` selected inside the word.
    pub fn with_bit(self, bit: u8) -> Self {
        self.with_accessor(Accessor::Bit(bit))
    }

    /// Returns a copy of this reference indexed by `index_var`.
    pub fn with_index_var(mut self, index_var: VarRef) -> Self {
        self.index = 0;
        self.index_expr = Some(Box::new(index_var));
        self
    }

    /// `true` when the index is computed at access time.
    pub fn is_indirect(&self) -> bool {
        self.index_expr.is_some()
    }

    /// Accessor after applying the kind's default, if any.
    pub fn effective_accessor(&self) -> Option<Accessor> {
        self.accessor.or_else(|| self.kind.default_accessor())
    }

    /// `true` when this reference selects a single bit.
    pub fn is_bit(&self) -> bool {
        match self.effective_accessor() {
            Some(accessor) => accessor.is_bit(),
            None => self.kind.is_bit(),
        }
    }

    /// `true` when this reference selects a 32-bit integer.
    pub fn is_word(&self) -> bool {
        !self.is_bit()
    }
}

impl fmt::Display for VarRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "%{}", self.kind.mnemonic())?;
        match &self.index_expr {
            Some(index) => write!(f, "[{index}]")?,
            None => write!(f, "{}", self.index)?,
        }
        if let Some(accessor) = self.accessor {
            if let Some(letter) = accessor_letter(self.kind, accessor) {
                write!(f, ".{letter}")?;
            }
        }
        // A timer, counter or step with no explicit accessor is displayed with
        // its implied one, so that the canonical form is never ambiguous.
        if self.accessor.is_none() {
            if let Some(implied) = self.kind.default_accessor() {
                if let Some(letter) = accessor_letter(self.kind, implied) {
                    write!(f, ".{letter}")?;
                }
            }
        }
        if let Some(Accessor::Bit(bit)) = self.accessor {
            write!(f, ".{bit}")?;
        }
        Ok(())
    }
}

/// Error returned when a variable reference cannot be parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VarParseError {
    input: String,
    reason: String,
}

impl VarParseError {
    fn new(input: &str, reason: impl Into<String>) -> Self {
        Self {
            input: input.to_owned(),
            reason: reason.into(),
        }
    }

    /// The text that failed to parse.
    pub fn input(&self) -> &str {
        &self.input
    }

    /// Why it failed.
    pub fn reason(&self) -> &str {
        &self.reason
    }
}

impl fmt::Display for VarParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid variable `{}`: {}", self.input, self.reason)
    }
}

impl std::error::Error for VarParseError {}

impl FromStr for VarRef {
    type Err = VarParseError;

    fn from_str(input: &str) -> Result<Self, VarParseError> {
        let rest = input
            .strip_prefix('%')
            .ok_or_else(|| VarParseError::new(input, "variable must start with '%'"))?;
        let (kind, rest) = split_kind(input, rest)?;
        let (index, index_expr, rest) = split_index(input, rest)?;
        // A bare timer, counter or step reference is normalized to its implied
        // accessor so that `%TM0` and `%TM0.Q` are the same value, not merely
        // equivalent.
        let accessor = split_accessor(input, kind, rest)?.or_else(|| kind.default_accessor());
        Ok(VarRef {
            kind,
            index,
            index_expr,
            accessor,
        })
    }
}

/// Splits the leading mnemonic off `rest`, longest match first.
fn split_kind<'a>(input: &str, rest: &'a str) -> Result<(VarKind, &'a str), VarParseError> {
    for (spelling, kind) in PREFIXES {
        if let Some(tail) = rest.strip_prefix(spelling) {
            return Ok((*kind, tail));
        }
    }
    Err(VarParseError::new(
        input,
        "unknown mnemonic; expected one of %M, %MW, %I, %Q, %IW, %QW, %TM, %C, %R, %X, %S, %QLED",
    ))
}

/// Parses either `[<var>]` or a decimal index.
fn split_index<'a>(
    input: &str,
    rest: &'a str,
) -> Result<(u32, Option<Box<VarRef>>, &'a str), VarParseError> {
    if let Some(inner) = rest.strip_prefix('[') {
        let end = inner
            .find(']')
            .ok_or_else(|| VarParseError::new(input, "missing ']' after the index variable"))?;
        let index_text = &inner[..end];
        let index_var: VarRef = index_text.parse().map_err(|error: VarParseError| {
            VarParseError::new(input, format!("bad index variable ({error})"))
        })?;
        return Ok((0, Some(Box::new(index_var)), &inner[end + 1..]));
    }
    let digits = rest
        .find(|character: char| !character.is_ascii_digit())
        .unwrap_or(rest.len());
    if digits == 0 {
        return Err(VarParseError::new(
            input,
            "expected a decimal index or an index variable",
        ));
    }
    let index: u32 = rest[..digits].parse().map_err(|_| {
        VarParseError::new(input, "the index must fit in a 32-bit unsigned integer")
    })?;
    Ok((index, None, &rest[digits..]))
}

/// Parses the optional `.X` / `.n` suffix and validates it against the kind.
fn split_accessor(
    input: &str,
    kind: VarKind,
    rest: &str,
) -> Result<Option<Accessor>, VarParseError> {
    if rest.is_empty() {
        return Ok(None);
    }
    let Some(suffix) = rest.strip_prefix('.') else {
        return Err(VarParseError::new(
            input,
            format!("unexpected `{rest}` after the index"),
        ));
    };
    if suffix.is_empty() {
        return Err(VarParseError::new(
            input,
            "missing accessor after '.' (expected .V, .P, .Q, .D, .E, .F, .I, .O, .S, .A or .n)",
        ));
    }
    let accessor = if suffix.chars().all(|character| character.is_ascii_digit()) {
        let bit: u8 = suffix
            .parse()
            .map_err(|_| VarParseError::new(input, "the bit selector must be 0..=31"))?;
        if bit > 31 {
            return Err(VarParseError::new(input, "the bit selector must be 0..=31"));
        }
        Accessor::Bit(bit)
    } else {
        let mut characters = suffix.chars();
        let letter = characters.next().unwrap_or_default();
        let trailing: String = characters.collect();
        if !trailing.is_empty() {
            return Err(VarParseError::new(
                input,
                format!("unexpected `{trailing}` after the accessor"),
            ));
        }
        match letter.to_ascii_uppercase() {
            'V' => Accessor::Value,
            'P' => Accessor::Preset,
            'Q' | 'D' => Accessor::Done,
            'E' => Accessor::Empty,
            'F' => Accessor::Full,
            'I' => Accessor::In,
            'O' => Accessor::Out,
            'S' => Accessor::Count,
            'A' => Accessor::Activity,
            other => {
                return Err(VarParseError::new(
                    input,
                    format!("unknown accessor `.{other}`"),
                ))
            }
        }
    };
    if !kind.accepts_accessor(accessor) {
        return Err(VarParseError::new(
            input,
            format!(
                "`{}` has no `{}` accessor",
                kind.mnemonic(),
                suffix.to_ascii_uppercase()
            ),
        ));
    }
    Ok(Some(accessor))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(text: &str) -> VarRef {
        text.parse()
            .unwrap_or_else(|error| panic!("`{text}` must parse: {error}"))
    }

    fn canonical(text: &str) -> String {
        parsed(text).to_string()
    }

    #[test]
    fn canonical_spellings_round_trip() {
        for text in [
            "%M0", "%MW5", "%I3", "%Q7", "%IW2", "%QW4", "%QLED0", "%S9", "%TM0.Q", "%TM0.V",
            "%TM0.P", "%C1.D", "%C1.E", "%C1.F", "%C1.P", "%C1.V", "%R0.E", "%R0.F", "%R0.I",
            "%R0.O", "%R0.S", "%X2.A", "%X2.V", "%MW0.3",
        ] {
            assert_eq!(canonical(text), text, "canonical form must be stable");
        }
    }

    #[test]
    fn classicladder_aliases_normalize_to_the_modern_mnemonic() {
        assert_eq!(canonical("%B0"), "%M0");
        assert_eq!(canonical("%W0"), "%MW0");
        assert_eq!(canonical("%B12"), "%M12");
        assert_eq!(canonical("%W12"), "%MW12");
    }

    #[test]
    fn bare_timer_counter_and_step_references_imply_their_default_accessor() {
        assert_eq!(canonical("%TM0"), "%TM0.Q");
        assert_eq!(canonical("%C0"), "%C0.D");
        assert_eq!(canonical("%X0"), "%X0.A");
        assert_eq!(parsed("%TM0").effective_accessor(), Some(Accessor::Done));
        assert_eq!(parsed("%TM0.Q"), parsed("%TM0"));
        assert_eq!(parsed("%C0"), parsed("%C0.D"));
    }

    #[test]
    fn accessors_are_rejected_on_kinds_that_do_not_have_them() {
        for text in [
            "%M0.I", "%M0.V", "%I3.P", "%Q1.S", "%IW0.O", "%S0.V", "%QLED0.E", "%R0.Q", "%R0.V",
            "%R0.A", "%X0.I", "%X0.D", "%C0.I", "%C0.A", "%TM0.E", "%TM0.S",
        ] {
            assert!(
                text.parse::<VarRef>().is_err(),
                "`{text}` must not parse as an accessor combination"
            );
        }
    }

    #[test]
    fn indirect_and_bit_forms() {
        let indirect = parsed("%MW[%MW0]");
        assert!(indirect.is_indirect());
        assert_eq!(indirect.index, 0);
        assert_eq!(indirect.to_string(), "%MW[%MW0]");
        assert_eq!(
            indirect.index_expr.as_deref(),
            Some(&VarRef::new(VarKind::MemWord, 0))
        );

        let bit = parsed("%MW0.3");
        assert_eq!(bit.accessor, Some(Accessor::Bit(3)));
        assert_eq!(bit.to_string(), "%MW0.3");

        let nested = parsed("%MW[%MW0.4]");
        assert_eq!(
            nested.index_expr.as_deref().and_then(|var| var.accessor),
            Some(Accessor::Bit(4))
        );
    }

    #[test]
    fn malformed_references_are_rejected() {
        for text in [
            "M0", "%", "%Z0", "%M", "%M-1", "%M0.", "%MW0.32", "%MW0.999", "%MW0.3.4", "%MW0:3",
            "%MW[%MW0", "%MW[]", "%MW[0]", "%M0 ", "%TM0.Z", "%MW0.Q", "%QLEDX",
        ] {
            assert!(text.parse::<VarRef>().is_err(), "`{text}` must be rejected");
        }
    }

    #[test]
    fn error_reports_the_input_and_a_reason() {
        let error = "%I3.P".parse::<VarRef>().expect_err("must fail");
        assert_eq!(error.input(), "%I3.P");
        assert!(error.reason().contains("accessor"), "{}", error.reason());
        assert!(error.to_string().contains("%I3.P"));
    }

    #[test]
    fn bit_and_word_classification_follows_the_accessor() {
        assert!(parsed("%M0").is_bit());
        assert!(parsed("%TM0.Q").is_bit());
        assert!(parsed("%TM0").is_bit());
        assert!(parsed("%C0.E").is_bit());
        assert!(parsed("%R0.F").is_bit());
        assert!(parsed("%X2.A").is_bit());
        assert!(parsed("%MW0.3").is_bit());
        // A bare register reference is the block's primary (empty) flag.
        assert!(parsed("%R0").is_bit());

        assert!(parsed("%MW5").is_word());
        assert!(parsed("%TM0.V").is_word());
        assert!(parsed("%C1.P").is_word());
        assert!(parsed("%R0.I").is_word());
        assert!(parsed("%R0.O").is_word());
        assert!(parsed("%R0.S").is_word());
        assert!(parsed("%X2.V").is_word());
        assert!(parsed("%QW0").is_word());
    }

    #[test]
    fn builders_produce_the_same_references_as_parsing() {
        assert_eq!(
            VarRef::new(VarKind::MemWord, 7).with_bit(3),
            parsed("%MW7.3")
        );
        assert_eq!(
            VarRef::new(VarKind::TimerIec, 2).with_accessor(Accessor::Preset),
            parsed("%TM2.P")
        );
        assert_eq!(
            VarRef::new(VarKind::MemWord, 0).with_index_var(VarRef::new(VarKind::MemWord, 1)),
            parsed("%MW[%MW1]")
        );
        assert_eq!(VarRef::new(VarKind::Counter, 4).to_string(), "%C4.D");
    }

    #[test]
    fn kind_predicates_describe_the_storage_class() {
        assert!(VarKind::MemBit.is_bit());
        assert!(VarKind::System.is_bit());
        assert!(VarKind::Led.is_bit());
        assert!(VarKind::MemWord.is_word());
        assert!(VarKind::PhysInWord.is_word());
        assert_eq!(VarKind::MemWord.mnemonic(), "MW");
        assert_eq!(VarKind::Led.mnemonic(), "QLED");
        assert_eq!(VarKind::Step.default_accessor(), Some(Accessor::Activity));
        assert_eq!(VarKind::MemBit.default_accessor(), None);
        assert!(VarKind::Register.accepts_accessor(Accessor::Count));
        assert!(!VarKind::Register.accepts_accessor(Accessor::Value));
        assert!(Accessor::Bit(0).is_bit());
        assert!(!Accessor::In.is_bit());
    }
}
