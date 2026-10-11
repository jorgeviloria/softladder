//! Incident recording and exact replay — **M8 slice three**.
//!
//! Where [`run_scenario`](crate::run_scenario) checks a program against its
//! specification, recording captures *what a run actually did*: [`record`]
//! drives a scenario and samples the watched outputs after every scan, and
//! [`replay`] re-applies the recorded inputs to a project and compares the
//! outputs. An identical project replays clean; a changed one reports every
//! divergence with its cycle and variable — which is what makes a field
//! incident reproducible from its recording.

use serde::{Deserialize, Serialize};
use softladder_core::{Value, VarRef};

use crate::runtime::Runtime;
use crate::scenario::Scenario;

/// One recorded scan: the inputs applied and the outputs sampled.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecordedCycle {
    /// Scan cycle, counting from `0`.
    pub cycle: u64,
    /// Simulated timestamp of the scan.
    pub now_ms: u64,
    /// Inputs applied before the scan, in step order.
    pub inputs: Vec<(VarRef, Value)>,
    /// Watched outputs sampled after the scan, in watch order.
    pub outputs: Vec<(VarRef, Value)>,
}

/// A recorded run: inputs plus sampled outputs, cycle by cycle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Recording {
    /// Scenario the recording was taken from.
    pub name: String,
    /// Scan period the run used, in milliseconds.
    pub period_ms: u64,
    /// One entry per cycle, from `0` to the scenario's maximum.
    pub cycles: Vec<RecordedCycle>,
}

/// One divergence found by [`replay`].
#[derive(Debug, Clone, PartialEq)]
pub struct ReplayMismatch {
    /// Cycle that diverged.
    pub cycle: u64,
    /// Variable that diverged.
    pub var: VarRef,
    /// Value the recording holds.
    pub expected: Value,
    /// Value the replay produced (`None` when the variable is unknown).
    pub actual: Option<Value>,
}

/// Outcome of [`replay`].
#[derive(Debug, Clone, PartialEq)]
pub struct ReplayReport {
    /// Recording that was replayed.
    pub name: String,
    /// Cycles replayed.
    pub cycles: usize,
    /// Divergences, in cycle order.
    pub mismatches: Vec<ReplayMismatch>,
}

impl ReplayReport {
    /// `true` when the replay reproduced the recording exactly.
    pub fn matches(&self) -> bool {
        self.mismatches.is_empty()
    }
}

/// Drives `scenario` on a fresh runtime and records `watch` after every scan.
///
/// Inputs are the step values applied before each scan; unwritable steps are
/// skipped (they never reach the store, so there is nothing to replay).
pub fn record(
    project: &softladder_core::Project,
    scenario: &Scenario,
    watch: &[VarRef],
) -> Recording {
    let mut runtime = Runtime::new(project.clone());
    runtime.start();
    let period = runtime.period_ms;

    let mut steps = scenario.steps.clone();
    steps.sort_by_key(|step| step.cycle);
    let max = scenario.max_cycle();

    let mut cycles = Vec::new();
    let mut cursor = 0;
    for cycle in 0..=max {
        let mut inputs = Vec::new();
        while cursor < steps.len() && steps[cursor].cycle == cycle {
            let step = &steps[cursor];
            if runtime
                .engine
                .store_mut()
                .set(&step.var, step.value)
                .is_ok()
            {
                inputs.push((step.var.clone(), step.value));
            }
            cursor += 1;
        }
        let now_ms = cycle.saturating_mul(period);
        runtime.tick(now_ms);
        let outputs = watch
            .iter()
            .filter_map(|var| {
                runtime
                    .engine
                    .store()
                    .get(var)
                    .map(|value| (var.clone(), value))
            })
            .collect();
        cycles.push(RecordedCycle {
            cycle,
            now_ms,
            inputs,
            outputs,
        });
    }
    Recording {
        name: scenario.name.clone(),
        period_ms: period,
        cycles,
    }
}

/// Re-applies `recording` to `project` and compares the watched outputs.
///
/// `now_ms` advances exactly as recorded, so a deterministic engine replays an
/// identical project with zero mismatches; anything else is reported.
pub fn replay(project: &softladder_core::Project, recording: &Recording) -> ReplayReport {
    let mut runtime = Runtime::new(project.clone());
    runtime.start();

    let mut mismatches = Vec::new();
    for recorded in &recording.cycles {
        for (var, value) in &recorded.inputs {
            let _ = runtime.engine.store_mut().set(var, *value);
        }
        runtime.tick(recorded.now_ms);
        for (var, expected) in &recorded.outputs {
            let actual = runtime.engine.store().get(var);
            if actual != Some(*expected) {
                mismatches.push(ReplayMismatch {
                    cycle: recorded.cycle,
                    var: var.clone(),
                    expected: *expected,
                    actual,
                });
            }
        }
    }
    ReplayReport {
        name: recording.name.clone(),
        cycles: recording.cycles.len(),
        mismatches,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenario::ScenarioStep;
    use softladder_core::{ElementKind, PlacedElement, Project, Rung, Section};

    fn var(text: &str) -> VarRef {
        text.parse::<VarRef>().expect("test variable parses")
    }

    fn lamp_project() -> Project {
        let mut project = Project::new("replay test");
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

    fn scenario() -> Scenario {
        Scenario::new(
            "lamp".to_owned(),
            vec![
                ScenarioStep {
                    cycle: 1,
                    var: var("%I0"),
                    value: Value::Bit(true),
                },
                ScenarioStep {
                    cycle: 3,
                    var: var("%I0"),
                    value: Value::Bit(false),
                },
            ],
            vec![],
        )
    }

    #[test]
    fn an_identical_project_replays_clean() {
        let project = lamp_project();
        let recording = record(&project, &scenario(), &[var("%Q0"), var("%I0")]);
        assert_eq!(recording.cycles.len(), 4);
        assert_eq!(recording.cycles[0].inputs, vec![]);
        assert_eq!(
            recording.cycles[1].inputs,
            vec![(var("%I0"), Value::Bit(true))]
        );
        assert_eq!(
            recording.cycles[1].outputs,
            vec![
                (var("%Q0"), Value::Bit(true)),
                (var("%I0"), Value::Bit(true))
            ]
        );

        let report = replay(&project, &recording);
        assert!(report.matches());
        assert_eq!(report.cycles, 4);
        assert!(report.mismatches.is_empty());
    }

    #[test]
    fn a_tampered_output_is_reported_with_cycle_and_var() {
        let project = lamp_project();
        let mut recording = record(&project, &scenario(), &[var("%Q0")]);
        recording.cycles[2].outputs = vec![(var("%Q0"), Value::Bit(false))];
        let report = replay(&project, &recording);
        assert!(!report.matches());
        assert_eq!(report.mismatches.len(), 1);
        let mismatch = &report.mismatches[0];
        assert_eq!(mismatch.cycle, 2);
        assert_eq!(mismatch.var, var("%Q0"));
        assert_eq!(mismatch.expected, Value::Bit(false));
        assert_eq!(mismatch.actual, Some(Value::Bit(true)));
    }

    #[test]
    fn a_changed_program_diverges_from_its_recording() {
        let project = lamp_project();
        let recording = record(&project, &scenario(), &[var("%Q0")]);

        // Rewire the rung through a normally-closed contact: with the button
        // released the lamp now reads on where the recording reads off, so the
        // very first cycle already diverges.
        let mut changed = lamp_project();
        changed.rungs[0].elements[0] =
            PlacedElement::with_var(ElementKind::ContactNc, var("%I0"), 0, 0);
        let report = replay(&changed, &recording);
        assert!(!report.matches());
        assert_eq!(report.mismatches[0].cycle, 0);
    }

    #[test]
    fn recordings_round_trip_through_json() {
        let recording = record(&lamp_project(), &scenario(), &[var("%Q0")]);
        let json = serde_json::to_string_pretty(&recording).expect("serializes");
        let decoded: Recording = serde_json::from_str(&json).expect("deserializes");
        assert_eq!(decoded, recording);
    }
}
