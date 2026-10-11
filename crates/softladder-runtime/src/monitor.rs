//! [`MonitorTarget`](softladder_monitor::MonitorTarget) for the runtime — **M6 slice two**.
//!
//! [`MonitoredRuntime`] wraps a [`Runtime`] with a force map: monitor writes
//! go straight to the store (and lose to the program on the next scan, exactly
//! like the bench's modify column), while forces are re-applied before every
//! [`MonitoredRuntime::tick`] so they hold while the program runs. Alarms still
//! read empty: the journal is a later M6 slice.

use std::collections::HashMap;

use softladder_core::{Project, ScanReport, Value, VarRef};
use softladder_monitor::{Alarm, MonitorTarget, TargetError};

use crate::runtime::{Runtime, RuntimeState};

/// A [`Runtime`] driven through the monitor protocol, with a force map.
#[derive(Debug, Clone)]
pub struct MonitoredRuntime {
    runtime: Runtime,
    forces: HashMap<VarRef, Value>,
}

impl MonitoredRuntime {
    /// Creates a stopped runtime for `project` with no forces installed.
    pub fn new(project: Project) -> Self {
        Self {
            runtime: Runtime::new(project),
            forces: HashMap::new(),
        }
    }

    /// Wrapped runtime.
    pub fn runtime(&self) -> &Runtime {
        &self.runtime
    }

    /// Mutable access to the wrapped runtime.
    pub fn runtime_mut(&mut self) -> &mut Runtime {
        &mut self.runtime
    }

    /// Installed forces, by variable.
    pub fn forces(&self) -> &HashMap<VarRef, Value> {
        &self.forces
    }

    /// Puts the runtime in [`RuntimeState::Run`].
    pub fn start(&mut self) {
        self.runtime.start();
    }

    /// Puts the runtime in [`RuntimeState::Stop`].
    pub fn stop(&mut self) {
        self.runtime.stop();
    }

    /// Requests exactly one more scan, then a return to [`RuntimeState::Stop`].
    pub fn run_one_cycle(&mut self) {
        self.runtime.run_one_cycle();
    }

    /// Puts the runtime in [`RuntimeState::Freeze`].
    pub fn freeze(&mut self) {
        self.runtime.freeze();
    }

    /// Runs at most one scan at simulated time `now_ms`.
    ///
    /// Forces are written to the store first, so a forced output holds while
    /// the program runs. Returns [`None`] when the state does not scan.
    pub fn tick(&mut self, now_ms: u64) -> Option<ScanReport> {
        for (var, value) in &self.forces {
            let _ = self.runtime.engine.store_mut().set(var, *value);
        }
        self.runtime.tick(now_ms)
    }

    fn deny_forced(&self, var: &VarRef) -> Option<TargetError> {
        if self.forces.contains_key(var) {
            Some(TargetError::new(
                "forced".to_owned(),
                format!("{var} is forced; release it first"),
            ))
        } else {
            None
        }
    }
}

impl MonitorTarget for MonitoredRuntime {
    fn status(&self) -> (String, u64, Option<u64>) {
        let state = match self.runtime.state {
            RuntimeState::Loading => "loading",
            RuntimeState::Stop => "stop",
            RuntimeState::Run => "run",
            RuntimeState::RunOneCycle => "run_one_cycle",
            RuntimeState::Freeze => "freeze",
        }
        .to_owned();
        (state, self.runtime.cycles, self.runtime.started_at_ms)
    }

    fn read(&self, var: &VarRef) -> Option<Value> {
        if let Some(forced) = self.forces.get(var) {
            return Some(*forced);
        }
        self.runtime.engine.store().get(var)
    }

    fn write(&mut self, var: &VarRef, value: &Value) -> Result<Option<Value>, TargetError> {
        if let Some(denied) = self.deny_forced(var) {
            return Err(denied);
        }
        let previous = self.runtime.engine.store().get(var);
        self.runtime
            .engine
            .store_mut()
            .set(var, *value)
            .map_err(|error| TargetError::new("not_writable".to_owned(), error.to_string()))?;
        Ok(previous)
    }

    fn force(&mut self, var: &VarRef, value: &Value) -> Result<Option<Value>, TargetError> {
        let previous = self.read(var);
        self.runtime
            .engine
            .store_mut()
            .set(var, *value)
            .map_err(|error| TargetError::new("not_writable".to_owned(), error.to_string()))?;
        self.forces.insert(var.clone(), *value);
        Ok(previous)
    }

    fn release(&mut self, var: &VarRef) -> bool {
        self.forces.remove(var).is_some()
    }

    fn alarms(&self) -> Vec<Alarm> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use softladder_core::{ElementKind, PlacedElement, Rung, Section};
    use softladder_monitor::{Request, Response, Role, Session, Token};

    fn var(text: &str) -> VarRef {
        text.parse::<VarRef>().expect("test variable parses")
    }

    fn lamp_project() -> Project {
        let mut project = Project::new("monitor target test");
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

    fn engineer() -> Token {
        Token::new("e-1".to_owned(), Role::Engineer)
    }

    #[test]
    fn status_reports_state_cycles_and_start() {
        let mut target = MonitoredRuntime::new(lamp_project());
        assert_eq!(MonitorTarget::status(&target), ("stop".to_owned(), 0, None));
        target.start();
        target.tick(100);
        target.tick(110);
        assert_eq!(
            MonitorTarget::status(&target),
            ("run".to_owned(), 2, Some(100))
        );
    }

    #[test]
    fn a_force_holds_while_the_program_runs() {
        let mut target = MonitoredRuntime::new(lamp_project());
        target.start();
        // With %I0 open the lamp stays off on its own.
        target.tick(0);
        assert_eq!(
            MonitorTarget::read(&target, &var("%Q0")),
            Some(Value::Bit(false))
        );

        target
            .force(&var("%Q0"), &Value::Bit(true))
            .expect("force installs");
        assert_eq!(target.forces().len(), 1);
        // The program would clear %Q0 every scan; the force wins.
        for now_ms in [10, 20, 30] {
            target.tick(now_ms);
            assert_eq!(
                MonitorTarget::read(&target, &var("%Q0")),
                Some(Value::Bit(true))
            );
        }
        assert!(target.release(&var("%Q0")));
        assert!(!target.release(&var("%Q0")));
        target.tick(40);
        assert_eq!(
            MonitorTarget::read(&target, &var("%Q0")),
            Some(Value::Bit(false))
        );
    }

    #[test]
    fn writes_lose_to_the_program_on_the_next_scan() {
        let mut target = MonitoredRuntime::new(lamp_project());
        target.start();
        let previous = target
            .write(&var("%Q0"), &Value::Bit(true))
            .expect("write lands");
        assert_eq!(previous, Some(Value::Bit(false)));
        assert_eq!(
            MonitorTarget::read(&target, &var("%Q0")),
            Some(Value::Bit(true))
        );
        target.tick(0);
        assert_eq!(
            MonitorTarget::read(&target, &var("%Q0")),
            Some(Value::Bit(false))
        );
    }

    #[test]
    fn writes_to_a_forced_variable_are_refused() {
        let mut target = MonitoredRuntime::new(lamp_project());
        target
            .force(&var("%Q0"), &Value::Bit(true))
            .expect("force installs");
        match target.write(&var("%Q0"), &Value::Bit(false)) {
            Err(error) => assert_eq!(error.code, "forced"),
            Ok(_) => panic!("a forced write must fail"),
        }
    }

    #[test]
    fn a_session_drives_the_runtime_end_to_end() {
        let mut session = Session::new(engineer(), MonitoredRuntime::new(lamp_project()));
        session.target_mut().start();
        assert_eq!(
            session.handle(Request::GetStatus),
            Response::Status {
                state: "run".to_owned(),
                cycles: 0,
                started_at_ms: None,
            }
        );
        assert_eq!(
            session.handle(Request::ForceVar {
                var: var("%Q0"),
                value: Value::Bit(true),
            }),
            Response::Ok
        );
        session.target_mut().tick(0);
        match session.handle(Request::GetVars {
            vars: vec![var("%Q0")],
        }) {
            Response::Vars { values } => {
                assert_eq!(values, vec![(var("%Q0"), Some(Value::Bit(true)))]);
            }
            other => panic!("expected vars, got {other:?}"),
        }
        assert_eq!(session.audit().len(), 1);
    }
}
