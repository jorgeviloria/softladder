//! Simulation panel: the on-screen switches, lamps and analog widgets that let
//! a program be commissioned before any hardware exists.
//!
//! Two types, deliberately separated:
//!
//! * [`SimulationPanel`] is the **bench layout**. It is part of the project, so
//!   a bench travels with the program it belongs to.
//! * [`PanelState`] is the **operator's positions**. It is runtime state and is
//!   never serialized: saving a project must not record that somebody left a
//!   switch closed.
//!
//! Nothing here renders, so the whole bench is unit-testable without a window.
//! Every widget addresses a physical variable (`%I…`, `%Q…`, `%IW…`, `%QW…`):
//! the panel stands in for the machine, it is not a general variable browser.

use serde::{Deserialize, Serialize};

use crate::expr::Value;
use crate::model::Project;
use crate::scan::VarStore;
use crate::vars::{VarKind, VarRef};

/// A two-state input widget: a toggle switch or a momentary push-button.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SimSwitch {
    /// Physical input the switch drives.
    pub var: VarRef,
    /// Text shown on the widget.
    pub label: String,
    /// `true` for a push-button (it springs back after one scan), `false` for a
    /// toggle that stays where the operator left it.
    #[serde(default)]
    pub momentary: bool,
}

/// A lamp driven by a physical output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SimLamp {
    /// Physical output the lamp follows.
    pub var: VarRef,
    /// Text shown on the widget.
    pub label: String,
}

/// A slider driving an analog (`%IW`) input.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SimAnalog {
    /// Analog input the slider drives.
    pub var: VarRef,
    /// Text shown on the widget.
    pub label: String,
    /// Lowest value the slider can send.
    pub min: i32,
    /// Highest value the slider can send.
    pub max: i32,
}

/// A gauge following an analog (`%QW`) output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SimGauge {
    /// Analog output the gauge follows.
    pub var: VarRef,
    /// Text shown on the widget.
    pub label: String,
    /// Lowest value on the dial.
    pub min: i32,
    /// Highest value on the dial.
    pub max: i32,
}

/// The bench laid out next to the ladder; part of the project.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct SimulationPanel {
    /// Toggle switches and push-buttons, in display order.
    #[serde(default)]
    pub switches: Vec<SimSwitch>,
    /// Lamps, in display order.
    #[serde(default)]
    pub lamps: Vec<SimLamp>,
    /// Analog sliders, in display order.
    #[serde(default)]
    pub analogs: Vec<SimAnalog>,
    /// Analog gauges, in display order.
    #[serde(default)]
    pub gauges: Vec<SimGauge>,
}

impl SimulationPanel {
    /// `true` when the panel has no widgets at all.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Total number of widgets.
    pub fn len(&self) -> usize {
        self.switches.len() + self.lamps.len() + self.analogs.len() + self.gauges.len()
    }

    /// Builds a panel that mirrors every physical variable the project uses.
    ///
    /// Inputs become switches (or sliders, for `%IW`), outputs become lamps (or
    /// gauges, for `%QW`), and a bound symbol provides the label. This is what
    /// the editor offers for a project that has no panel yet, so that one click
    /// is enough to start commissioning.
    pub fn auto_fill(project: &Project) -> Self {
        let mut panel = Self::default();
        let mut seen: Vec<VarRef> = Vec::new();
        let mut vars: Vec<VarRef> = project
            .sections
            .iter()
            .flat_map(|section| section.rungs.iter())
            .filter_map(|rung_id| project.rung(*rung_id))
            .flat_map(|rung| rung.elements.iter())
            .filter_map(|element| element.var.clone())
            .collect();
        vars.sort_by_key(|var| (var.kind.mnemonic(), var.index));

        for var in vars {
            if seen.contains(&var) {
                continue;
            }
            seen.push(var.clone());
            let label = symbol_for(project, &var).unwrap_or_else(|| var.to_string());
            match (var.kind, var.accessor) {
                (VarKind::PhysIn, _) => panel.switches.push(SimSwitch {
                    var,
                    label,
                    momentary: false,
                }),
                (VarKind::PhysOut, _) => panel.lamps.push(SimLamp { var, label }),
                (VarKind::PhysInWord, _) => panel.analogs.push(SimAnalog {
                    var,
                    label,
                    min: 0,
                    max: 100,
                }),
                (VarKind::PhysOutWord, _) => panel.gauges.push(SimGauge {
                    var,
                    label,
                    min: 0,
                    max: 100,
                }),
                _ => {}
            }
        }
        panel
    }

    /// Ensures the panel only addresses physical variables of the right width.
    ///
    /// Returns one message per offending widget, in panel order; the editor
    /// shows them in the Problems panel and the CLI can report them too.
    pub fn validate(&self) -> Vec<String> {
        let mut problems = Vec::new();
        for switch in &self.switches {
            if switch.var.kind != VarKind::PhysIn || switch.var.accessor.is_some() {
                problems.push(format!(
                    "switch `{}` should address a plain %I input, found `{}`",
                    switch.label, switch.var
                ));
            }
        }
        for lamp in &self.lamps {
            if lamp.var.kind != VarKind::PhysOut || lamp.var.accessor.is_some() {
                problems.push(format!(
                    "lamp `{}` should address a plain %Q output, found `{}`",
                    lamp.label, lamp.var
                ));
            }
        }
        for analog in &self.analogs {
            if analog.var.kind != VarKind::PhysInWord || analog.var.accessor.is_some() {
                problems.push(format!(
                    "slider `{}` should address a plain %IW input, found `{}`",
                    analog.label, analog.var
                ));
            }
            if analog.min > analog.max {
                problems.push(format!(
                    "slider `{}` has an inverted range ({}..{})",
                    analog.label, analog.min, analog.max
                ));
            }
        }
        for gauge in &self.gauges {
            if gauge.var.kind != VarKind::PhysOutWord || gauge.var.accessor.is_some() {
                problems.push(format!(
                    "gauge `{}` should address a plain %QW output, found `{}`",
                    gauge.label, gauge.var
                ));
            }
            if gauge.min > gauge.max {
                problems.push(format!(
                    "gauge `{}` has an inverted range ({}..{})",
                    gauge.label, gauge.min, gauge.max
                ));
            }
        }
        problems
    }
}

/// One output value read back from the bench.
#[derive(Debug, Clone, PartialEq)]
pub struct SimReading {
    /// Variable that was read.
    pub var: VarRef,
    /// Widget label.
    pub label: String,
    /// Current value.
    pub value: Value,
}

/// The operator's positions on the bench; runtime state, never saved.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PanelState {
    /// One entry per switch; `true` means closed (or pressed).
    closed: Vec<bool>,
    /// One entry per analog slider.
    analogs: Vec<i32>,
}

impl PanelState {
    /// Creates the state a bench starts from: everything open, sliders at
    /// their minimum.
    pub fn new(panel: &SimulationPanel) -> Self {
        Self {
            closed: vec![false; panel.switches.len()],
            analogs: panel.analogs.iter().map(|analog| analog.min).collect(),
        }
    }

    /// Grows or shrinks the state so that it matches `panel` again, keeping the
    /// positions of the widgets that are still there.
    pub fn sync(&mut self, panel: &SimulationPanel) {
        self.closed.resize(panel.switches.len(), false);
        let defaults: Vec<i32> = panel.analogs.iter().map(|analog| analog.min).collect();
        for (index, default) in defaults.iter().enumerate() {
            if index >= self.analogs.len() {
                self.analogs.push(*default);
            }
        }
        self.analogs.truncate(panel.analogs.len());
    }

    /// `true` when the switch at `index` is closed.
    pub fn is_closed(&self, index: usize) -> bool {
        self.closed.get(index).copied().unwrap_or(false)
    }

    /// Sets or clears the switch at `index`.
    pub fn set_closed(&mut self, index: usize, closed: bool) {
        if let Some(slot) = self.closed.get_mut(index) {
            *slot = closed;
        }
    }

    /// Flips the switch at `index`.
    pub fn toggle(&mut self, index: usize) {
        let closed = self.is_closed(index);
        self.set_closed(index, !closed);
    }

    /// Current slider position, clamped into the widget's range.
    pub fn analog(&self, panel: &SimulationPanel, index: usize) -> i32 {
        let value = self.analogs.get(index).copied().unwrap_or(0);
        match panel.analogs.get(index) {
            Some(analog) => value.clamp(analog.min.min(analog.max), analog.max.max(analog.min)),
            None => value,
        }
    }

    /// Moves the slider at `index`, clamping into the widget's range.
    pub fn set_analog(&mut self, panel: &SimulationPanel, index: usize, value: i32) {
        if index >= self.analogs.len() {
            return;
        }
        let clamped = match panel.analogs.get(index) {
            Some(analog) => value.clamp(analog.min.min(analog.max), analog.max.max(analog.min)),
            None => value,
        };
        if let Some(slot) = self.analogs.get_mut(index) {
            *slot = clamped;
        }
    }

    /// Clears every momentary button, as a real spring does.
    pub fn release_momentary(&mut self, panel: &SimulationPanel) {
        for (index, switch) in panel.switches.iter().enumerate() {
            if switch.momentary {
                self.set_closed(index, false);
            }
        }
    }

    /// Writes the operator's positions into the store's input image.
    ///
    /// Call it right before a scan; call [`PanelState::release_momentary`]
    /// right after, so that a push-button spans exactly one scan.
    pub fn apply_inputs(&self, panel: &SimulationPanel, store: &mut VarStore) {
        for (index, switch) in panel.switches.iter().enumerate() {
            let _ = store.set(&switch.var, Value::Bit(self.is_closed(index)));
        }
        for (index, analog) in panel.analogs.iter().enumerate() {
            let value = self.analog(panel, index);
            let _ = store.set(&analog.var, Value::Word(value));
        }
    }

    /// Reads the panel's outputs (lamps and gauges) out of the store.
    pub fn read_outputs(&self, panel: &SimulationPanel, store: &VarStore) -> Vec<SimReading> {
        let mut readings = Vec::with_capacity(panel.lamps.len() + panel.gauges.len());
        for lamp in &panel.lamps {
            readings.push(SimReading {
                var: lamp.var.clone(),
                label: lamp.label.clone(),
                value: store.get(&lamp.var).unwrap_or(Value::Bit(false)),
            });
        }
        for gauge in &panel.gauges {
            readings.push(SimReading {
                var: gauge.var.clone(),
                label: gauge.label.clone(),
                value: store.get(&gauge.var).unwrap_or(Value::Word(0)),
            });
        }
        readings
    }
}

/// Finds the symbol bound to `var`, if the project has one.
fn symbol_for(project: &Project, var: &VarRef) -> Option<String> {
    project
        .symbols
        .iter()
        .find(|symbol| symbol.var.as_ref() == Some(var))
        .map(|symbol| symbol.name.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ElementKind, PlacedElement, Rung, Section, Symbol};

    fn var(text: &str) -> VarRef {
        text.parse().expect("variable parses")
    }

    fn bench() -> SimulationPanel {
        SimulationPanel {
            switches: vec![
                SimSwitch {
                    var: var("%I0"),
                    label: "start".to_owned(),
                    momentary: false,
                },
                SimSwitch {
                    var: var("%I1"),
                    label: "push".to_owned(),
                    momentary: true,
                },
            ],
            lamps: vec![SimLamp {
                var: var("%Q0"),
                label: "green".to_owned(),
            }],
            analogs: vec![SimAnalog {
                var: var("%IW0"),
                label: "dial".to_owned(),
                min: 0,
                max: 1000,
            }],
            gauges: vec![SimGauge {
                var: var("%QW0"),
                label: "meter".to_owned(),
                min: 0,
                max: 100,
            }],
        }
    }

    #[test]
    fn an_empty_project_gets_an_empty_panel() {
        let panel = SimulationPanel::auto_fill(&Project::new("empty"));
        assert!(panel.is_empty());
        assert_eq!(panel.len(), 0);
        assert!(panel.validate().is_empty());
        assert_eq!(PanelState::new(&panel), PanelState::default());
    }

    #[test]
    fn auto_fill_mirrors_the_physical_variables_of_the_project() {
        let mut project = Project::new("mirror");
        let mut rung = Rung::new(1);
        for (kind, text, col) in [
            (ElementKind::ContactNo, "%I0", 0),
            (ElementKind::CoilOut, "%Q0", 1),
            (ElementKind::ContactNo, "%I0", 2),
            (ElementKind::ContactNo, "%M3", 3),
            (ElementKind::Compare, "%IW1", 4),
            (ElementKind::Operate, "%QW2", 5),
        ] {
            rung.elements
                .push(PlacedElement::with_var(kind, var(text), col, 0));
        }
        project.rungs.push(rung);
        let mut section = Section::new(1, "Main");
        section.rungs.push(1);
        project.sections.push(section);
        project.symbols.push(Symbol {
            name: "start_button".to_owned(),
            var: Some(var("%I0")),
            comment: String::new(),
            unit: None,
        });

        let panel = SimulationPanel::auto_fill(&project);
        assert_eq!(panel.switches.len(), 1, "one switch for %I0, not two");
        assert_eq!(panel.switches[0].label, "start_button");
        assert_eq!(panel.lamps.len(), 1);
        assert_eq!(panel.lamps[0].label, "%Q0");
        assert_eq!(panel.analogs.len(), 1);
        assert_eq!(panel.gauges.len(), 1);
        assert!(panel.validate().is_empty(), "{:?}", panel.validate());
    }

    #[test]
    fn the_operator_drives_the_inputs_and_reads_the_outputs() {
        let panel = bench();
        let mut state = PanelState::new(&panel);
        let mut store = VarStore::with_default_sizes();

        // Everything starts open and at its minimum.
        state.apply_inputs(&panel, &mut store);
        assert_eq!(store.get(&var("%I0")), Some(Value::Bit(false)));
        assert_eq!(store.get(&var("%IW0")), Some(Value::Word(0)));

        state.toggle(0);
        state.set_analog(&panel, 0, 5000);
        state.apply_inputs(&panel, &mut store);
        assert_eq!(store.get(&var("%I0")), Some(Value::Bit(true)));
        assert_eq!(
            store.get(&var("%IW0")),
            Some(Value::Word(1000)),
            "the slider clamps into its range"
        );

        store.set(&var("%Q0"), Value::Bit(true)).expect("writable");
        store.set(&var("%QW0"), Value::Word(42)).expect("writable");
        let readings = state.read_outputs(&panel, &store);
        assert_eq!(readings.len(), 2);
        assert_eq!(readings[0].label, "green");
        assert_eq!(readings[0].value, Value::Bit(true));
        assert_eq!(readings[1].value, Value::Word(42));

        // A momentary button springs back, a toggle does not.
        state.set_closed(1, true);
        state.release_momentary(&panel);
        assert!(state.is_closed(0), "the toggle stays where it was left");
        assert!(!state.is_closed(1), "the push-button springs back");
    }

    #[test]
    fn sync_keeps_positions_of_widgets_that_survive() {
        let panel = bench();
        let mut state = PanelState::new(&panel);
        state.toggle(0);
        state.set_analog(&panel, 0, 700);

        let mut smaller = panel.clone();
        smaller.switches.pop();
        state.sync(&smaller);
        assert!(state.is_closed(0));
        assert_eq!(state.analog(&smaller, 0), 700);

        state.sync(&panel);
        assert!(state.is_closed(0));
        assert!(!state.is_closed(1), "a new switch starts open");
        assert_eq!(state.analog(&panel, 0), 700);
    }

    #[test]
    fn validation_flags_wrong_variables_and_inverted_ranges() {
        let panel = SimulationPanel {
            switches: vec![SimSwitch {
                var: var("%Q0"),
                label: "wrong".to_owned(),
                momentary: false,
            }],
            lamps: vec![SimLamp {
                var: var("%MW0.1"),
                label: "wrong lamp".to_owned(),
            }],
            analogs: vec![SimAnalog {
                var: var("%IW0"),
                label: "inverted".to_owned(),
                min: 90,
                max: 10,
            }],
            gauges: vec![SimGauge {
                var: var("%QW0"),
                label: "fine".to_owned(),
                min: 0,
                max: 10,
            }],
        };
        let problems = panel.validate();
        assert_eq!(problems.len(), 3, "{problems:?}");
        assert!(problems.iter().any(|text| text.contains("wrong")));
        assert!(problems.iter().any(|text| text.contains("inverted")));
    }

    #[test]
    fn the_panel_round_trips_through_the_project_file() {
        let panel = bench();
        let mut project = Project::new("bench");
        project.simulation = panel.clone();
        let text = serde_json::to_string(&project).expect("serializes");
        let back: Project = serde_json::from_str(&text).expect("deserializes");
        assert_eq!(back.simulation, panel);
        assert!(text.contains("\"simulation\""));
        // Positions are runtime state and never reach the file.
        assert!(!text.contains("closed"));
    }

    #[test]
    fn a_project_without_a_panel_still_loads() {
        let text = r#"{"schema_version":2,"name":"old","author":"","comment":"",
            "sections":[],"rungs":[],"symbols":[],
            "scan":{"period_ms":10,"input_period_ms":10}}"#;
        let project: Project = serde_json::from_str(text).expect("deserializes");
        assert!(project.simulation.is_empty());
    }
}
