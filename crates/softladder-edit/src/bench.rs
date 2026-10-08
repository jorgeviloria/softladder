//! The simulation bench: a running program plus the operator's bench panel.
//!
//! A [`Bench`] pairs the project's [`SimulationPanel`] (the widget layout, which
//! belongs to the project) with a [`PanelState`] (the operator's switch and
//! slider positions, which never do). All scans run through
//! [`Clock::simulated`], so stepping a bench is deterministic: the same clicks
//! in the same order always produce the same outputs.

use softladder_core::{
    Diagnostic, PanelState, Project, ScanEngine, ScanReport, SimReading, SimulationPanel,
};
use softladder_runtime::{Clock, Runtime, RuntimeState};

/// A deterministic simulation bench for `project`.
#[derive(Debug)]
pub struct Bench {
    /// The supervisor that owns the project, the engine and the run state.
    runtime: Runtime,
    /// Bench layout currently displayed.
    panel: SimulationPanel,
    /// Operator positions of `panel`.
    state: PanelState,
    /// Deterministic clock; cycle `k` runs at `(k - 1) * period_ms`.
    clock: Clock,
    /// Last output readings, refreshed after every scan.
    readings: Vec<SimReading>,
    /// Diagnostics of the last scan.
    diagnostics: Vec<Diagnostic>,
}

impl Bench {
    /// Builds a bench for `project`; the runtime starts in [`RuntimeState::Stop`].
    pub fn new(project: Project) -> Self {
        let panel = project.simulation.clone();
        let clock = Clock::simulated(u64::from(project.scan.period_ms));
        let runtime = Runtime::new(project);
        let state = PanelState::new(&panel);
        let readings = state.read_outputs(&panel, runtime.engine.store());
        Self {
            runtime,
            panel,
            state,
            clock,
            readings,
            diagnostics: Vec::new(),
        }
    }

    /// The runtime supervisor, including the scan engine and its store.
    pub fn runtime(&self) -> &Runtime {
        &self.runtime
    }

    /// The runtime supervisor, mutably.
    pub fn runtime_mut(&mut self) -> &mut Runtime {
        &mut self.runtime
    }

    /// The scan engine executing the project.
    pub fn engine(&self) -> &ScanEngine {
        &self.runtime.engine
    }

    /// The scan engine executing the project, mutably.
    pub fn engine_mut(&mut self) -> &mut ScanEngine {
        &mut self.runtime.engine
    }

    /// The operator's positions.
    pub fn panel_state(&self) -> &PanelState {
        &self.state
    }

    /// The operator's positions, mutably.
    pub fn panel_state_mut(&mut self) -> &mut PanelState {
        &mut self.state
    }

    /// Replaces the bench panel, keeping the positions of widgets that survive
    /// at the same index ([`PanelState::sync`]).
    pub fn set_panel(&mut self, panel: SimulationPanel) {
        self.panel = panel;
        self.state.sync(&self.panel);
        self.readings = self
            .state
            .read_outputs(&self.panel, self.runtime.engine.store());
    }

    /// Runs one bench cycle, or returns [`None`] when the runtime is not
    /// scanning (stopped, frozen or loading).
    ///
    /// The order is fixed and matches a real bench: apply the operator's inputs,
    /// scan once at the simulated timestamp, release the momentary buttons, then
    /// read the outputs back.
    pub fn step(&mut self) -> Option<ScanReport> {
        if !self.runtime.state.is_scanning() {
            return None;
        }
        let cycle = self.runtime.cycles.saturating_add(1);
        let now_ms = self.clock.next_now_ms(cycle);

        let store = self.runtime.engine.store_mut();
        self.state.apply_inputs(&self.panel, store);

        let Some(report) = self.runtime.tick(now_ms) else {
            // The state stopped scanning between the check and the tick; the
            // buttons must still spring back so the bench never sticks.
            self.state.release_momentary(&self.panel);
            return None;
        };

        self.state.release_momentary(&self.panel);
        self.readings = self
            .state
            .read_outputs(&self.panel, self.runtime.engine.store());
        self.diagnostics = report.diagnostics.clone();
        Some(report)
    }

    /// Puts the bench in [`RuntimeState::Run`], restarting the scan engine.
    ///
    /// The operator's positions are kept, but the variable store and the
    /// simulated clock start again from the beginning of the program.
    pub fn start(&mut self) {
        self.runtime.start();
        self.clock = Clock::simulated(u64::from(self.runtime.project.scan.period_ms));
        self.diagnostics.clear();
        self.readings = self
            .state
            .read_outputs(&self.panel, self.runtime.engine.store());
    }

    /// Puts the bench in [`RuntimeState::Stop`], keeping the variables.
    pub fn stop(&mut self) {
        self.runtime.stop();
    }

    /// Requests exactly one scan, then a return to [`RuntimeState::Stop`].
    pub fn run_one_cycle(&mut self) {
        self.runtime.run_one_cycle();
    }

    /// Current lifecycle state.
    pub fn state(&self) -> RuntimeState {
        self.runtime.state
    }

    /// The latest lamp and gauge readings, in panel order.
    pub fn readings(&self) -> &[SimReading] {
        &self.readings
    }

    /// Number of scans performed since the last [`Bench::start`].
    pub fn cycles(&self) -> u64 {
        self.runtime.cycles
    }

    /// Diagnostics produced by the last [`Bench::step`].
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Hot-reloads the program at a cycle boundary.
    ///
    /// The store, the run state and the cycle counter are kept, so timers,
    /// counters and outputs continue where they were; only the program changes.
    /// The bench panel comes from the new project and the operator's positions
    /// survive for the widgets that still exist ([`PanelState::sync`]).
    pub fn reload(&mut self, project: Project) {
        self.panel = project.simulation.clone();
        self.state.sync(&self.panel);
        self.clock = Clock::simulated(u64::from(project.scan.period_ms));
        self.runtime.project = project.clone();
        *self.runtime.engine.project_mut() = project;
        // The cell index caches geometry and block spans; a project swap with
        // the same element count would otherwise keep a stale index.
        self.runtime.engine.refresh();
        self.readings = self
            .state
            .read_outputs(&self.panel, self.runtime.engine.store());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use softladder_core::{
        ElementKind, PlacedElement, Rung, ScanConfig, Section, SimAnalog, SimGauge, SimLamp,
        SimSwitch, TimerMode, Value, VarRef,
    };

    fn var(text: &str) -> VarRef {
        text.parse().expect("test variable parses")
    }

    /// One rung: `%I0` drives `%Q0`, with a toggle and a lamp on the bench.
    fn lamp_project() -> Project {
        let mut project = Project::new("bench tests");
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
        project.simulation = SimulationPanel {
            switches: vec![SimSwitch {
                var: var("%I0"),
                label: "start".to_owned(),
                momentary: false,
            }],
            lamps: vec![SimLamp {
                var: var("%Q0"),
                label: "green".to_owned(),
            }],
            ..SimulationPanel::default()
        };
        project
    }

    #[test]
    fn a_new_bench_starts_stopped_with_readings_ready() {
        let bench = Bench::new(lamp_project());
        assert_eq!(bench.state(), RuntimeState::Stop);
        assert_eq!(bench.cycles(), 0);
        assert_eq!(bench.readings().len(), 1);
        assert_eq!(bench.readings()[0].label, "green");
        assert_eq!(bench.readings()[0].value, Value::Bit(false));
        assert!(bench.diagnostics().is_empty());
    }

    #[test]
    fn a_toggle_energises_an_output_after_one_step() {
        let mut bench = Bench::new(lamp_project());
        bench.start();
        assert_eq!(bench.state(), RuntimeState::Run);

        bench.panel_state_mut().toggle(0);
        let report = bench.step().expect("the running bench scans");
        assert!(!report.stopped_for_mad_loop);
        assert!(report.diagnostics.is_empty());
        assert_eq!(bench.cycles(), 1);
        assert_eq!(bench.readings()[0].value, Value::Bit(true));
        assert_eq!(
            bench.engine().store().get(&var("%Q0")),
            Some(Value::Bit(true))
        );

        bench.panel_state_mut().toggle(0);
        bench.step().expect("scans again");
        assert_eq!(bench.readings()[0].value, Value::Bit(false));
        assert_eq!(bench.cycles(), 2);
    }

    #[test]
    fn a_momentary_button_is_low_again_on_the_next_step() {
        let mut project = lamp_project();
        project.simulation.switches[0].momentary = true;
        let mut bench = Bench::new(project);
        bench.start();
        bench.panel_state_mut().set_closed(0, true);

        bench.step().expect("scans");
        assert_eq!(bench.readings()[0].value, Value::Bit(true));
        assert!(
            !bench.panel_state().is_closed(0),
            "the button springs back after the scan"
        );

        bench.step().expect("scans again");
        assert_eq!(bench.readings()[0].value, Value::Bit(false));
    }

    #[test]
    fn analog_sliders_drive_the_input_words_and_clamp() {
        let mut project = lamp_project();
        project.simulation.analogs = vec![SimAnalog {
            var: var("%IW0"),
            label: "dial".to_owned(),
            min: 0,
            max: 1000,
        }];
        let mut bench = Bench::new(project);
        bench.start();

        let panel = bench.runtime().project.simulation.clone();
        bench.panel_state_mut().set_analog(&panel, 0, 700);
        bench.step().expect("scans");
        assert_eq!(
            bench.engine().store().get(&var("%IW0")),
            Some(Value::Word(700))
        );

        bench.panel_state_mut().set_analog(&panel, 0, 99_999);
        bench.step().expect("scans again");
        assert_eq!(
            bench.engine().store().get(&var("%IW0")),
            Some(Value::Word(1000)),
            "the slider clamps into its range"
        );
    }

    #[test]
    fn readings_follow_the_outputs_including_analog_gauges() {
        let mut project = lamp_project();
        project.rungs[0].elements.push(PlacedElement::with_params(
            ElementKind::Operate,
            2,
            0,
            &["%QW0", "=", "42"],
        ));
        project.simulation.gauges = vec![SimGauge {
            var: var("%QW0"),
            label: "meter".to_owned(),
            min: 0,
            max: 100,
        }];
        let mut bench = Bench::new(project);
        bench.start();
        bench.panel_state_mut().toggle(0);
        bench.step().expect("scans");

        let readings = bench.readings();
        assert_eq!(readings.len(), 2);
        assert_eq!(readings[0].var, var("%Q0"));
        assert_eq!(readings[0].label, "green");
        assert_eq!(readings[0].value, Value::Bit(true));
        assert_eq!(readings[1].var, var("%QW0"));
        assert_eq!(readings[1].label, "meter");
        assert_eq!(readings[1].value, Value::Word(42));
    }

    #[test]
    fn the_simulated_clock_advances_by_exactly_one_period_per_step() {
        // A TON with a 100 ms preset and a 100 ms scan period expires after
        // exactly two steps: the first runs at t = 0 (delta 0), the second at
        // t = 100 (delta 100). Any other step size would change the result.
        let mut project = Project::new("clock");
        project.scan = ScanConfig {
            period_ms: 100,
            input_period_ms: 10,
        };
        let mut section = Section::new(1, "Main");
        section.rungs.push(1);
        project.sections.push(section);
        project.rungs.push(Rung {
            elements: vec![
                PlacedElement::with_var(ElementKind::ContactNo, var("%I0"), 0, 0),
                PlacedElement::with_params(
                    ElementKind::Timer {
                        mode: TimerMode::On,
                    },
                    1,
                    0,
                    &["100"],
                ),
                PlacedElement::with_var(ElementKind::CoilOut, var("%Q0"), 2, 0),
            ],
            ..Rung::new(1)
        });
        project.rungs[0].elements[1].var = Some(var("%TM0"));
        project.simulation = SimulationPanel {
            switches: vec![SimSwitch {
                var: var("%I0"),
                label: "enable".to_owned(),
                momentary: false,
            }],
            lamps: vec![SimLamp {
                var: var("%Q0"),
                label: "done".to_owned(),
            }],
            ..SimulationPanel::default()
        };

        let mut bench = Bench::new(project);
        bench.start();
        bench.panel_state_mut().toggle(0);

        bench.step().expect("first scan at t = 0");
        assert_eq!(
            bench.readings()[0].value,
            Value::Bit(false),
            "delta 0 has not counted a time base unit yet"
        );

        bench.step().expect("second scan at t = 100");
        assert_eq!(
            bench.readings()[0].value,
            Value::Bit(true),
            "exactly one 100 ms period elapsed, so the timer expired"
        );
        assert_eq!(bench.cycles(), 2);
    }

    #[test]
    fn a_stopped_bench_does_not_scan_but_run_one_cycle_does() {
        let mut bench = Bench::new(lamp_project());
        assert!(bench.step().is_none());
        assert_eq!(bench.cycles(), 0);

        bench.run_one_cycle();
        assert_eq!(bench.state(), RuntimeState::RunOneCycle);
        assert!(bench.step().is_some());
        assert_eq!(bench.cycles(), 1);
        assert_eq!(bench.state(), RuntimeState::Stop, "one cycle, then stop");
        assert!(bench.step().is_none());

        bench.start();
        assert!(bench.step().is_some());
        bench.stop();
        assert_eq!(bench.state(), RuntimeState::Stop);
        assert!(bench.step().is_none());
        assert_eq!(bench.cycles(), 1, "start restarted the cycle counter");
    }

    #[test]
    fn start_resets_the_store_but_keeps_the_operator_positions() {
        let mut bench = Bench::new(lamp_project());
        bench.start();
        bench.panel_state_mut().toggle(0);
        bench.step().expect("scans");
        assert_eq!(bench.readings()[0].value, Value::Bit(true));

        bench.stop();
        bench.start();
        assert_eq!(bench.cycles(), 0, "start restarts the cycle counter");
        assert!(bench.panel_state().is_closed(0), "the switch stays closed");
        assert_eq!(
            bench.engine().store().get(&var("%Q0")),
            Some(Value::Bit(false)),
            "the store restarts from a clean state"
        );
        assert_eq!(bench.readings()[0].value, Value::Bit(false));
    }

    #[test]
    fn reload_keeps_operator_positions_and_runs_the_new_program() {
        let project = lamp_project();
        let mut bench = Bench::new(project.clone());
        bench.start();
        bench.panel_state_mut().toggle(0);
        bench.step().expect("scans");
        assert_eq!(bench.readings()[0].value, Value::Bit(true));

        let mut edited = project.clone();
        edited.rungs[0].elements[1].var = Some(var("%Q1"));
        edited.simulation.lamps[0].var = var("%Q1");
        bench.reload(edited.clone());

        assert!(bench.panel_state().is_closed(0), "the position survives");
        assert_eq!(bench.runtime().project, edited);
        assert_eq!(bench.state(), RuntimeState::Run, "reload keeps running");
        bench.step().expect("scans the new program");
        assert_eq!(
            bench.engine().store().get(&var("%Q1")),
            Some(Value::Bit(true))
        );
        assert_eq!(bench.readings()[0].var, var("%Q1"));
        assert_eq!(bench.readings()[0].value, Value::Bit(true));
    }

    #[test]
    fn reload_adds_new_widgets_open_and_keeps_the_old_ones() {
        let project = lamp_project();
        let mut bench = Bench::new(project.clone());
        bench.start();
        bench.panel_state_mut().toggle(0);

        let mut edited = project.clone();
        edited.simulation.switches.push(SimSwitch {
            var: var("%I1"),
            label: "second".to_owned(),
            momentary: true,
        });
        bench.reload(edited);

        assert!(bench.panel_state().is_closed(0));
        assert!(
            !bench.panel_state().is_closed(1),
            "a new switch starts open"
        );
    }

    #[test]
    fn set_panel_keeps_positions_and_refreshes_readings() {
        let mut bench = Bench::new(lamp_project());
        bench.start();
        bench.panel_state_mut().toggle(0);

        bench.set_panel(SimulationPanel {
            switches: vec![
                SimSwitch {
                    var: var("%I0"),
                    label: "start".to_owned(),
                    momentary: false,
                },
                SimSwitch {
                    var: var("%I1"),
                    label: "extra".to_owned(),
                    momentary: false,
                },
            ],
            lamps: vec![SimLamp {
                var: var("%Q0"),
                label: "green".to_owned(),
            }],
            ..SimulationPanel::default()
        });
        assert!(bench.panel_state().is_closed(0));
        assert!(!bench.panel_state().is_closed(1));
        assert_eq!(bench.readings().len(), 1);
    }

    #[test]
    fn bench_diagnostics_report_scan_errors() {
        let mut project = Project::new("divide by zero");
        let mut section = Section::new(1, "Main");
        section.rungs.push(1);
        project.sections.push(section);
        project.rungs.push(Rung {
            elements: vec![PlacedElement::with_params(
                ElementKind::Compare,
                0,
                0,
                &["1 / 0"],
            )],
            ..Rung::new(1)
        });

        let mut bench = Bench::new(project);
        bench.start();
        let report = bench.step().expect("scans");
        assert!(report.diagnostics.iter().any(|d| d.code == "SL-E002"));
        assert!(bench.diagnostics().iter().any(|d| d.code == "SL-E002"));
    }

    #[test]
    fn mutable_access_reaches_the_runtime_engine_and_panel() {
        let mut bench = Bench::new(lamp_project());
        assert_eq!(bench.runtime().state, RuntimeState::Stop);
        bench.runtime_mut().run_one_cycle();
        assert_eq!(bench.state(), RuntimeState::RunOneCycle);

        let store = bench.engine_mut().store_mut();
        assert!(store.set(&var("%M0"), Value::Bit(true)).is_ok());
        assert_eq!(
            bench.engine().store().get(&var("%M0")),
            Some(Value::Bit(true))
        );

        bench.panel_state_mut().set_closed(0, true);
        assert!(bench.panel_state().is_closed(0));
    }
}
