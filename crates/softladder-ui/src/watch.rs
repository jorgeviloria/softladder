//! Watch & force table state.
//!
//! Monitoring is the job: the table lists variables, shows their live values in
//! the chosen format, and offers a *modify* value and a *force* per row — the
//! monitor/modify columns every commissioning tool has. See `docs/UX.md` §7.

use softladder_core::{Value, VarRef};

/// How a monitored value is displayed and edited.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ValueFormat {
    /// `TRUE` / `FALSE`, or `1` / `0` for a word.
    #[default]
    Bool,
    /// A signed decimal integer.
    Signed,
    /// `0x1F` style.
    Hex,
    /// A 32-bit float view of the word.
    Real,
}

impl ValueFormat {
    /// Every format, for the format picker.
    pub const ALL: [ValueFormat; 4] = [
        ValueFormat::Bool,
        ValueFormat::Signed,
        ValueFormat::Hex,
        ValueFormat::Real,
    ];

    /// The label shown to the user.
    pub fn label(self) -> &'static str {
        match self {
            ValueFormat::Bool => "Bool",
            ValueFormat::Signed => "Signed",
            ValueFormat::Hex => "Hex",
            ValueFormat::Real => "Real",
        }
    }

    /// Formats `value` for display.
    pub fn display(self, value: Option<Value>) -> String {
        match (self, value) {
            (_, None) => "?".to_owned(),
            (ValueFormat::Bool, Some(value)) => {
                if value.as_i64() == 0 {
                    "FALSE".to_owned()
                } else {
                    "TRUE".to_owned()
                }
            }
            (ValueFormat::Signed, Some(value)) => value.as_i64().to_string(),
            (ValueFormat::Hex, Some(value)) => format!("0x{:04X}", value.as_i64() as u32 & 0xFFFF),
            (ValueFormat::Real, Some(value)) => format!("{:.3}", value.as_f64()),
        }
    }

    /// Parses a typed *modify* value, rejecting nonsense instead of guessing.
    pub fn parse(self, text: &str) -> Result<Value, String> {
        let text = text.trim();
        if text.is_empty() {
            return Err("enter a value".to_owned());
        }
        match self {
            ValueFormat::Bool => match text.to_ascii_uppercase().as_str() {
                "TRUE" | "1" | "ON" => Ok(Value::Bit(true)),
                "FALSE" | "0" | "OFF" => Ok(Value::Bit(false)),
                other => Err(format!("`{other}` is not TRUE/FALSE")),
            },
            ValueFormat::Signed => text
                .parse::<i32>()
                .map(Value::Word)
                .map_err(|_| format!("`{text}` is not a signed integer")),
            ValueFormat::Hex => {
                let digits = text
                    .strip_prefix("0x")
                    .or_else(|| text.strip_prefix("0X"))
                    .or_else(|| text.strip_prefix('$'))
                    .unwrap_or(text);
                i32::from_str_radix(digits, 16)
                    .map(Value::Word)
                    .map_err(|_| format!("`{text}` is not hexadecimal"))
            }
            ValueFormat::Real => text
                .parse::<f64>()
                .map(Value::Real)
                .map_err(|_| format!("`{text}` is not a number")),
        }
    }
}

/// One row of the watch table.
#[derive(Debug, Clone, PartialEq)]
pub struct WatchRow {
    /// The monitored variable.
    pub var: VarRef,
    /// How its value is shown.
    pub format: ValueFormat,
    /// Text typed into the *modify* column.
    pub modify: String,
    /// The forced value, while a force is active on this row.
    pub force: Option<bool>,
}

impl WatchRow {
    /// A row monitoring `var` in the default format.
    pub fn new(var: VarRef) -> Self {
        Self {
            var,
            format: ValueFormat::default(),
            modify: String::new(),
            force: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn var(text: &str) -> VarRef {
        text.parse().expect("variable parses")
    }

    #[test]
    fn every_format_round_trips_a_word() {
        assert_eq!(ValueFormat::Signed.display(Some(Value::Word(-42))), "-42");
        assert_eq!(ValueFormat::Hex.display(Some(Value::Word(31))), "0x001F");
        assert_eq!(ValueFormat::Bool.display(Some(Value::Word(0))), "FALSE");
        assert_eq!(ValueFormat::Bool.display(Some(Value::Word(7))), "TRUE");
        assert_eq!(ValueFormat::Real.display(Some(Value::Real(1.5))), "1.500");
        assert_eq!(ValueFormat::Signed.display(None), "?");
    }

    #[test]
    fn modify_values_are_parsed_strictly() {
        assert_eq!(ValueFormat::Bool.parse("TRUE"), Ok(Value::Bit(true)));
        assert_eq!(ValueFormat::Bool.parse(" off "), Ok(Value::Bit(false)));
        assert!(ValueFormat::Bool.parse("maybe").is_err());
        assert_eq!(ValueFormat::Signed.parse("-7"), Ok(Value::Word(-7)));
        assert!(ValueFormat::Signed.parse("7.5").is_err());
        assert_eq!(ValueFormat::Hex.parse("0x1F"), Ok(Value::Word(31)));
        assert_eq!(ValueFormat::Hex.parse("$1F"), Ok(Value::Word(31)));
        assert!(ValueFormat::Hex.parse("zz").is_err());
        assert_eq!(ValueFormat::Real.parse("2.5"), Ok(Value::Real(2.5)));
        assert!(ValueFormat::Real.parse("").is_err());
    }

    #[test]
    fn the_format_labels_are_unique() {
        let mut labels: Vec<&str> = ValueFormat::ALL.iter().map(|f| f.label()).collect();
        let count = labels.len();
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), count, "two formats share a label");
        assert_eq!(ValueFormat::default(), ValueFormat::Bool);
    }

    #[test]
    fn a_new_row_starts_empty_and_unforced() {
        let row = WatchRow::new(var("%I0"));
        assert_eq!(row.var, var("%I0"));
        assert_eq!(row.format, ValueFormat::Bool);
        assert!(row.modify.is_empty());
        assert_eq!(row.force, None);
    }
}
