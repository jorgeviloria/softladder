//! Pure queries over the editor's project, problems and live engine state.
//!
//! Everything the UI needs to *decide* something — which rung a diagnostic
//! points at, which variables the watch tab lists, which cells are energised,
//! what the status bar says — lives here as a function of plain values. None of
//! it needs a window or an event loop, so all of it is unit-tested.

use std::path::Path;

use softladder_core::model::WireMode;
use softladder_core::{
    eval, parse, Accessor, Diagnostic, ElementKind, PlacedElement, Project, Rung, Severity, Symbol,
    Value, VarRef, VarStore,
};
use softladder_edit::RuntimeState;

/// A rung together with the section that shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RungRef {
    /// Index of the owning section in `Project::sections`.
    pub section: usize,
    /// Position of the rung inside `Section::rungs`.
    pub position: usize,
    /// Id of the rung.
    pub rung: u32,
}

/// Where a diagnostic points inside the project.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProblemTarget {
    /// Id of the offending rung.
    pub rung: u32,
    /// Cell the diagnostic names, when its message carries one.
    pub cell: Option<(u8, u8)>,
}

/// The rungs a section executes, skipping ids the project does not define.
///
/// A missing id is a data error, not a reason to panic: the editor must open
/// and show such a project so the user can repair it.
pub fn section_rungs(project: &Project, section_index: usize) -> Vec<u32> {
    project
        .sections
        .get(section_index)
        .map(|section| {
            section
                .rungs
                .iter()
                .copied()
                .filter(|id| project.rung(*id).is_some())
                .collect()
        })
        .unwrap_or_default()
}

/// Finds the section that displays `rung`, in section order.
///
/// A section may name a rung the project does not define; such an id is not
/// found, which is what keeps the rest of the UI from selecting a rung that
/// cannot be drawn.
pub fn find_rung(project: &Project, rung: u32) -> Option<RungRef> {
    for (section, entry) in project.sections.iter().enumerate() {
        if let Some(position) = entry.rungs.iter().position(|id| *id == rung) {
            if project.rung(rung).is_some() {
                return Some(RungRef {
                    section,
                    position,
                    rung,
                });
            }
            return None;
        }
    }
    None
}

/// Resolves a diagnostic to the rung (and cell) it belongs to.
///
/// `Diagnostic::rung` is a position inside the diagnostic's section, so both
/// indices are validated against the project: an out-of-range or missing rung
/// simply yields `None`.
pub fn problem_target(project: &Project, diagnostic: &Diagnostic) -> Option<ProblemTarget> {
    let cell = cell_from_message(&diagnostic.message);
    let rung = match (diagnostic.section, diagnostic.rung) {
        (Some(section), Some(position)) => project
            .sections
            .get(section)
            .and_then(|entry| entry.rungs.get(position))
            .copied()
            .filter(|id| project.rung(*id).is_some())?,
        (Some(_), None) => return None,
        (None, Some(position)) => project
            .sections
            .iter()
            .filter_map(|entry| entry.rungs.get(position).copied())
            .find(|id| project.rung(*id).is_some())?,
        (None, None) => return None,
    };
    Some(ProblemTarget { rung, cell })
}

/// Extracts the cell a diagnostic message names, if it names one.
///
/// `lint` reports duplicate cells as
/// `two elements are placed on cell (col 3, row 1)`, which is the only
/// diagnostic that carries a position; everything else keeps its rung.
pub fn cell_from_message(message: &str) -> Option<(u8, u8)> {
    let rest = message.split("cell (col ").nth(1)?;
    let mut parts = rest.split(", row ");
    let col = parts.next()?.trim().parse::<u8>().ok()?;
    let row = parts.next()?.split(')').next()?.trim().parse::<u8>().ok()?;
    Some((col, row))
}

/// `true` when a diagnostic points at `rung`.
pub fn rung_has_problem(project: &Project, problems: &[Diagnostic], rung: u32) -> bool {
    problems
        .iter()
        .any(|diagnostic| problem_target(project, diagnostic).map(|t| t.rung) == Some(rung))
}

/// Number of diagnostics that point at `rung`.
pub fn rung_problem_count(project: &Project, problems: &[Diagnostic], rung: u32) -> usize {
    problems
        .iter()
        .filter(|diagnostic| problem_target(project, diagnostic).map(|t| t.rung) == Some(rung))
        .count()
}

/// Number of errors and warnings in `problems`.
pub fn problem_counts(problems: &[Diagnostic]) -> (usize, usize) {
    let errors = problems
        .iter()
        .filter(|diagnostic| diagnostic.severity == Severity::Error)
        .count();
    (errors, problems.len().saturating_sub(errors))
}

/// Short text for a runtime lifecycle state, used by the status bar.
pub fn run_state_text(state: RuntimeState) -> &'static str {
    match state {
        RuntimeState::Loading => "LOAD",
        RuntimeState::Stop => "STOP",
        RuntimeState::Run => "RUN",
        RuntimeState::RunOneCycle => "SCAN",
        RuntimeState::Freeze => "FREEZE",
    }
}

/// Title of the native window.
pub fn window_title(name: &str, dirty: bool, path: Option<&Path>) -> String {
    let name = if name.trim().is_empty() {
        path.and_then(Path::file_name)
            .and_then(|file| file.to_str())
            .unwrap_or("untitled")
    } else {
        name
    };
    format!("{name}{} — SoftLadder", if dirty { " *" } else { "" })
}

/// The whole status bar, as one line.
///
/// Kept pure so the exact wording (and the dirty marker) is pinned by a test.
pub fn status_text(
    state: RuntimeState,
    cycles: u64,
    last_scan_ms: f64,
    path: Option<&Path>,
    dirty: bool,
    errors: usize,
    warnings: usize,
) -> String {
    let scan = if last_scan_ms.is_finite() {
        last_scan_ms.max(0.0)
    } else {
        0.0
    };
    let place = match path {
        Some(path) => format!("{}{}", path.display(), if dirty { " *" } else { "" }),
        None => format!("untitled{}", if dirty { " *" } else { "" }),
    };
    let problems = match (errors, warnings) {
        (0, 0) => "no problems".to_owned(),
        (0, warnings) => format!("{warnings} warning{}", plural(warnings)),
        (errors, 0) => format!("{errors} error{}", plural(errors)),
        (errors, warnings) => format!(
            "{errors} error{}, {warnings} warning{}",
            plural(errors),
            plural(warnings)
        ),
    };
    format!(
        "{}   cycles {}   scan {scan:.2} ms   {place}   {problems}",
        run_state_text(state),
        cycles
    )
}

fn plural(count: usize) -> &'static str {
    if count == 1 {
        ""
    } else {
        "s"
    }
}

/// Renders a variable value for the watch tab and the bench.
pub fn value_text(value: Option<Value>) -> String {
    match value {
        None => "—".to_owned(),
        Some(Value::Bit(true)) => "true".to_owned(),
        Some(Value::Bit(false)) => "false".to_owned(),
        Some(Value::Word(word)) => word.to_string(),
        Some(Value::DWord(word)) => word.to_string(),
        Some(Value::Real(real)) => {
            if real.is_finite() {
                format!("{real}")
            } else {
                "—".to_owned()
            }
        }
    }
}

/// A value as a plain integer, clamped into the `i32` range.
pub fn value_i32(value: &Value) -> i32 {
    match *value {
        Value::Bit(bit) => i32::from(bit),
        Value::Word(word) => word,
        Value::DWord(word) => word.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
        Value::Real(real) => {
            if real.is_finite() {
                real as i32
            } else {
                0
            }
        }
    }
}

/// `true` when the element's own bit currently reads as set.
///
/// This is the best-effort live indication the canvas uses: an element with no
/// variable (a `CMP`, an `OPE` or a bare `wire`) is never reported as
/// energised, because nothing in the store can tell.
pub fn element_energised(element: &PlacedElement, store: &VarStore) -> bool {
    element
        .var
        .as_ref()
        .and_then(|var| store.get(var))
        .map(Value::as_bool)
        .unwrap_or(false)
}

/// `true` when an element lets power through from its left side.
///
/// Contacts conduct according to their variable (a normally-closed one conducts
/// while its variable is low), a connection always conducts, and every other
/// element — coils and the function blocks — conducts into its own output, which
/// is what makes an energised coil draw bright. Edge contacts are approximated
/// by their level, because the canvas does not keep the engine's edge history.
pub fn element_conducts(element: &PlacedElement, store: &VarStore) -> bool {
    let bit = element
        .var
        .as_ref()
        .and_then(|var| store.get(var))
        .map(Value::as_bool)
        .unwrap_or(false);
    match element.kind {
        ElementKind::ContactNo | ElementKind::ContactRising | ElementKind::ContactFalling => bit,
        ElementKind::ContactNc => element.var.is_some() && !bit,
        _ => true,
    }
}

/// Power flow for every cell of `rung`, as `(col, row, energised)`.
///
/// The approximation the canvas draws with: a row is fed by the left rail, and
/// each cell passes power on only when it is fed *and* conducts, so a gap or an
/// open contact visibly breaks the wire. Cells are reported in `(row, col)`
/// order and a row with nothing in column 0 stays dead, matching `SL-W001`.
pub fn power_grid(rung: &Rung, store: &VarStore) -> Vec<(u8, u8, bool)> {
    let mut rows: Vec<u8> = rung.elements.iter().map(|element| element.row).collect();
    rows.sort_unstable();
    rows.dedup();

    let mut power = Vec::new();
    for row in rows {
        let mut live = false;
        let max_col = rung
            .elements
            .iter()
            .filter(|element| element.row == row)
            .map(|element| element.col)
            .max()
            .unwrap_or(0);
        for col in 0..=max_col {
            let element = rung
                .elements
                .iter()
                .find(|element| element.col == col && element.row == row);
            // Column 0 is fed by the rail; every other column is fed by the
            // cell to its left.
            let fed = col == 0 || live;
            live = match element {
                Some(element) => fed && element_conducts(element, store),
                None => false,
            };
            power.push((col, row, live));
        }
    }
    power
}

/// Looks up the power state of a cell in a [`power_grid`] result.
pub fn cell_power(power: &[(u8, u8, bool)], col: u8, row: u8) -> bool {
    power
        .iter()
        .find(|(cell_col, cell_row, _)| *cell_col == col && *cell_row == row)
        .map(|(_, _, live)| *live)
        .unwrap_or(false)
}

/// Power state of one cell of a rung.
///
/// `fed` is the power arriving at the cell's left side (the rail, or the OR over
/// the cell's vertical block) and `live` is the power leaving it to the right,
/// which is what the canvas colours the wire and the glyph with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CellPower {
    /// Column of the cell.
    pub col: u8,
    /// Row of the cell.
    pub row: u8,
    /// Power arriving at the cell's left side.
    pub fed: bool,
    /// Power leaving the cell to the right.
    pub live: bool,
}

/// The rows a rung uses, counting every row a function block occupies.
///
/// [`Rung::row_count`] only looks at the rows elements are *placed* on; a
/// counter placed on row 0 reads four rows, so the canvas needs the taller
/// number to draw the block and its input rows.
pub fn rung_rows(rung: &Rung) -> u8 {
    let rows = rung
        .elements
        .iter()
        .map(|element| usize::from(element.row) + usize::from(block_span(element.kind)))
        .max()
        .unwrap_or(1)
        .max(1);
    u8::try_from(rows).unwrap_or(u8::MAX)
}

/// The columns a rung uses (the largest column plus one, at least one).
pub fn rung_cols(rung: &Rung) -> u8 {
    rung.elements
        .iter()
        .map(|element| usize::from(element.col) + 1)
        .max()
        .unwrap_or(1)
        .max(1)
        .try_into()
        .unwrap_or(u8::MAX)
}

/// Number of grid rows a function block reads, one for a single-cell element.
///
/// Mirrors the scan engine: a counter reads `CU/CD/R/LD` from four rows, a
/// register `R/IN/OUT` from three, and a timer from its own row.
pub fn block_span(kind: ElementKind) -> u8 {
    match kind {
        ElementKind::Counter { .. } => 4,
        ElementKind::Register { .. } => 3,
        _ => 1,
    }
}

/// The element whose cell covers `(col, row)`, counting the rows a block reads.
///
/// A counter is placed once and occupies four rows; clicking any of them has to
/// find that one element, which is what makes a block draggable as a unit.
pub fn element_at_cell(rung: &Rung, col: u8, row: u8) -> Option<&PlacedElement> {
    let (col, row) = (usize::from(col), usize::from(row));
    rung.elements.iter().find(|element| {
        usize::from(element.col) == col
            && row >= usize::from(element.row)
            && row < usize::from(element.row) + usize::from(block_span(element.kind))
    })
}

/// Power flow for every cell of `rung`, as the scan engine computes it.
///
/// This is [`power_grid`] extended with the two things the live diagram needs:
/// the vertical links (`connected_with_top`, `docs/SEMANTICS.md` §2) so parallel
/// branches and their merge points read the way they run, and the power arriving
/// at a cell (`fed`) so a wire stays lit when the element that follows it is
/// open. Cells are reported in column-major order, one entry per cell of
/// `rung_rows` × `rung_cols`.
pub fn power_map(rung: &Rung, store: &VarStore) -> Vec<CellPower> {
    let rows = usize::from(rung_rows(rung));
    let cols = usize::from(rung_cols(rung));
    let mut power = Vec::with_capacity(rows.saturating_mul(cols));
    if rung.elements.is_empty() || rows == 0 || cols == 0 {
        return power;
    }
    let explicit = rung.wire_mode == WireMode::Explicit;
    let mut previous = vec![false; rows];
    let mut current = vec![false; rows];
    let mut fed = vec![false; rows];
    for col in 0..cols {
        std::mem::swap(&mut previous, &mut current);
        for (row, slot) in fed.iter_mut().enumerate() {
            *slot = state_on_left(rung, col, row, &previous, rows);
        }
        // What an empty cell carries: nothing in an inert row, nothing across an
        // explicit gap, and the power from the left in an implicit live row.
        for (row, slot) in current.iter_mut().enumerate() {
            let exists = element_at_cell(rung, cell_col(col), cell_row(row)).is_some();
            *slot = if !covers(rung, row) || (explicit && !exists) {
                false
            } else {
                fed.get(row).copied().unwrap_or(false)
            };
        }
        // Then the cells that hold an element overwrite their own output.
        for element in &rung.elements {
            if usize::from(element.col) != col {
                continue;
            }
            let row = usize::from(element.row);
            let span = usize::from(block_span(element.kind));
            if span > 1 {
                // A block writes its own output on its first row; the rest of
                // its rows are inputs, so they conduct what reaches them.
                if let Some(slot) = current.get_mut(row) {
                    *slot = element_energised(element, store);
                }
                for offset in 1..span {
                    if let Some(slot) = row.checked_add(offset).and_then(|at| current.get_mut(at)) {
                        *slot = fed.get(row + offset).copied().unwrap_or(false);
                    }
                }
            } else if let Some(slot) = current.get_mut(row) {
                *slot = fed.get(row).copied().unwrap_or(false) && conducts(element, store);
            }
        }
        for row in 0..rows {
            power.push(CellPower {
                col: cell_col(col),
                row: cell_row(row),
                fed: fed.get(row).copied().unwrap_or(false),
                live: current.get(row).copied().unwrap_or(false),
            });
        }
    }
    power
}

/// A column index as the `u8` the model stores, saturating.
fn cell_col(col: usize) -> u8 {
    u8::try_from(col).unwrap_or(u8::MAX)
}

/// A row index as the `u8` the model stores, saturating.
fn cell_row(row: usize) -> u8 {
    u8::try_from(row).unwrap_or(u8::MAX)
}

/// The power state of a cell in a [`power_map`] result, or a dead cell.
pub fn cell_state(power: &[CellPower], col: u8, row: u8) -> CellPower {
    power
        .iter()
        .find(|cell| cell.col == col && cell.row == row)
        .copied()
        .unwrap_or(CellPower {
            col,
            row,
            fed: false,
            live: false,
        })
}

/// `true` when `row` holds (part of) an element, i.e. when the row is live.
fn covers(rung: &Rung, row: usize) -> bool {
    rung.elements.iter().any(|element| {
        let start = usize::from(element.row);
        row >= start && row < start + usize::from(block_span(element.kind))
    })
}

/// `true` when the cell at `(col, row)` declares a link with the one above it.
fn linked_up(rung: &Rung, col: usize, row: usize) -> bool {
    row > 0
        && rung.elements.iter().any(|element| {
            usize::from(element.col) == col
                && usize::from(element.row) == row
                && element.connected_with_top
        })
}

/// `state_on_left` of the scan engine: the rail for column zero, and the OR of
/// the previous column over the cell's vertical block everywhere else.
fn state_on_left(rung: &Rung, col: usize, row: usize, previous: &[bool], rows: usize) -> bool {
    if col == 0 {
        // An empty cell in column zero touches nothing.
        return element_at_cell(rung, 0, cell_row(row)).is_some();
    }
    let mut result = previous.get(row).copied().unwrap_or(false);
    let mut y = row;
    while y > 0 && linked_up(rung, col, y) {
        y -= 1;
        result |= previous.get(y).copied().unwrap_or(false);
    }
    let mut y = row.saturating_add(1);
    while y < rows && linked_up(rung, col, y) {
        result |= previous.get(y).copied().unwrap_or(false);
        y += 1;
    }
    result
}

/// `true` when the element passes power from its left side to its right side.
fn conducts(element: &PlacedElement, store: &VarStore) -> bool {
    if element.kind == ElementKind::Compare {
        return compare_holds(element, store);
    }
    element_conducts(element, store)
}

/// Evaluates a compare block's expression the way the engine does.
fn compare_holds(element: &PlacedElement, store: &VarStore) -> bool {
    let expression = match element.params.len() {
        1 => element.params.first().cloned().unwrap_or_default(),
        3 => element
            .params
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(" "),
        _ => return false,
    };
    parse(&expression)
        .and_then(|expr| eval(&expr, store))
        .map(Value::as_bool)
        .unwrap_or(false)
}

/// Variables the rung uses, in element order and without repeats.
pub fn watch_vars(project: &Project, rung: Option<u32>) -> Vec<VarRef> {
    let Some(rung) = rung.and_then(|id| project.rung(id)) else {
        return Vec::new();
    };
    let mut vars: Vec<VarRef> = Vec::new();
    for element in &rung.elements {
        if let Some(var) = element.var.as_ref() {
            if !vars.contains(var) {
                vars.push(var.clone());
            }
        }
    }
    vars
}

/// Every variable any section of the project uses, sorted by kind and index.
pub fn used_vars(project: &Project) -> Vec<VarRef> {
    let mut vars: Vec<VarRef> = Vec::new();
    for section in &project.sections {
        for id in &section.rungs {
            let Some(rung) = project.rung(*id) else {
                continue;
            };
            for element in &rung.elements {
                if let Some(var) = element.var.as_ref() {
                    if !vars.contains(var) {
                        vars.push(var.clone());
                    }
                }
            }
        }
    }
    vars.sort_by_key(|var| (var.kind.mnemonic(), var.index));
    vars
}

/// The symbol bound to `var`, if the project declares one.
pub fn symbol_for<'a>(project: &'a Project, var: &VarRef) -> Option<&'a Symbol> {
    project
        .symbols
        .iter()
        .find(|symbol| symbol.var.as_ref() == Some(var))
}

/// The elapsed/current value of a timer, counter or register.
///
/// `accessor` picks the sub-value (`.V` for elapsed, `.S` for the number of
/// values a register holds); the element's own variable supplies the kind and
/// index, and `None` is returned when the element has no variable.
pub fn block_value(element: &PlacedElement, store: &VarStore, accessor: Accessor) -> Option<Value> {
    let var = element.var.clone()?.with_accessor(accessor);
    store.get(&var)
}

#[cfg(test)]
mod tests {
    use super::*;
    use softladder_core::{ElementKind, Rung, Section, Symbol, Value, VarRef, VarStore};

    fn var(text: &str) -> VarRef {
        text.parse().expect("test variable parses")
    }

    fn project() -> Project {
        let mut project = Project::new("queries");
        let mut main = Section::new(1, "Main");
        main.rungs.push(10);
        main.rungs.push(20);
        let mut sub = Section::new(2, "Sub");
        sub.rungs.push(30);
        sub.rungs.push(999); // missing on purpose
        project.sections.push(main);
        project.sections.push(sub);
        project.rungs.push(Rung {
            id: 10,
            label: "lamp".to_owned(),
            elements: vec![
                PlacedElement::with_var(ElementKind::ContactNo, var("%I0"), 0, 0),
                PlacedElement::with_var(ElementKind::CoilOut, var("%Q0"), 1, 0),
                PlacedElement::with_var(ElementKind::ContactNo, var("%I0"), 2, 0),
            ],
            ..Rung::new(10)
        });
        project.rungs.push(Rung {
            id: 20,
            elements: vec![PlacedElement::with_var(
                ElementKind::ContactNo,
                var("%I1"),
                0,
                0,
            )],
            ..Rung::new(20)
        });
        project.rungs.push(Rung {
            id: 30,
            elements: vec![PlacedElement::with_params(
                ElementKind::Compare,
                0,
                0,
                &["%MW0", ">", "3"],
            )],
            ..Rung::new(30)
        });
        project.symbols.push(Symbol {
            name: "start".to_owned(),
            var: Some(var("%I0")),
            comment: "start button".to_owned(),
            unit: None,
        });
        project
    }

    fn diagnostic(section: Option<usize>, rung: Option<usize>, message: &str) -> Diagnostic {
        Diagnostic {
            severity: Severity::Error,
            code: "SL-E009",
            section,
            rung,
            message: message.to_owned(),
        }
    }

    #[test]
    fn section_rungs_skips_ids_the_project_does_not_define() {
        let project = project();
        assert_eq!(section_rungs(&project, 0), vec![10, 20]);
        assert_eq!(section_rungs(&project, 1), vec![30], "999 is skipped");
        assert_eq!(section_rungs(&project, 9), Vec::<u32>::new());
    }

    #[test]
    fn find_rung_reports_the_section_and_position() {
        let project = project();
        assert_eq!(
            find_rung(&project, 20),
            Some(RungRef {
                section: 0,
                position: 1,
                rung: 20
            })
        );
        assert_eq!(
            find_rung(&project, 30),
            Some(RungRef {
                section: 1,
                position: 0,
                rung: 30
            })
        );
        assert_eq!(find_rung(&project, 999), None, "referenced but undefined");
    }

    #[test]
    fn problem_targets_resolve_section_and_rung_positions_to_ids() {
        let project = project();
        let target = problem_target(&project, &diagnostic(Some(0), Some(1), "anything"));
        assert_eq!(
            target,
            Some(ProblemTarget {
                rung: 20,
                cell: None
            })
        );
        let target = problem_target(&project, &diagnostic(Some(1), Some(0), "anything"));
        assert_eq!(
            target,
            Some(ProblemTarget {
                rung: 30,
                cell: None
            })
        );
    }

    #[test]
    fn problem_targets_tolerate_missing_and_out_of_range_ids() {
        let project = project();
        // Position 1 of section 1 references the missing rung 999.
        assert_eq!(
            problem_target(&project, &diagnostic(Some(1), Some(1), "missing")),
            None
        );
        assert_eq!(
            problem_target(&project, &diagnostic(Some(7), Some(0), "no section")),
            None
        );
        assert_eq!(
            problem_target(&project, &diagnostic(Some(0), Some(9), "past the end")),
            None
        );
        assert_eq!(
            problem_target(&project, &diagnostic(None, None, "no context")),
            None
        );
        // With only a position, the first section that has one wins.
        assert_eq!(
            problem_target(&project, &diagnostic(None, Some(0), "position only")),
            Some(ProblemTarget {
                rung: 10,
                cell: None
            })
        );
        assert_eq!(
            problem_target(&project, &diagnostic(None, Some(9), "position only")),
            None
        );
    }

    #[test]
    fn a_duplicate_cell_diagnostic_carries_the_cell() {
        let message = "two elements are placed on cell (col 3, row 1)";
        assert_eq!(cell_from_message(message), Some((3, 1)));
        let project = project();
        let target = problem_target(&project, &diagnostic(Some(0), Some(0), message));
        assert_eq!(
            target,
            Some(ProblemTarget {
                rung: 10,
                cell: Some((3, 1))
            })
        );
        assert_eq!(cell_from_message("nothing here"), None);
        assert_eq!(cell_from_message("cell (col x, row 1)"), None);
        assert_eq!(cell_from_message("cell (col 1, row )"), None);
    }

    #[test]
    fn problem_counts_split_errors_from_warnings() {
        let problems = vec![
            Diagnostic::new(Severity::Error, "SL-E001", "a".to_owned()),
            Diagnostic::new(Severity::Warning, "SL-W001", "b".to_owned()),
            Diagnostic::new(Severity::Error, "SL-E002", "c".to_owned()),
            Diagnostic::new(Severity::Info, "SL-I001", "d".to_owned()),
        ];
        assert_eq!(problem_counts(&problems), (2, 2));
        assert_eq!(problem_counts(&[]), (0, 0));

        let project = project();
        let scoped = vec![
            diagnostic(Some(0), Some(0), "first").with_section(0),
            diagnostic(Some(0), Some(1), "second"),
        ];
        assert!(rung_has_problem(&project, &scoped, 10));
        assert!(rung_has_problem(&project, &scoped, 20));
        assert!(!rung_has_problem(&project, &scoped, 30));
        assert_eq!(rung_problem_count(&project, &scoped, 10), 1);
        assert_eq!(rung_problem_count(&project, &scoped, 30), 0);
    }

    #[test]
    fn run_state_text_names_every_lifecycle_state() {
        assert_eq!(run_state_text(RuntimeState::Loading), "LOAD");
        assert_eq!(run_state_text(RuntimeState::Stop), "STOP");
        assert_eq!(run_state_text(RuntimeState::Run), "RUN");
        assert_eq!(run_state_text(RuntimeState::RunOneCycle), "SCAN");
        assert_eq!(run_state_text(RuntimeState::Freeze), "FREEZE");
    }

    #[test]
    fn window_title_uses_the_project_name_and_a_dirty_marker() {
        use std::path::Path;
        assert_eq!(window_title("traffic", false, None), "traffic — SoftLadder");
        assert_eq!(
            window_title("traffic", true, None),
            "traffic * — SoftLadder"
        );
        assert_eq!(
            window_title("", false, Some(Path::new("/tmp/plant.slprj"))),
            "plant.slprj — SoftLadder"
        );
        assert_eq!(window_title("   ", false, None), "untitled — SoftLadder");
    }

    #[test]
    fn status_text_reports_state_cycles_scan_path_and_problems() {
        use std::path::Path;
        let path = Path::new("/tmp/plant.slprj");
        assert_eq!(
            status_text(RuntimeState::Run, 412, 0.031, Some(path), false, 0, 0),
            "RUN   cycles 412   scan 0.03 ms   /tmp/plant.slprj   no problems"
        );
        assert_eq!(
            status_text(RuntimeState::Stop, 0, 0.0, Some(path), true, 2, 1),
            "STOP   cycles 0   scan 0.00 ms   /tmp/plant.slprj *   2 errors, 1 warning"
        );
        assert_eq!(
            status_text(RuntimeState::RunOneCycle, 7, f64::NAN, None, false, 0, 1),
            "SCAN   cycles 7   scan 0.00 ms   untitled   1 warning"
        );
        assert_eq!(
            status_text(RuntimeState::Freeze, 1, 1.5, None, true, 1, 0),
            "FREEZE   cycles 1   scan 1.50 ms   untitled *   1 error"
        );
        assert_eq!(
            status_text(RuntimeState::Loading, 0, -4.0, None, false, 3, 2),
            "LOAD   cycles 0   scan 0.00 ms   untitled   3 errors, 2 warnings"
        );
    }

    #[test]
    fn values_render_readably() {
        assert_eq!(value_text(None), "—");
        assert_eq!(value_text(Some(Value::Bit(true))), "true");
        assert_eq!(value_text(Some(Value::Bit(false))), "false");
        assert_eq!(value_text(Some(Value::Word(-7))), "-7");
        assert_eq!(value_text(Some(Value::DWord(1_000_000))), "1000000");
        assert_eq!(value_text(Some(Value::Real(1.5))), "1.5");
        assert_eq!(value_text(Some(Value::Real(f64::NAN))), "—");
    }

    #[test]
    fn values_convert_to_integers_without_overflowing() {
        assert_eq!(value_i32(&Value::Bit(true)), 1);
        assert_eq!(value_i32(&Value::Bit(false)), 0);
        assert_eq!(value_i32(&Value::Word(-7)), -7);
        assert_eq!(value_i32(&Value::DWord(i64::MAX)), i32::MAX);
        assert_eq!(value_i32(&Value::DWord(i64::MIN)), i32::MIN);
        assert_eq!(value_i32(&Value::Real(2.9)), 2);
        assert_eq!(value_i32(&Value::Real(f64::INFINITY)), 0);
    }

    #[test]
    fn watch_vars_follow_the_rung_and_drop_repeats() {
        let project = project();
        assert_eq!(
            watch_vars(&project, Some(10)),
            vec![var("%I0"), var("%Q0")],
            "%I0 appears once"
        );
        assert_eq!(watch_vars(&project, Some(20)), vec![var("%I1")]);
        assert_eq!(
            watch_vars(&project, Some(30)),
            Vec::<VarRef>::new(),
            "no var"
        );
        assert_eq!(watch_vars(&project, None), Vec::<VarRef>::new());
        assert_eq!(watch_vars(&project, Some(999)), Vec::<VarRef>::new());
    }

    #[test]
    fn used_vars_covers_the_whole_project_in_a_stable_order() {
        let project = project();
        let vars = used_vars(&project);
        assert_eq!(vars, vec![var("%I0"), var("%I1"), var("%Q0")]);
        assert_eq!(used_vars(&Project::new("empty")), Vec::<VarRef>::new());
    }

    #[test]
    fn symbols_are_found_by_their_variable() {
        let project = project();
        let symbol = symbol_for(&project, &var("%I0")).expect("bound");
        assert_eq!(symbol.name, "start");
        assert_eq!(symbol_for(&project, &var("%Q0")), None);
    }

    #[test]
    fn power_flows_from_the_rail_and_stops_at_an_open_contact() {
        let mut store = VarStore::with_default_sizes();
        let rung = Rung {
            elements: vec![
                PlacedElement::with_var(ElementKind::ContactNo, var("%I0"), 0, 0),
                PlacedElement::with_var(ElementKind::ContactNo, var("%I1"), 1, 0),
                PlacedElement::with_var(ElementKind::CoilOut, var("%Q0"), 2, 0),
            ],
            ..Rung::new(1)
        };

        store.set(&var("%I0"), Value::Bit(true)).expect("writable");
        store.set(&var("%I1"), Value::Bit(true)).expect("writable");
        let power = power_grid(&rung, &store);
        assert!(cell_power(&power, 0, 0));
        assert!(cell_power(&power, 1, 0));
        assert!(cell_power(&power, 2, 0));

        store.set(&var("%I1"), Value::Bit(false)).expect("writable");
        let power = power_grid(&rung, &store);
        assert!(cell_power(&power, 0, 0));
        assert!(!cell_power(&power, 1, 0), "the open contact breaks the row");
        assert!(!cell_power(&power, 2, 0), "nothing downstream is fed");
        assert!(!cell_power(&power, 9, 9), "unknown cells are dead");
    }

    #[test]
    fn a_normally_closed_contact_conducts_while_its_variable_is_low() {
        let mut store = VarStore::with_default_sizes();
        let rung = Rung {
            elements: vec![
                PlacedElement::with_var(ElementKind::ContactNc, var("%I0"), 0, 0),
                PlacedElement::with_var(ElementKind::CoilOut, var("%Q0"), 1, 0),
            ],
            ..Rung::new(1)
        };
        let power = power_grid(&rung, &store);
        assert!(cell_power(&power, 0, 0));
        assert!(
            cell_power(&power, 1, 0),
            "an open input closes the NC contact"
        );

        store.set(&var("%I0"), Value::Bit(true)).expect("writable");
        let power = power_grid(&rung, &store);
        assert!(!cell_power(&power, 1, 0), "a closed input opens it again");
    }

    #[test]
    fn a_row_with_no_cell_in_column_zero_is_dead() {
        let store = VarStore::with_default_sizes();
        let rung = Rung {
            elements: vec![PlacedElement::with_var(
                ElementKind::ContactNo,
                var("%I0"),
                2,
                0,
            )],
            ..Rung::new(1)
        };
        let power = power_grid(&rung, &store);
        assert!(!cell_power(&power, 0, 0), "column 0 is empty");
        assert!(!cell_power(&power, 1, 0));
        assert!(!cell_power(&power, 2, 0));
    }

    #[test]
    fn power_is_tracked_per_row() {
        let mut store = VarStore::with_default_sizes();
        store.set(&var("%I0"), Value::Bit(true)).expect("writable");
        let rung = Rung {
            elements: vec![
                PlacedElement::with_var(ElementKind::ContactNo, var("%I0"), 0, 0).linked_up(),
                PlacedElement::with_var(ElementKind::ContactNo, var("%I1"), 0, 1),
            ],
            ..Rung::new(1)
        };
        let power = power_grid(&rung, &store);
        assert!(cell_power(&power, 0, 0));
        assert!(!cell_power(&power, 0, 1), "row 1 has its own flow");
    }

    #[test]
    fn elements_without_a_variable_are_never_energised() {
        let store = VarStore::with_default_sizes();
        let compare = PlacedElement::with_params(ElementKind::Compare, 0, 0, &["%MW0", "=", "0"]);
        assert!(!element_energised(&compare, &store));
        let contact = PlacedElement::with_var(ElementKind::ContactNo, var("%I5"), 0, 0);
        assert!(
            !element_energised(&contact, &store),
            "unset inputs read low"
        );
    }

    #[test]
    fn block_values_read_a_sub_value_of_the_elements_variable() {
        let mut store = VarStore::with_default_sizes();
        let timer = PlacedElement::with_var(
            ElementKind::Timer {
                mode: Default::default(),
            },
            var("%TM0"),
            0,
            0,
        );
        store
            .set(&var("%TM0.V"), Value::Word(250))
            .expect("writable");
        assert_eq!(
            block_value(&timer, &store, Accessor::Value),
            Some(Value::Word(250))
        );
        let connection = PlacedElement::new(ElementKind::Connection, 0, 0);
        assert_eq!(block_value(&connection, &store, Accessor::Value), None);
    }

    #[test]
    fn queries_never_panic_on_an_empty_project() {
        let project = Project::new("empty");
        assert_eq!(section_rungs(&project, 0), Vec::<u32>::new());
        assert_eq!(find_rung(&project, 0), None);
        assert_eq!(watch_vars(&project, Some(0)), Vec::<VarRef>::new());
        assert_eq!(used_vars(&project), Vec::<VarRef>::new());
        assert!(!rung_has_problem(&project, &[], 0));
        assert_eq!(problem_counts(&[]), (0, 0));
        assert_eq!(
            problem_target(&project, &diagnostic(None, Some(0), "x")),
            None
        );
    }
}
