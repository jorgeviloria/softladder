//! Sequential Function Chart (SFC / Grafcet) model.
//!
//! A [`SequentialPage`] is one page of the chart: a comment, the [`Step`]s
//! drawn on it and the [`Transition`]s that connect them. An SFC
//! [`Section`](crate::model::Section) owns one page through its
//! `sequential_page` field; the scan engine executes those pages in
//! `Project::sections` order.
//!
//! The evolution rule is documented in `docs/SEMANTICS.md`: a transition fires
//! when its condition holds **and** every step it deactivates (`from`) is
//! active; the fired transitions of one section are applied together, clearing
//! first and setting afterwards, so a chain of transitions never fires through
//! in a single scan.
//!
//! Every type here is plain data: it performs no I/O, carries no runtime state
//! and round-trips through `serde`. Step activity and elapsed time live in the
//! scan engine's [`VarStore`](crate::scan::VarStore) (`%X<n>.A` / `%X<n>.V`),
//! so a page stays a pure description of the program.

use serde::{Deserialize, Serialize};

use crate::expr::Expr;

/// A single SFC step.
///
/// The step is addressed by its [`number`](Step::number): the engine publishes
/// the step's activity as `%X<number>.A` and its elapsed time as
/// `%X<number>.V`. ClassicLadder stores the steps in a fixed array and shows
/// the user a number that is distinct from the array slot; the importer
/// translates the slot references of every transition into this number, so the
/// model only ever works with the number the user sees.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Step {
    /// Step number, unique inside the project and the index of `%X<number>`.
    pub number: u32,
    /// `true` when the step is active at start-up.
    pub is_initial: bool,
    /// Editor X coordinate on its page.
    pub x: i32,
    /// Editor Y coordinate on its page.
    pub y: i32,
    /// Page the step is drawn on.
    pub page: u32,
}

impl Step {
    /// Creates an inactive, non-initial step at the origin of `page`.
    pub fn new(number: u32, page: u32) -> Self {
        Self {
            number,
            is_initial: false,
            x: 0,
            y: 0,
            page,
        }
    }
}

/// A transition between steps.
///
/// A transition stores the *outgoing* side of the chart in `from` and the
/// *incoming* side in `to`. ClassicLadder keeps up to ten steps in each
/// direction, which is how AND divergences (one transition activating several
/// steps) and AND convergences (one transition requiring several active steps)
/// are written. A `from` set with more than one step makes the transition fire
/// only when **all** of them are active; a `to` set with more than one step
/// activates them simultaneously.
///
/// Several transitions may share the same `from` step, which is how an OR
/// divergence is written: each branch carries its own condition and the branch
/// whose condition holds fires first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Transition {
    /// Transition number, unique inside the project.
    ///
    /// ClassicLadder stores transitions in a fixed array and numbers them by
    /// their slot in that array; the number is not shown to the user. The
    /// importer normalizes the numbers densely (ascending source slot), which
    /// is what the exporter writes back.
    pub number: u32,
    /// Firing condition; `None` means "always true".
    pub condition: Option<Expr>,
    /// Step numbers the transition deactivates when it fires. Every one of them
    /// must be active for the transition to fire (AND convergence).
    pub from: Vec<u32>,
    /// Step numbers the transition activates when it fires (AND divergence).
    pub to: Vec<u32>,
    /// Page the transition is drawn on.
    pub page: u32,
    /// Editor X coordinate on its page.
    pub x: i32,
    /// Editor Y coordinate on its page.
    pub y: i32,
}

impl Transition {
    /// Creates a transition with no condition and no targets.
    pub fn new(number: u32, page: u32) -> Self {
        Self {
            number,
            condition: None,
            from: Vec::new(),
            to: Vec::new(),
            page,
            x: 0,
            y: 0,
        }
    }

    /// `true` when the transition fires whenever its source steps are active.
    pub fn is_unconditional(&self) -> bool {
        self.condition.is_none()
    }
}

/// A page of the sequential chart, as drawn by the editor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SequentialPage {
    /// Page number, unique inside the project.
    pub number: u32,
    /// Free-form page comment (ClassicLadder's `P<n>,<comment>` record).
    pub comment: String,
    /// Steps drawn on this page.
    pub steps: Vec<Step>,
    /// Transitions drawn on this page.
    pub transitions: Vec<Transition>,
}

impl SequentialPage {
    /// Creates an empty page.
    pub fn new(number: u32, comment: impl Into<String>) -> Self {
        Self {
            number,
            comment: comment.into(),
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

    /// `true` when the page holds neither a step nor a transition.
    ///
    /// An empty page can still carry a comment; the scan engine treats a page
    /// with no steps as a page that cannot change anything.
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty() && self.transitions.is_empty()
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
            ..Transition::default()
        });
        assert!(page.step(1).is_some());
        assert!(page.step(9).is_none());
        assert!(!page.is_empty());
        assert_eq!(
            page.transition(1).and_then(|t| t.condition.clone()),
            Some("%I0".parse::<Expr>().expect("condition parses"))
        );
    }

    #[test]
    fn an_unconditional_transition_reports_itself() {
        let transition = Transition::new(0, 0);
        assert!(transition.is_unconditional());
        assert_eq!(transition.from, Vec::<u32>::new());
        assert_eq!(transition.to, Vec::<u32>::new());
    }

    #[test]
    fn a_page_round_trips_through_serde() {
        let mut page = SequentialPage::new(2, "Loop");
        page.steps.push(Step::new(10, 2));
        page.transitions.push(Transition {
            number: 3,
            condition: None,
            from: vec![10],
            to: vec![11, 12],
            page: 2,
            x: 4,
            y: 6,
        });
        let text = serde_json::to_string(&page).expect("a page serializes");
        let back: SequentialPage = serde_json::from_str(&text).expect("a page deserializes");
        assert_eq!(back, page);
    }
}
