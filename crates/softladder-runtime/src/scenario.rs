//! `.sltest` scenarios — **M8 slice one (format + deterministic runner)**.
//!
//! A scenario is the PLC equivalent of a unit test: an input timeline plus
//! output assertions, both keyed by scan cycle. [`run_scenario`] drives a
//! fresh [`Runtime`] through cycle `0..=max`, applying each [`ScenarioStep`]
//! before its cycle's scan and checking each [`ScenarioAssert`] after it, with
//! `now_ms` advancing exactly one scan period per cycle — so the same project
//! and scenario always produce the same [`ScenarioResult`]. The CLI's `test`
//! subcommand (next slice) loads `.sltest` files with [`load_scenario`] and
//! maps the result to a process exit code.

use std::path::Path;

use serde::{Deserialize, Deserializer, Serialize};
use softladder_core::{Project, Value, VarRef};
use thiserror::Error;

use crate::runtime::Runtime;

/// Why a scenario file could not be loaded.
#[derive(Debug, Error)]
pub enum ScenarioError {
    /// The file could not be read.
    #[error("cannot read scenario `{path}`: {message}")]
    Io {
        /// File that was read.
        path: String,
        /// OS error.
        message: String,
    },
    /// The file is not a valid scenario.
    #[error("cannot parse scenario `{path}`: {message}")]
    Parse {
        /// File that was parsed.
        path: String,
        /// Parser error.
        message: String,
    },
}

/// One input change applied before its cycle's scan.
///
/// Files use the friendly form (`"var": "%I0"`, `"value": true`); the
/// structured serde form of [`VarRef`]/[`Value`] decodes too.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ScenarioStep {
    /// Scan cycle the change applies to, counting from `0`.
    pub cycle: u64,
    /// Variable to write (typically an input).
    pub var: VarRef,
    /// Value to write; coerced like any store write.
    pub value: Value,
}

impl<'de> Deserialize<'de> for ScenarioStep {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = StepForm::deserialize(deserializer)?;
        Ok(ScenarioStep {
            cycle: raw.cycle(),
            var: parse_var::<D>(raw.var())?,
            value: raw.value(),
        })
    }
}

/// One output check performed after its cycle's scan.
///
/// Same friendly file form as [`ScenarioStep`].
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ScenarioAssert {
    /// Scan cycle checked, counting from `0`.
    pub cycle: u64,
    /// Variable to read (typically an output).
    pub var: VarRef,
    /// Expected value.
    pub expected: Value,
}

impl<'de> Deserialize<'de> for ScenarioAssert {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = AssertForm::deserialize(deserializer)?;
        Ok(ScenarioAssert {
            cycle: raw.cycle(),
            var: parse_var::<D>(raw.var())?,
            expected: raw.value(),
        })
    }
}

/// A scalar in a friendly file: boolean, integer or real.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
enum FriendlyValue {
    /// A bit.
    Bool(bool),
    /// A word when it fits `i32`, a double word otherwise.
    Int(i64),
    /// A real.
    Float(f64),
}

impl From<FriendlyValue> for Value {
    fn from(value: FriendlyValue) -> Value {
        match value {
            FriendlyValue::Bool(bit) => Value::Bit(bit),
            FriendlyValue::Int(int) => i32::try_from(int)
                .map(Value::Word)
                .unwrap_or(Value::DWord(int)),
            FriendlyValue::Float(real) => Value::Real(real),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum StepForm {
    /// Structured serde form.
    Structured {
        /// Cycle.
        cycle: u64,
        /// Variable.
        var: VarRef,
        /// Value.
        value: Value,
    },
    /// Hand-written form.
    Friendly {
        /// Cycle.
        cycle: u64,
        /// Variable as text, e.g. `"%I0"`.
        var: String,
        /// Scalar value.
        value: FriendlyValue,
    },
}

impl StepForm {
    fn cycle(&self) -> u64 {
        match self {
            StepForm::Structured { cycle, .. } | StepForm::Friendly { cycle, .. } => *cycle,
        }
    }

    fn value(&self) -> Value {
        match self {
            StepForm::Structured { value, .. } => *value,
            StepForm::Friendly { value, .. } => Value::from(value.clone()),
        }
    }

    fn var(&self) -> Result<VarRef, String> {
        match self {
            StepForm::Structured { var, .. } => Ok(var.clone()),
            StepForm::Friendly { var, .. } => var.parse().map_err(|error| format!("{error}")),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum AssertForm {
    /// Structured serde form.
    Structured {
        /// Cycle.
        cycle: u64,
        /// Variable.
        var: VarRef,
        /// Expected value.
        expected: Value,
    },
    /// Hand-written form.
    Friendly {
        /// Cycle.
        cycle: u64,
        /// Variable as text.
        var: String,
        /// Expected scalar value.
        expected: FriendlyValue,
    },
}

impl AssertForm {
    fn cycle(&self) -> u64 {
        match self {
            AssertForm::Structured { cycle, .. } | AssertForm::Friendly { cycle, .. } => *cycle,
        }
    }

    fn value(&self) -> Value {
        match self {
            AssertForm::Structured { expected, .. } => *expected,
            AssertForm::Friendly { expected, .. } => Value::from(expected.clone()),
        }
    }

    fn var(&self) -> Result<VarRef, String> {
        match self {
            AssertForm::Structured { var, .. } => Ok(var.clone()),
            AssertForm::Friendly { var, .. } => var.parse().map_err(|error| format!("{error}")),
        }
    }
}

fn parse_var<'de, D: Deserializer<'de>>(
    parsed: Result<VarRef, String>,
) -> Result<VarRef, D::Error> {
    use serde::de::Error;
    parsed.map_err(D::Error::custom)
}

/// A `.sltest` scenario: timeline plus assertions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scenario {
    /// Human readable name, shown in reports.
    pub name: String,
    /// Input changes, in any order; sorted by cycle on run.
    #[serde(default)]
    pub steps: Vec<ScenarioStep>,
    /// Output checks, in any order.
    #[serde(default)]
    pub assertions: Vec<ScenarioAssert>,
}

impl Scenario {
    /// Creates a scenario from steps and assertions.
    pub fn new(name: String, steps: Vec<ScenarioStep>, assertions: Vec<ScenarioAssert>) -> Self {
        Self {
            name,
            steps,
            assertions,
        }
    }

    /// Highest cycle the run must reach.
    pub fn max_cycle(&self) -> u64 {
        self.steps
            .iter()
            .map(|step| step.cycle)
            .chain(self.assertions.iter().map(|assert| assert.cycle))
            .max()
            .unwrap_or(0)
    }
}

/// How one assertion failed.
#[derive(Debug, Clone, PartialEq)]
pub enum FailureKind {
    /// The step's write was refused (unknown or read-only variable).
    SetupError {
        /// Engine error.
        message: String,
    },
    /// The value after the scan differs.
    Mismatch {
        /// What the scenario wanted.
        expected: Value,
        /// What the store held (`None` when the variable is unknown).
        actual: Option<Value>,
    },
}

/// One failed step or assertion.
#[derive(Debug, Clone, PartialEq)]
pub struct ScenarioFailure {
    /// Cycle that failed.
    pub cycle: u64,
    /// Variable that failed.
    pub var: VarRef,
    /// How it failed.
    pub kind: FailureKind,
}

/// Outcome of [`run_scenario`].
#[derive(Debug, Clone, PartialEq)]
pub struct ScenarioResult {
    /// Scenario that ran.
    pub name: String,
    /// Highest cycle reached (inclusive).
    pub cycles: u64,
    /// Assertions checked.
    pub asserted: usize,
    /// Failures, in cycle order.
    pub failures: Vec<ScenarioFailure>,
}

impl ScenarioResult {
    /// `true` when every assertion held and every step applied.
    pub fn passed(&self) -> bool {
        self.failures.is_empty()
    }
}

/// Loads a `.sltest` JSON file.
pub fn load_scenario(path: &Path) -> Result<Scenario, ScenarioError> {
    let text = std::fs::read_to_string(path).map_err(|error| ScenarioError::Io {
        path: path.display().to_string(),
        message: error.to_string(),
    })?;
    serde_json::from_str(&text).map_err(|error| ScenarioError::Parse {
        path: path.display().to_string(),
        message: error.to_string(),
    })
}

/// Runs `scenario` against `project` deterministically.
pub fn run_scenario(project: &Project, scenario: &Scenario) -> ScenarioResult {
    let mut runtime = Runtime::new(project.clone());
    runtime.start();
    let period = runtime.period_ms;

    let mut steps = scenario.steps.clone();
    steps.sort_by_key(|step| step.cycle);
    let mut asserts = scenario.assertions.clone();
    asserts.sort_by_key(|assert| assert.cycle);

    let mut failures = Vec::new();
    let max = scenario.max_cycle();
    let mut step_cursor = 0;
    for cycle in 0..=max {
        while step_cursor < steps.len() && steps[step_cursor].cycle == cycle {
            let step = &steps[step_cursor];
            if let Err(error) = runtime.engine.store_mut().set(&step.var, step.value) {
                failures.push(ScenarioFailure {
                    cycle,
                    var: step.var.clone(),
                    kind: FailureKind::SetupError {
                        message: error.to_string(),
                    },
                });
            }
            step_cursor += 1;
        }
        let now_ms = cycle.saturating_mul(period);
        runtime.tick(now_ms);
        for assert in asserts.iter().filter(|assert| assert.cycle == cycle) {
            let actual = runtime.engine.store().get(&assert.var);
            if actual != Some(assert.expected) {
                failures.push(ScenarioFailure {
                    cycle,
                    var: assert.var.clone(),
                    kind: FailureKind::Mismatch {
                        expected: assert.expected,
                        actual,
                    },
                });
            }
        }
    }
    ScenarioResult {
        name: scenario.name.clone(),
        cycles: max,
        asserted: scenario.assertions.len(),
        failures,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use softladder_core::{ElementKind, PlacedElement, Rung, Section};

    fn var(text: &str) -> VarRef {
        text.parse::<VarRef>().expect("test variable parses")
    }

    fn lamp_project() -> Project {
        let mut project = Project::new("scenario test");
        let mut section = Section::new(1, "Main");
        section.rungs.push(1);
        project.sections.push(section);
        project.rungs.push(Rung {
            elements: vec![
                PlacedElement::with_var(ElementKind::ContactNo, var("%I0"), 0, 0),
                PlacedElement::with_var(ElementKind::CoilOut, var("%Q0"), 1, 0),
            ],
            ..Rung::new(1)
        });
        project
    }

    fn start_stop() -> Scenario {
        Scenario::new(
            "lamp follows the button".to_owned(),
            vec![
                ScenarioStep {
                    cycle: 0,
                    var: var("%I0"),
                    value: Value::Bit(false),
                },
                ScenarioStep {
                    cycle: 2,
                    var: var("%I0"),
                    value: Value::Bit(true),
                },
            ],
            vec![
                ScenarioAssert {
                    cycle: 0,
                    var: var("%Q0"),
                    expected: Value::Bit(false),
                },
                ScenarioAssert {
                    cycle: 1,
                    var: var("%Q0"),
                    expected: Value::Bit(false),
                },
                ScenarioAssert {
                    cycle: 2,
                    var: var("%Q0"),
                    expected: Value::Bit(true),
                },
            ],
        )
    }

    #[test]
    fn a_passing_scenario_checks_every_cycle() {
        let result = run_scenario(&lamp_project(), &start_stop());
        assert!(result.passed());
        assert_eq!(result.cycles, 2);
        assert_eq!(result.asserted, 3);
        assert!(result.failures.is_empty());
    }

    #[test]
    fn a_wrong_expectation_reports_cycle_var_and_values() {
        let mut scenario = start_stop();
        scenario.assertions[2].expected = Value::Bit(false);
        let result = run_scenario(&lamp_project(), &scenario);
        assert!(!result.passed());
        assert_eq!(result.failures.len(), 1);
        let failure = &result.failures[0];
        assert_eq!(failure.cycle, 2);
        assert_eq!(failure.var, var("%Q0"));
        assert_eq!(
            failure.kind,
            FailureKind::Mismatch {
                expected: Value::Bit(false),
                actual: Some(Value::Bit(true)),
            }
        );
    }

    #[test]
    fn an_unwritable_step_is_a_setup_error_not_a_panic() {
        let scenario = Scenario::new(
            "bad step".to_owned(),
            vec![ScenarioStep {
                cycle: 0,
                // A register's `.E` flag reads but never writes.
                var: "%R0.E".parse().expect("parses"),
                value: Value::Bit(true),
            }],
            vec![],
        );
        let result = run_scenario(&lamp_project(), &scenario);
        assert!(!result.passed());
        assert_eq!(result.failures.len(), 1);
        assert!(matches!(
            result.failures[0].kind,
            FailureKind::SetupError { .. }
        ));
    }

    #[test]
    fn friendly_files_decode_with_scalars() {
        let json = r#"{
            "name": "hand-written",
            "steps": [
                {"cycle": 0, "var": "%I0", "value": true},
                {"cycle": 1, "var": "%MW0", "value": 17},
                {"cycle": 2, "var": "%MW1", "value": 1.5}
            ],
            "assertions": [
                {"cycle": 2, "var": "%Q0", "expected": false}
            ]
        }"#;
        let scenario: Scenario = serde_json::from_str(json).expect("friendly decodes");
        assert_eq!(scenario.steps[0].value, Value::Bit(true));
        assert_eq!(scenario.steps[1].value, Value::Word(17));
        assert_eq!(scenario.steps[2].value, Value::Real(1.5));
        assert_eq!(scenario.assertions[0].expected, Value::Bit(false));

        let bad = r#"{"name":"bad","steps":[{"cycle":0,"var":"%ZZZ","value":true}]}"#;
        assert!(serde_json::from_str::<Scenario>(bad).is_err());
    }

    #[test]
    fn scenarios_round_trip_through_json_files() {
        let scenario = start_stop();
        let json = serde_json::to_string_pretty(&scenario).expect("serializes");
        let dir = std::env::temp_dir();
        let path = dir.join("softladder-scenario-round-trip.sltest");
        std::fs::write(&path, &json).expect("writes");
        let loaded = load_scenario(&path).expect("loads");
        assert_eq!(loaded, scenario);
        let _ = std::fs::remove_file(&path);

        assert!(matches!(
            load_scenario(std::path::Path::new("/definitely/not/here.sltest")),
            Err(ScenarioError::Io { .. })
        ));
        let bad = dir.join("softladder-scenario-bad.sltest");
        std::fs::write(&bad, "{not json").expect("writes");
        assert!(matches!(
            load_scenario(&bad),
            Err(ScenarioError::Parse { .. })
        ));
        let _ = std::fs::remove_file(&bad);
    }
}
