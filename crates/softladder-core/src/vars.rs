//! Variable references (`%M0`, `%MW5`, `%I3`, …) and their ClassicLadder aliases.
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
//! Variables may also be indexed indirectly — `%MW[%MW0]` reads or writes the
//! word whose index is held in `%MW0` — and a bit may be selected inside a word
//! with a trailing `.n`, as in `%MW0.3`.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Storage class of a SoftLadder variable.
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
    /// IEC timer done bit (`%TM`).
    TimerIec,
    /// IEC timer elapsed value (`%TM<n>.V`).
    TimerIecValue,
    /// Counter done bit (`%C`).
    Counter,
    /// Counter current value (`%C<n>.V`).
    CounterValue,
    /// Register / FIFO–LIFO stack (`%R`).
    Register,
    /// Sequential step activity bit (`%X`).
    Step,
    /// System bit (`%S`).
    System,
    /// User LED / status lamp (`%QLED`).
    Led,
}

/// Prefix spellings accepted for each [`VarKind`], longest first.
const PREFIXES: &[(&str, VarKind)] = &[
    ("QLED", VarKind::Led),
    ("IW", VarKind::PhysInWord),
    ("QW", VarKind::PhysOutWord),
    ("TM", VarKind::TimerIec),
    ("MW", VarKind::MemWord),
    ("M", VarKind::MemBit),
    ("B", VarKind::MemBit),
    ("I", VarKind::PhysIn),
    ("Q", VarKind::PhysOut),
    ("C", VarKind::Counter),
    ("R", VarKind::Register),
    ("X", VarKind::Step),
    ("S", VarKind::System),
    ("W", VarKind::MemWord),
];

impl VarKind {
    /// Canonical mnemonic (without the leading `%`) used when displaying.
    pub fn mnemonic(self) -> &'static str {
        match self {
            VarKind::MemBit | VarKind::MemWord => "M",
            VarKind::PhysIn | VarKind::PhysInWord => "I",
            VarKind::PhysOut | VarKind::PhysOutWord => "Q",
            VarKind::TimerIec | VarKind::TimerIecValue => "TM",
            VarKind::Counter | VarKind::CounterValue => "C",
            VarKind::Register => "R",
            VarKind::Step => "X",
            VarKind::System => "S",
            VarKind::Led => "QLED",
        }
    }

    /// Suffix appended inside the prefix, matching [`VarKind::mnemonic`].
    fn mnemonic_suffix(self) -> &'static str {
        match self {
            VarKind::MemWord => "W",
            VarKind::PhysInWord => "W",
            VarKind::PhysOutWord => "W",
            _ => "",
        }
    }

    /// `true` for kinds that hold a single bit.
    pub fn is_bit(self) -> bool {
        matches!(
            self,
            VarKind::MemBit
                | VarKind::PhysIn
                | VarKind::PhysOut
                | VarKind::TimerIec
                | VarKind::Counter
                | VarKind::Step
                | VarKind::System
                | VarKind::Led
        )
    }

    /// `true` for kinds that hold a 32-bit integer.
    pub fn is_word(self) -> bool {
        !self.is_bit()
    }

    /// The `.V` companion of this kind, when it has one.
    pub fn value_variant(self) -> Option<VarKind> {
        match self {
            VarKind::TimerIec => Some(VarKind::TimerIecValue),
            VarKind::Counter => Some(VarKind::CounterValue),
            _ => None,
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
    /// Optional selected bit inside a word variable (`%MW0.3`).
    pub bit: Option<u8>,
}

impl VarRef {
    /// Creates a direct reference to `index` inside `kind`.
    pub fn new(kind: VarKind, index: u32) -> Self {
        Self {
            kind,
            index,
            index_expr: None,
            bit: None,
        }
    }

    /// Returns a copy of this reference with `bit` selected inside the word.
    pub fn with_bit(mut self, bit: u8) -> Self {
        self.bit = Some(bit);
        self
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
}

impl fmt::Display for VarRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "%")?;
        write!(f, "{}", self.kind.mnemonic())?;
        write!(f, "{}", self.kind.mnemonic_suffix())?;
        match &self.index_expr {
            Some(index) => write!(f, "[{index}]")?,
            None => write!(f, "{}", self.index)?,
        }
        if matches!(self.kind, VarKind::TimerIecValue | VarKind::CounterValue) {
            write!(f, ".V")?;
        }
        if let Some(bit) = self.bit {
            write!(f, ".{bit}")?;
        }
        Ok(())
    }
}

impl FromStr for VarRef {
    type Err = VarParseError;

    fn from_str(input: &str) -> Result<Self, VarParseError> {
        let rest = input
            .strip_prefix('%')
            .ok_or_else(|| VarParseError::new(input, "variable must start with '%'"))?;

        let (mut kind, rest) = PREFIXES
            .iter()
            .find(|(prefix, _)| rest.starts_with(prefix))
            .map(|(prefix, kind)| (*kind, &rest[prefix.len()..]))
            .ok_or_else(|| VarParseError::new(input, "unknown variable class"))?;

        let (index, index_expr, rest) = if let Some(inner) = rest.strip_prefix('[') {
            let end = inner
                .find(']')
                .ok_or_else(|| VarParseError::new(input, "unterminated '[' in index expression"))?;
            let inner_ref = inner[..end]
                .parse::<VarRef>()
                .map_err(|_| VarParseError::new(input, "invalid index expression"))?;
            (0, Some(Box::new(inner_ref)), &inner[end + 1..])
        } else {
            let digits = rest
                .find(|c: char| !c.is_ascii_digit())
                .unwrap_or(rest.len());
            if digits == 0 {
                return Err(VarParseError::new(input, "missing variable index"));
            }
            let index = rest[..digits]
                .parse::<u32>()
                .map_err(|_| VarParseError::new(input, "index out of range"))?;
            (index, None, &rest[digits..])
        };

        let mut bit = None;
        if let Some(suffix) = rest.strip_prefix('.') {
            if suffix.eq_ignore_ascii_case("V") {
                kind = kind.value_variant().ok_or_else(|| {
                    VarParseError::new(input, "this variable class has no '.V' value")
                })?;
            } else if !suffix.is_empty() && suffix.bytes().all(|b| b.is_ascii_digit()) {
                let selected = suffix
                    .parse::<u8>()
                    .map_err(|_| VarParseError::new(input, "bit selector out of range"))?;
                if selected > 31 {
                    return Err(VarParseError::new(input, "bit selector must be 0..=31"));
                }
                bit = Some(selected);
            } else {
                return Err(VarParseError::new(input, "unknown '.' suffix"));
            }
        } else if !rest.is_empty() {
            return Err(VarParseError::new(
                input,
                "trailing characters after the index",
            ));
        }

        if index_expr.is_some() && bit.is_some() {
            return Err(VarParseError::new(
                input,
                "a bit selector cannot be combined with an index expression",
            ));
        }

        Ok(VarRef {
            kind,
            index,
            index_expr,
            bit,
        })
    }
}

/// Error returned when a variable reference cannot be parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VarParseError {
    input: String,
    reason: &'static str,
}

impl VarParseError {
    fn new(input: &str, reason: &'static str) -> Self {
        Self {
            input: input.to_owned(),
            reason,
        }
    }

    /// The text that failed to parse.
    pub fn input(&self) -> &str {
        &self.input
    }

    /// A short static explanation of the failure.
    pub fn reason(&self) -> &'static str {
        self.reason
    }
}

impl fmt::Display for VarParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid variable reference `{}`: {}",
            self.input, self.reason
        )
    }
}

impl std::error::Error for VarParseError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(text: &str) -> VarRef {
        text.parse::<VarRef>().expect("variable should parse")
    }

    #[test]
    fn canonical_forms_round_trip() {
        for text in [
            "%M0", "%MW5", "%I3", "%Q2", "%IW1", "%QW2", "%TM0", "%TM7.V", "%C3", "%C3.V", "%R0",
            "%X4", "%S1", "%QLED0", "%M12.5", "%MW3.31",
        ] {
            let reference = parsed(text);
            assert_eq!(reference.to_string(), text, "round trip of {text}");
            assert_eq!(text.parse::<VarRef>().expect("parses").to_string(), text);
        }
    }

    #[test]
    fn classicladder_aliases_map_to_modern_kinds() {
        assert_eq!(parsed("%B0"), parsed("%M0"));
        assert_eq!(parsed("%W5"), parsed("%MW5"));
        assert_eq!(parsed("%W5").to_string(), "%MW5");
        assert_eq!(parsed("%B0").to_string(), "%M0");
        assert_eq!(parsed("%I0").kind, VarKind::PhysIn);
        assert_eq!(parsed("%Q1").kind, VarKind::PhysOut);
        assert_eq!(parsed("%IW2").kind, VarKind::PhysInWord);
        assert_eq!(parsed("%QW2").kind, VarKind::PhysOutWord);
        assert_eq!(parsed("%TM1").kind, VarKind::TimerIec);
        assert_eq!(parsed("%C1").kind, VarKind::Counter);
        assert_eq!(parsed("%R1").kind, VarKind::Register);
        assert_eq!(parsed("%X1").kind, VarKind::Step);
        assert_eq!(parsed("%S1").kind, VarKind::System);
    }

    #[test]
    fn indirect_and_bit_forms() {
        let indirect = parsed("%MW[%MW0]");
        assert!(indirect.is_indirect());
        assert_eq!(indirect.to_string(), "%MW[%MW0]");
        assert_eq!(indirect.kind, VarKind::MemWord);

        let bit = parsed("%M0.3");
        assert_eq!(bit.bit, Some(3));
        assert_eq!(bit.index, 0);

        let built = VarRef::new(VarKind::MemWord, 0)
            .with_index_var(VarRef::new(VarKind::MemWord, 1))
            .to_string();
        assert_eq!(built, "%MW[%MW1]");
    }

    #[test]
    fn malformed_input_is_rejected() {
        for text in [
            "",
            "M0",
            "%",
            "%Z3",
            "%M",
            "%Mabc",
            "%M0x",
            "%MW[%M0",
            "%MW[]",
            "%M0.",
            "%M0.Q",
            "%M0.99",
            "%C0.V.1",
            "%MW[%MW0].3",
        ] {
            assert!(
                text.parse::<VarRef>().is_err(),
                "`{text}` should not parse as a variable"
            );
        }
    }

    #[test]
    fn kind_predicates_are_consistent() {
        assert!(VarKind::MemBit.is_bit());
        assert!(!VarKind::MemBit.is_word());
        assert!(VarKind::MemWord.is_word());
        assert!(!VarKind::MemWord.is_bit());
        assert_eq!(
            VarKind::TimerIec.value_variant(),
            Some(VarKind::TimerIecValue)
        );
        assert_eq!(VarKind::Register.value_variant(), None);
    }
}
