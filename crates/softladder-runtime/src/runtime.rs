//! Lifecycle supervision: state machine, clock driving and scan statistics.

use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use softladder_core::{Diagnostic, Project, ScanEngine, ScanReport};
use softladder_project::{native, ProjectError};

/// How a batch of scans obtains the simulated timestamp `now_ms` of each cycle.
///
/// The scan engine never reads the clock (see `docs/SEMANTICS.md` §4): the
/// caller passes `now_ms` to [`Runtime::tick`]. A `Clock` is the only place
/// that decides what that value is, which is what makes a batch reproducible.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Clock {
    /// Deterministic time: cycle `k` (1-based) runs at `(k - 1) * period_ms`.
    ///
    /// Nothing reads the wall clock and nothing sleeps, so the same project and
    /// the same variable state always produce the same `now_ms` sequence.
    Simulated {
        /// Period between two cycles, in milliseconds.
        period_ms: u64,
    },
    /// Wall-clock time, paced by [`std::thread::sleep`].
    ///
    /// [`Clock::next_now_ms`] sleeps until the slot `(k - 1) * period_ms` of
    /// cycle `k` and returns that slot. When a cycle is requested after its slot
    /// has already passed — the previous scan overran — the real elapsed time is
    /// returned instead; [`Runtime`] counts that as a missed tick when the gap
    /// exceeds the period.
    Realtime {
        /// Period between two cycles, in milliseconds.
        period_ms: u64,
        /// Instant the clock was created; the origin of `now_ms`.
        started: Instant,
    },
}

impl Clock {
    /// Creates a deterministic clock with `period_ms` between two cycles.
    pub fn simulated(period_ms: u64) -> Self {
        Clock::Simulated { period_ms }
    }

    /// Creates a wall-clock paced clock with `period_ms` between two cycles.
    pub fn realtime(period_ms: u64) -> Self {
        Clock::Realtime {
            period_ms,
            started: Instant::now(),
        }
    }

    /// Period between two cycles, in milliseconds.
    pub fn period_ms(&self) -> u64 {
        match *self {
            Clock::Simulated { period_ms } | Clock::Realtime { period_ms, .. } => period_ms,
        }
    }

    /// `true` for [`Clock::Simulated`], i.e. when the run is reproducible.
    pub fn is_simulated(&self) -> bool {
        matches!(self, Clock::Simulated { .. })
    }

    /// Returns the `now_ms` value of cycle `cycle` (1-based).
    ///
    /// A simulated clock is a pure function of the cycle number. A realtime
    /// clock may sleep to keep the configured pace, which is why the method
    /// takes `&mut self`.
    pub fn next_now_ms(&mut self, cycle: u64) -> u64 {
        match *self {
            Clock::Simulated { period_ms } => cycle.saturating_sub(1).saturating_mul(period_ms),
            Clock::Realtime { period_ms, started } => {
                let target = cycle.saturating_sub(1).saturating_mul(period_ms);
                let elapsed = started.elapsed().as_millis() as u64;
                if elapsed < target {
                    thread::sleep(Duration::from_millis(target - elapsed));
                    target
                } else {
                    elapsed
                }
            }
        }
    }
}

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
    /// `true` when the runtime is driving a [`Clock::Simulated`] batch.
    ///
    /// A simulated batch never reads the wall clock, so [`ScanStats::last_ms`]
    /// and [`ScanStats::max_ms`] are not meaningful measurements of it and
    /// [`ScanStats::missed`] stays `0`.
    pub simulated: bool,
}

/// Outcome of a batch of scans driven by [`Runtime::run_cycles`].
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ScanSummary {
    /// `true` when the batch used [`Clock::Simulated`].
    pub simulated: bool,
    /// Number of scans performed.
    pub cycles: u64,
    /// Period between two cycles, in milliseconds.
    pub period_ms: u64,
    /// Longest scan duration, in milliseconds.
    ///
    /// Always `0.0` for a simulated batch, which does not measure wall-clock
    /// time; reporting a real measurement would make two identical runs differ.
    pub max_scan_ms: f64,
    /// Mean scan duration, in milliseconds; `0.0` for a simulated batch.
    pub average_scan_ms: f64,
    /// Ticks that arrived later than `period_ms` after the previous one.
    pub missed: u64,
    /// Simulated timestamp of the last scan, if any ran.
    pub last_now_ms: Option<u64>,
    /// Diagnostics collected over the whole batch, in scan order.
    pub diagnostics: Vec<Diagnostic>,
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
    /// Scan period that decides whether a tick is late, in milliseconds.
    ///
    /// Initialized from `project.scan.period_ms`; [`Runtime::run_cycles`] sets it
    /// to the period of the driving [`Clock`].
    pub period_ms: u64,
    stats: ScanStats,
    last_tick_ms: Option<u64>,
}

impl Runtime {
    /// Creates a stopped runtime for `project`.
    pub fn new(project: Project) -> Self {
        let period_ms = u64::from(project.scan.period_ms);
        Self {
            engine: ScanEngine::new(project.clone()),
            project,
            state: RuntimeState::Stop,
            cycles: 0,
            started_at_ms: None,
            period_ms,
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
            if self.state == RuntimeState::Run && now_ms.saturating_sub(previous) > self.period_ms {
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

    /// Runs `cycles` scans driven by `clock` and summarizes the batch.
    ///
    /// The runtime is restarted first, exactly like [`Runtime::start`], and its
    /// [`Runtime::period_ms`] is taken from `clock`. A simulated clock therefore
    /// advances `now_ms` by exactly one period per cycle, which can never be
    /// "later than the period" and so reports [`ScanSummary::missed`] `= 0`.
    ///
    /// Wall-clock durations are measured by [`Runtime::tick`] but are reported as
    /// `0.0` for a simulated batch, so that the summary of two identical runs is
    /// identical. Callers that need the per-cycle [`ScanReport`] should keep
    /// using [`Runtime::tick`] directly.
    pub fn run_cycles(&mut self, clock: &mut Clock, cycles: u64) -> ScanSummary {
        self.start();
        self.period_ms = clock.period_ms();
        self.stats.simulated = clock.is_simulated();

        let missed_before = self.stats.missed;
        let mut summary = ScanSummary {
            simulated: clock.is_simulated(),
            period_ms: clock.period_ms(),
            ..ScanSummary::default()
        };
        let mut total_ms = 0.0_f64;

        for cycle in 1..=cycles {
            let now_ms = clock.next_now_ms(cycle);
            let Some(report) = self.tick(now_ms) else {
                break;
            };
            total_ms += self.stats.last_ms;
            summary.max_scan_ms = summary.max_scan_ms.max(self.stats.last_ms);
            summary.cycles = summary.cycles.saturating_add(1);
            summary.last_now_ms = Some(now_ms);
            summary.diagnostics.extend(report.diagnostics);
        }

        if summary.cycles > 0 {
            summary.average_scan_ms = total_ms / summary.cycles as f64;
        }
        summary.missed = self.stats.missed.saturating_sub(missed_before);
        if summary.simulated {
            summary.max_scan_ms = 0.0;
            summary.average_scan_ms = 0.0;
        }
        summary
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
    fn a_simulated_clock_steps_by_exactly_one_period() {
        let period_ms = 13;
        let mut clock = Clock::simulated(period_ms);
        assert!(clock.is_simulated());
        assert_eq!(clock.period_ms(), period_ms);
        for cycle in 1..=1000u64 {
            assert_eq!(clock.next_now_ms(cycle), (cycle - 1) * period_ms);
        }
    }

    #[test]
    fn a_simulated_batch_never_misses_a_tick() {
        let period_ms = 7;
        let mut runtime = Runtime::new(project_with_lamp());
        let mut clock = Clock::simulated(period_ms);
        let summary = runtime.run_cycles(&mut clock, 1000);

        assert!(summary.simulated);
        assert_eq!(summary.cycles, 1000);
        assert_eq!(summary.period_ms, period_ms);
        assert_eq!(summary.missed, 0);
        assert_eq!(summary.last_now_ms, Some(999 * period_ms));
        // A simulated batch does not report wall-clock measurements: they would
        // differ between two identical runs.
        assert_eq!(summary.max_scan_ms, 0.0);
        assert_eq!(summary.average_scan_ms, 0.0);
        assert_eq!(summary.diagnostics, Vec::new());

        assert!(runtime.stats().simulated);
        assert_eq!(runtime.stats().missed, 0);
        assert_eq!(runtime.stats().cycles, 1000);
    }

    #[test]
    fn a_simulated_batch_ignores_a_longer_project_period() {
        // `--period-ms` overrides `ScanConfig::period_ms`; the exact simulated
        // step must not be mistaken for a late tick even when it is larger.
        let mut runtime = Runtime::new(project_with_lamp());
        let mut clock = Clock::simulated(1000);
        let summary = runtime.run_cycles(&mut clock, 5);
        assert_eq!(summary.missed, 0);
        assert_eq!(summary.cycles, 5);
        assert_eq!(runtime.stats().missed, 0);
    }

    #[test]
    fn run_cycles_starts_a_stopped_runtime() {
        let mut runtime = Runtime::new(project_with_lamp());
        assert_eq!(runtime.state, RuntimeState::Stop);
        let mut clock = Clock::simulated(10);
        let summary = runtime.run_cycles(&mut clock, 3);
        assert_eq!(runtime.state, RuntimeState::Run);
        assert_eq!(summary.cycles, 3);
        assert_eq!(runtime.cycles, 3);
        assert_eq!(runtime.started_at_ms, Some(0));
    }

    #[test]
    fn a_realtime_clock_paces_itself_with_the_wall_clock() {
        let period_ms = 10;
        let cycles = 3;
        let mut clock = Clock::realtime(period_ms);
        let started = Instant::now();
        let mut stamps = Vec::new();
        for cycle in 1..=cycles {
            stamps.push(clock.next_now_ms(cycle));
        }
        let elapsed = started.elapsed();

        assert!(!clock.is_simulated());
        assert_eq!(clock.period_ms(), period_ms);
        assert!(stamps.windows(2).all(|window| window[0] <= window[1]));
        // The clock paces every cycle after the first up to its slot.
        assert!(stamps[(cycles - 1) as usize] >= (cycles - 1) * period_ms);
        assert!(elapsed >= Duration::from_millis((cycles - 1) * period_ms));
    }

    #[test]
    fn a_realtime_batch_reports_real_measurements() {
        let mut runtime = Runtime::new(project_with_lamp());
        let mut clock = Clock::realtime(5);
        let summary = runtime.run_cycles(&mut clock, 3);
        assert!(!summary.simulated);
        assert_eq!(summary.cycles, 3);
        assert_eq!(summary.period_ms, 5);
        assert!(summary.max_scan_ms >= 0.0);
        assert!(summary.average_scan_ms >= 0.0);
        assert!(!runtime.stats().simulated);
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
