//! Lifecycle supervision: state machine, tick loop and scan statistics.

use std::path::Path;
use std::time::Instant;

use softladder_core::{Project, ScanEngine, ScanReport};
use softladder_project::{native, ProjectError};

/// Lifecycle state of a [`Runtime`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum RuntimeState {
    /// A project is being loaded; no scan runs.
    Loading,
    /// The runtime is halted; [`Runtime::tick`] does nothing.
    #[default]
    Stop,
    /// The runtime scans on every tick.
    Run,
    /// Exactly one more scan runs, then the runtime stops.
    RunOneCycle,
    /// The runtime is frozen: no scan runs, but the state is preserved.
    Freeze,
}

impl RuntimeState {
    /// `true` when the state allows a scan to run.
    pub fn is_scanning(self) -> bool {
        matches!(self, RuntimeState::Run | RuntimeState::RunOneCycle)
    }
}

/// Performance counters of a running runtime.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ScanStats {
    /// Number of scans performed since the runtime was created.
    pub cycles: u64,
    /// Duration of the most recent scan, in milliseconds.
    pub last_ms: f64,
    /// Longest scan seen so far, in milliseconds.
    pub max_ms: f64,
    /// Number of ticks that arrived later than the configured scan period.
    pub missed: u64,
}

/// Supervisor around a [`ScanEngine`].
#[derive(Debug, Clone)]
pub struct Runtime {
    /// Project being executed.
    pub project: Project,
    /// Deterministic scan engine, including the variable store.
    pub engine: ScanEngine,
    /// Current lifecycle state.
    pub state: RuntimeState,
    /// Number of scans performed since the runtime was created.
    pub cycles: u64,
    /// Simulated timestamp of the first scan after the last start, if any.
    pub started_at_ms: Option<u64>,
    stats: ScanStats,
    last_tick_ms: Option<u64>,
}

impl Runtime {
    /// Creates a stopped runtime for `project`.
    pub fn new(project: Project) -> Self {
        Self {
            engine: ScanEngine::new(project.clone()),
            project,
            state: RuntimeState::Stop,
            cycles: 0,
            started_at_ms: None,
            stats: ScanStats::default(),
            last_tick_ms: None,
        }
    }

    /// Loads a project from disk and wraps it in a stopped runtime.
    pub fn from_file(path: &Path) -> Result<Self, ProjectError> {
        Ok(Self::new(native::load(path)?))
    }

    /// Puts the runtime in [`RuntimeState::Run`].
    ///
    /// The simulated start timestamp is recorded by the next tick.
    pub fn start(&mut self) {
        self.state = RuntimeState::Run;
        self.started_at_ms = None;
        self.last_tick_ms = None;
        self.engine = ScanEngine::new(self.project.clone());
        self.cycles = 0;
    }

    /// Records the start timestamp and puts the runtime in [`RuntimeState::Run`].
    pub fn start_at(&mut self, now_ms: u64) {
        self.start();
        self.started_at_ms = Some(now_ms);
        self.last_tick_ms = Some(now_ms);
    }

    /// Puts the runtime in [`RuntimeState::Stop`].
    pub fn stop(&mut self) {
        self.state = RuntimeState::Stop;
    }

    /// Requests exactly one more scan, then a return to [`RuntimeState::Stop`].
    pub fn run_one_cycle(&mut self) {
        self.state = RuntimeState::RunOneCycle;
    }

    /// Puts the runtime in [`RuntimeState::Freeze`].
    pub fn freeze(&mut self) {
        self.state = RuntimeState::Freeze;
    }

    /// Performance counters.
    pub fn stats(&self) -> &ScanStats {
        &self.stats
    }

    /// Runs at most one scan at simulated time `now_ms`.
    ///
    /// Returns [`None`] when the current state does not scan, so a caller can
    /// drive `tick` unconditionally from a UI or a timer loop.
    pub fn tick(&mut self, now_ms: u64) -> Option<ScanReport> {
        if !self.state.is_scanning() {
            return None;
        }

        if let Some(previous) = self.last_tick_ms {
            if self.state == RuntimeState::Run
                && now_ms.saturating_sub(previous) > u64::from(self.project.scan.period_ms)
            {
                self.stats.missed = self.stats.missed.saturating_add(1);
            }
        }
        if self.started_at_ms.is_none() {
            self.started_at_ms = Some(now_ms);
        }
        self.last_tick_ms = Some(now_ms);

        let started = Instant::now();
        let report = self.engine.scan_once(now_ms);
        let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;

        self.stats.cycles = self.stats.cycles.saturating_add(1);
        self.stats.last_ms = elapsed_ms;
        self.stats.max_ms = self.stats.max_ms.max(elapsed_ms);
        self.cycles = self.cycles.saturating_add(1);

        if self.state == RuntimeState::RunOneCycle {
            self.state = RuntimeState::Stop;
        }
        Some(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use softladder_core::{ElementKind, PlacedElement, Rung, Section, Value, VarRef};

    fn var(text: &str) -> VarRef {
        text.parse::<VarRef>().expect("test variable parses")
    }

    fn project_with_lamp() -> Project {
        let mut project = Project::new("runtime test");
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

    #[test]
    fn a_stopped_runtime_does_not_scan() {
        let mut runtime = Runtime::new(project_with_lamp());
        assert_eq!(runtime.state, RuntimeState::Stop);
        assert!(runtime.tick(0).is_none());
        assert_eq!(runtime.cycles, 0);
        assert!(runtime.started_at_ms.is_none());
    }

    #[test]
    fn start_and_stop_drive_the_scan_loop() {
        let mut runtime = Runtime::new(project_with_lamp());
        runtime.start();
        assert_eq!(runtime.state, RuntimeState::Run);
        assert!(runtime.tick(0).is_some());
        assert_eq!(runtime.started_at_ms, Some(0));
        assert_eq!(runtime.cycles, 1);
        assert!(runtime.tick(10).is_some());
        assert_eq!(runtime.cycles, 2);
        runtime.stop();
        assert!(runtime.tick(20).is_none());
        assert_eq!(runtime.cycles, 2);
        assert_eq!(runtime.stats().cycles, 2);
    }

    #[test]
    fn a_single_cycle_scans_once_and_stops() {
        let mut runtime = Runtime::new(project_with_lamp());
        runtime.run_one_cycle();
        assert_eq!(runtime.state, RuntimeState::RunOneCycle);
        assert!(runtime.tick(0).is_some());
        assert_eq!(runtime.state, RuntimeState::Stop);
        assert!(runtime.tick(10).is_none());
        assert_eq!(runtime.cycles, 1);
    }

    #[test]
    fn freezing_suspends_scanning_without_losing_state() {
        let mut runtime = Runtime::new(project_with_lamp());
        runtime.start();
        runtime.tick(0);
        runtime.freeze();
        assert_eq!(runtime.state, RuntimeState::Freeze);
        assert!(runtime.tick(1000).is_none());
        runtime.start();
        assert!(runtime.tick(2000).is_some());
    }

    #[test]
    fn restarting_resets_the_engine_and_the_counters() {
        let mut runtime = Runtime::new(project_with_lamp());
        runtime.start();
        runtime
            .engine
            .store_mut()
            .set(&var("%I0"), Value::Bit(true))
            .expect("input can be set");
        runtime.tick(0);
        assert_eq!(
            runtime.engine.store().get(&var("%Q0")),
            Some(Value::Bit(true))
        );
        runtime.start();
        assert_eq!(runtime.cycles, 0);
        assert_eq!(
            runtime.engine.store().get(&var("%Q0")),
            Some(Value::Bit(false))
        );
    }

    #[test]
    fn late_ticks_are_counted_as_missed() {
        let mut runtime = Runtime::new(project_with_lamp());
        runtime.start_at(0);
        runtime.tick(10);
        runtime.tick(20);
        assert_eq!(runtime.stats().missed, 0);
        runtime.tick(500);
        assert_eq!(runtime.stats().missed, 1);
        runtime.run_one_cycle();
        runtime.tick(5000);
        // A single-cycle request is a manual step, not a missed deadline.
        assert_eq!(runtime.stats().missed, 1);
        assert!(runtime.stats().max_ms >= 0.0);
    }

    #[test]
    fn loading_a_missing_project_fails() {
        let error =
            Runtime::from_file(Path::new("/definitely/not/here.slprj")).expect_err("must fail");
        assert!(matches!(error, ProjectError::Io(_)));
    }

    #[test]
    fn states_report_whether_they_scan() {
        assert!(RuntimeState::Run.is_scanning());
        assert!(RuntimeState::RunOneCycle.is_scanning());
        assert!(!RuntimeState::Stop.is_scanning());
        assert!(!RuntimeState::Freeze.is_scanning());
        assert!(!RuntimeState::Loading.is_scanning());
        assert_eq!(RuntimeState::default(), RuntimeState::Stop);
    }
}
