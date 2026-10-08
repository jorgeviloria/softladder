//! Sequential Function Chart (SFC / Grafcet) model.
//!
//! This module is a **model skeleton only**. It defines the data structures
//! that the editor and the project file format need so that SFC sections can be
//! stored, displayed and round-tripped, but nothing here is executed.
//!
//! The SFC engine — step activation, transition firing, divergence/convergence
//! and the interaction with ladder sections — lands in **M4**. Until then the
//! scan engine reports SFC sections with the diagnostic `SL-W002` and skips
//! them, which keeps every scan deterministic and side-effect free.
//!
//! The layout fields (`x`, `y`, `page`) are editor-only and never influence
//! execution.

use serde::{Deserialize, Serialize};

use crate::expr::Expr;

/// A single SFC step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Step {
    /// Step number, unique inside the project.
    pub number: u32,
    /// `true` when the step is active at start-up.
    pub is_initial: bool,
    /// `true` while the step is active (runtime state, filled in by M4).
    pub active: bool,
    /// Editor X coordinate on its page.
    pub x: i32,
    /// Editor Y coordinate on its page.
    pub y: i32,
    /// Editor page the step is drawn on.
    pub page: u32,
}

/// A transition between steps.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Transition {
    /// Transition number, unique inside the project.
    pub number: u32,
    /// Firing condition; `None` means "always true".
    pub condition: Option<Expr>,
    /// Step numbers the transition comes from.
    pub from: Vec<u32>,
    /// Step numbers the transition goes to.
    pub to: Vec<u32>,
    /// Editor page the transition is drawn on.
    pub page: u32,
}

/// A page of the sequential chart, as drawn by the editor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SequentialPage {
    /// Page number.
    pub number: u32,
    /// Page name.
    pub name: String,
    /// Steps drawn on this page.
    pub steps: Vec<Step>,
    /// Transitions drawn on this page.
    pub transitions: Vec<Transition>,
}

impl SequentialPage {
    /// Creates an empty page.
    pub fn new(number: u32, name: impl Into<String>) -> Self {
        Self {
            number,
            name: name.into(),
            steps: Vec::new(),
            transitions: Vec::new(),
        }
    }

    /// Looks up a step by number.
    pub fn step(&self, number: u32) -> Option<&Step> {
        self.steps.iter().find(|step| step.number == number)
    }

    /// Looks up a transition by number.
    pub fn transition(&self, number: u32) -> Option<&Transition> {
        self.transitions
            .iter()
            .find(|transition| transition.number == number)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_are_plain_data() {
        let mut page = SequentialPage::new(0, "Main sequence");
        page.steps.push(Step {
            number: 1,
            is_initial: true,
            ..Step::default()
        });
        page.transitions.push(Transition {
            number: 1,
            condition: Some("%I0".parse::<Expr>().expect("condition parses")),
            from: vec![1],
            to: vec![2],
            page: 0,
        });
        assert!(page.step(1).is_some());
        assert!(page.step(9).is_none());
        assert_eq!(
            page.transition(1).and_then(|t| t.condition.clone()),
            Some("%I0".parse::<Expr>().expect("condition parses"))
        );
    }
}
