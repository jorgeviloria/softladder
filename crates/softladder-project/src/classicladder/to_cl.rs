//! SoftLadder → ClassicLadder: the element-level export.
//!
//! The exporter rebuilds the geometry the reference implementation expects: a
//! dense `12 × max(8, rows)` matrix per rung, `ELE_CONNECTION` cells for the
//! gaps of an implicitly wired rung, `ELE_UNUSABLE` body cells for multi-cell
//! blocks, and reverse-mapped `VarType`/`VarNum` fields. The instance CSVs, the
//! sections, the symbols, `general.txt` and `project_infos.txt` are regenerated
//! from the project; every other part of the template document is passed
//! through byte for byte.
//!
//! Everything the reference cannot express is reported as `SL-W033` instead of
//! being dropped silently.

use std::collections::{BTreeMap, BTreeSet};

use softladder_core::model::WireMode;
use softladder_core::{
    CounterKind, Diagnostic, ElementKind, PlacedElement, Project, Rung, Section, SectionLanguage,
    Severity, Symbol, VarKind, VarRef,
};

use super::document::Document;
use super::expr_map;
use super::mapping::{self, encode_var, RUNG_HEIGHT, RUNG_WIDTH};
use super::ExportReport;
use crate::ProjectError;

/// Reference minimum for `SIZE_NBR_RUNGS` (from `classicladder.h`).
const MIN_RUNGS: usize = 300;
/// Reference minimum for `SIZE_NBR_BITS`.
const MIN_BITS: u32 = 500;
/// Reference minimum for `SIZE_NBR_WORDS`.
const MIN_WORDS: u32 = 200;
/// Reference minimum for the deprecated `SIZE_NBR_TIMERS`.
const MIN_TIMERS: usize = 10;
/// Reference minimum for the deprecated `SIZE_NBR_MONOSTABLES`.
const MIN_MONOSTABLES: usize = 10;
/// Reference minimum for `SIZE_NBR_COUNTERS`.
const MIN_COUNTERS: usize = 50;
/// Reference minimum for `SIZE_NBR_TIMERS_IEC`.
const MIN_TIMERS_IEC: usize = 50;
/// Reference minimum for `SIZE_NBR_REGISTERS`.
const MIN_REGISTERS: usize = 10;
/// Reference minimum for `SIZE_REGISTER_LIST`.
const MIN_REGISTER_LIST: u32 = 500;
/// Reference minimum for `SIZE_NBR_PHYS_INPUTS`.
const MIN_PHYS_INPUTS: u32 = 50;
/// Reference minimum for `SIZE_NBR_PHYS_OUTPUTS`.
const MIN_PHYS_OUTPUTS: u32 = 50;
/// Reference minimum for `SIZE_NBR_ARITHM_EXPR`.
const MIN_ARITHM_EXPR: usize = 200;
/// Reference minimum for `SIZE_NBR_SECTIONS`.
const MIN_SECTIONS: usize = 10;
/// Reference minimum for `SIZE_NBR_SYMBOLS`.
const MIN_SYMBOLS: usize = 300;
/// Reference minimum for `SIZE_NBR_PHYS_WORDS_INPUTS`.
const MIN_PHYS_WORDS_INPUTS: u32 = 25;
/// Reference minimum for `SIZE_NBR_PHYS_WORDS_OUTPUTS`.
const MIN_PHYS_WORDS_OUTPUTS: u32 = 25;

/// Part names that keep their reference position in the exported container.
///
/// The empty string marks where the `rung_<n>.csv` parts belong.
const CANONICAL_PARTS: [&str; 19] = [
    "project_infos.txt",
    "general.txt",
    "timers.csv",
    "monostables.csv",
    "counters.csv",
    "timers_iec.csv",
    "registers.csv",
    "arithmetic_expressions.csv",
    "",
    "sections.csv",
    "sequential.csv",
    "symbols.csv",
    "ioconf.csv",
    "com_params.txt",
    "modbusioconf.csv",
    "config_events.csv",
    "modem_config.txt",
    "remote_alarms.txt",
    "spy_vars.csv",
];

/// One cell of the exported matrix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Cell {
    etype: i64,
    connected_with_top: bool,
    var_type: i64,
    var_num: i64,
    indexed: Option<(i64, i64)>,
}

impl Cell {
    /// The reference spelling of the cell.
    fn render(&self) -> String {
        let mut text = format!(
            "{}-{}-{}/{}",
            self.etype,
            u8::from(self.connected_with_top),
            self.var_type,
            self.var_num
        );
        if let Some((index_type, index_num)) = self.indexed {
            text.push_str(&format!("[{index_type}/{index_num}]"));
        }
        text
    }
}

/// Turns a SoftLadder project into a ClassicLadder document.
///
/// The document of an earlier import should be passed as `extras` so that every
/// part SoftLadder does not model survives untouched; an authored project uses
/// [`Document::empty`].
///
/// # Errors
///
/// Currently infallible; the signature keeps room for future I/O-backed
/// templates.
pub(crate) fn export(project: &Project, extras: &Document) -> Result<ExportReport, ProjectError> {
    Exporter::new(project, extras).run()
}

/// The exporter's working state.
struct Exporter<'a> {
    project: &'a Project,
    extras: &'a Document,
    diagnostics: Vec<Diagnostic>,
    expressions: Vec<String>,
    expression_index: BTreeMap<(u32, usize), i64>,
    timers: BTreeMap<u32, (i64, i64, i64)>,
    counters: BTreeMap<u32, i64>,
    registers: BTreeMap<u32, i64>,
    links: BTreeMap<u32, (i64, i64)>,
}

impl<'a> Exporter<'a> {
    fn new(project: &'a Project, extras: &'a Document) -> Self {
        Self {
            project,
            extras,
            diagnostics: Vec::new(),
            expressions: Vec::new(),
            expression_index: BTreeMap::new(),
            timers: BTreeMap::new(),
            counters: BTreeMap::new(),
            registers: BTreeMap::new(),
            links: BTreeMap::new(),
        }
    }

    /// Runs the export.
    fn run(mut self) -> Result<ExportReport, ProjectError> {
        self.collect_instances();
        self.links = self.compute_links();
        if !self.project.simulation.is_empty() {
            self.warn(format!(
                "simulation panel: the bench has {} widget(s), which ClassicLadder cannot express; \
                 it was not exported",
                self.project.simulation.len()
            ));
        }
        let mut generated: Vec<(String, String)> = vec![
            ("project_infos.txt".to_owned(), self.project_infos()),
            ("general.txt".to_owned(), self.general()),
            ("counters.csv".to_owned(), self.counters_csv()),
            ("timers_iec.csv".to_owned(), self.timers_csv()),
            ("registers.csv".to_owned(), self.registers_csv()),
            (
                "arithmetic_expressions.csv".to_owned(),
                self.expressions_csv(),
            ),
            ("sections.csv".to_owned(), self.sections_csv()),
            ("symbols.csv".to_owned(), self.symbols_csv()),
        ];
        let project = self.project;
        let mut rungs: Vec<(u32, String)> = Vec::new();
        for rung in &project.rungs {
            rungs.push((rung.id, self.rung_csv(rung)));
        }
        rungs.sort_by_key(|(id, _)| *id);
        let document = assemble(&mut generated, rungs, self.extras);
        Ok(ExportReport {
            document,
            diagnostics: self.diagnostics,
        })
    }

    /// Records a warning diagnostic.
    fn warn(&mut self, message: String) {
        self.diagnostics
            .push(Diagnostic::new(Severity::Warning, "SL-W033", message));
    }

    /// Collects the instance tables and the expression table from the elements.
    fn collect_instances(&mut self) {
        let project = self.project;
        for rung in &project.rungs {
            for (position, element) in rung.elements.iter().enumerate() {
                let where_ = format!(
                    "rung_{}.csv cell ({},{})",
                    rung.id, element.col, element.row
                );
                match element.kind {
                    ElementKind::Timer { mode } => {
                        let Some(index) = block_index(element, VarKind::TimerIec) else {
                            self.warn(format!(
                                "{where_}: timer block has no `%TM<n>` instance variable; element \
                                 omitted"
                            ));
                            continue;
                        };
                        let preset = element.params.first().map(String::as_str).unwrap_or("");
                        let (base, units) = match mapping::split_timer_preset(preset) {
                            Some((base, units)) => (base, units),
                            None => {
                                self.warn(format!(
                                    "{where_}: timer preset `{preset}` is not a literal duration; \
                                     exported as 0"
                                ));
                                (mapping::BASE_100MS, 0)
                            }
                        };
                        let mode = mapping::timer_mode_code(mode);
                        if let Some(previous) = self.timers.get(&index).copied() {
                            if previous != (base, units, mode) {
                                self.warn(format!(
                                    "{where_}: timer %TM{index} is used twice with different \
                                     settings; the first one wins"
                                ));
                            }
                        } else {
                            self.timers.insert(index, (base, units, mode));
                        }
                    }
                    ElementKind::Counter { kind } => {
                        let Some(index) = block_index(element, VarKind::Counter) else {
                            self.warn(format!(
                                "{where_}: counter block has no `%C<n>` instance variable; element \
                                 omitted"
                            ));
                            continue;
                        };
                        if kind != CounterKind::UpDown {
                            self.warn(format!(
                                "{where_}: counter kind {kind:?} cannot be expressed; the reference \
                                 always counts on both edges"
                            ));
                        }
                        let preset = element
                            .params
                            .first()
                            .and_then(|text| text.trim().parse::<i64>().ok())
                            .unwrap_or(0);
                        if let Some(previous) = self.counters.get(&index).copied() {
                            if previous != preset {
                                self.warn(format!(
                                    "{where_}: counter %C{index} is used twice with different \
                                     presets; the first one wins"
                                ));
                            }
                        } else {
                            self.counters.insert(index, preset);
                        }
                    }
                    ElementKind::Register { mode } => {
                        let Some(index) = block_index(element, VarKind::Register) else {
                            self.warn(format!(
                                "{where_}: register block has no `%R<n>` instance variable; element \
                                 omitted"
                            ));
                            continue;
                        };
                        let mode = mapping::register_mode_code(mode);
                        if let Some(previous) = self.registers.get(&index).copied() {
                            if previous != mode {
                                self.warn(format!(
                                    "{where_}: register %R{index} is used twice with different \
                                     modes; the first one wins"
                                ));
                            }
                        } else {
                            self.registers.insert(index, mode);
                        }
                    }
                    ElementKind::Compare | ElementKind::Operate => {
                        let text = self.expression_text(element, &where_);
                        if text.is_empty() {
                            self.warn(format!(
                                "{where_}: the block has an empty expression; element omitted"
                            ));
                            continue;
                        }
                        let index = self.expressions.len() as i64;
                        self.expressions.push(text);
                        self.expression_index.insert((rung.id, position), index);
                    }
                    _ => {}
                }
            }
        }
    }

    /// ClassicLadder text for a compare/operate element.
    fn expression_text(&mut self, element: &PlacedElement, where_: &str) -> String {
        if element.kind == ElementKind::Compare {
            let translated = expr_map::expression_to_classic(&element.params.join(" "));
            if let Some(reason) = translated.reason {
                self.warn(format!("{where_}: compare expression: {reason}"));
            }
            return translated.classic;
        }
        if element.params.len() < 2 {
            let translated = expr_map::expression_to_classic(&element.params.join(" "));
            if let Some(reason) = translated.reason {
                self.warn(format!("{where_}: operate expression: {reason}"));
            }
            return translated.classic;
        }
        let target = expr_map::expression_to_classic(&element.params[0]);
        if let Some(reason) = target.reason {
            self.warn(format!("{where_}: operate target: {reason}"));
        }
        let rhs_text = element.params[1..].join(" ");
        let rhs = strip_assignment_marker(&rhs_text);
        let value = expr_map::expression_to_classic(rhs);
        if let Some(reason) = value.reason {
            self.warn(format!("{where_}: operate expression: {reason}"));
        }
        format!("{}:={}", target.classic, value.classic)
    }

    /// Renders one rung file.
    fn rung_csv(&mut self, rung: &Rung) -> String {
        let mut alive: BTreeMap<(u8, u8), Cell> = BTreeMap::new();
        for (position, element) in rung.elements.iter().enumerate() {
            let where_ = format!(
                "rung_{}.csv cell ({},{})",
                rung.id, element.col, element.row
            );
            if element.col >= RUNG_WIDTH {
                self.warn(format!(
                    "{where_}: column {} is outside the reference matrix ({RUNG_WIDTH} wide); \
                     element omitted",
                    element.col
                ));
                continue;
            }
            if let Some(cell) = self.encode_element(rung, position, element, &where_) {
                // Multi-cell blocks are written by the expansion below, which
                // puts the block's own cell on the reference's "alive" column.
                let (width, _) = mapping::block_geometry(&element.kind);
                if width <= 1 {
                    alive.insert((element.col, element.row), cell);
                }
            }
        }
        let mut rows = RUNG_HEIGHT;
        for element in &rung.elements {
            let (_, height) = mapping::block_geometry(&element.kind);
            rows = rows.max(element.row.saturating_add(height));
        }
        if let Some(max_row) = rung.elements.iter().map(|element| element.row).max() {
            if max_row >= RUNG_HEIGHT {
                self.warn(format!(
                    "rung_{}.csv: row {max_row} is below the reference matrix height \
                     ({RUNG_HEIGHT}); the reference will ignore it",
                    rung.id
                ));
            }
        }
        let mut matrix: Vec<Vec<Cell>> =
            vec![vec![Cell::default(); usize::from(RUNG_WIDTH)]; usize::from(rows)];
        for ((column, row), cell) in &alive {
            if let Some(line) = matrix.get_mut(usize::from(*row)) {
                if let Some(slot) = line.get_mut(usize::from(*column)) {
                    *slot = *cell;
                }
            }
        }
        // Multi-cell blocks are expanded to their reference geometry: the block
        // itself goes on the "alive" cell at the top-right corner of the
        // rectangle, the rest of the rectangle becomes `ELE_UNUSABLE` body
        // cells, and any vertical link our model recorded on one of those cells
        // is copied onto the body cell so the reference walks the same links.
        for (position, element) in rung.elements.iter().enumerate() {
            let (width, height) = mapping::block_geometry(&element.kind);
            if width <= 1 && height <= 1 {
                continue;
            }
            let where_ = format!(
                "rung_{}.csv cell ({},{})",
                rung.id, element.col, element.row
            );
            let Some(alive_column) = element.col.checked_add(width - 1) else {
                continue;
            };
            if alive_column >= RUNG_WIDTH {
                self.warn(format!(
                    "{where_}: the block needs {width} columns but its alive cell would land on \
                     column {alive_column}, outside the reference matrix ({RUNG_WIDTH} wide); \
                     only its body was written"
                ));
                continue;
            }
            let Some(block) = self.encode_element(rung, position, element, &where_) else {
                continue;
            };
            let body = Cell {
                etype: mapping::ELE_UNUSABLE,
                connected_with_top: false,
                var_type: 0,
                var_num: 0,
                indexed: None,
            };
            for column in element.col..=alive_column {
                for row in element.row..element.row.saturating_add(height) {
                    let is_alive = (column, row) == (alive_column, element.row);
                    let occupied = rung
                        .elements
                        .iter()
                        .find(|candidate| candidate.col == column && candidate.row == row);
                    // Our own wire, written when the block was imported: the
                    // reference keeps that cell as part of the block's body, so
                    // only its vertical link is carried over. A real element (an
                    // element the user placed inside the block's rectangle) is a
                    // conflict worth reporting.
                    let is_own_cell = column == element.col && row == element.row;
                    if let Some(candidate) = occupied {
                        if candidate.kind != ElementKind::Connection && !is_alive && !is_own_cell {
                            self.warn(format!(
                                "rung_{}.csv cell ({column},{row}): `{:?}` overlaps the body of \
                                 the block at ({},{}); the block was expanded over it",
                                rung.id, candidate.kind, element.col, element.row
                            ));
                        }
                    }
                    if let Some(line) = matrix.get_mut(usize::from(row)) {
                        if let Some(slot) = line.get_mut(usize::from(column)) {
                            if is_alive {
                                let mut cell = block;
                                if let Some(candidate) = occupied {
                                    cell.connected_with_top = candidate.connected_with_top;
                                }
                                *slot = cell;
                            } else {
                                let mut cell = body;
                                if let Some(candidate) = occupied {
                                    cell.connected_with_top = candidate.connected_with_top;
                                }
                                *slot = cell;
                            }
                        }
                    }
                }
            }
        }
        // An implicitly wired rung conducts through its empty cells, so every
        // gap inside a live row becomes an explicit connection.
        if rung.wire_mode == WireMode::Implicit {
            fill_gaps(rung, &alive, &mut matrix);
        }
        let (prev, next) = self.links.get(&rung.id).copied().unwrap_or((-1, -1));
        let label = one_line(&rung.label);
        let comment = one_line(&rung.comment);
        if label != rung.label || comment != rung.comment {
            self.warn(format!(
                "rung_{}.csv: the label or comment contains a line break, which the format cannot \
                 express; it was flattened",
                rung.id
            ));
        }
        let mut text = String::new();
        text.push_str("#VER=3.0\n");
        text.push_str(&format!("#LABEL={label}\n"));
        if comment.chars().count() > 29 {
            text.push_str(&format!("#COMMENT_LONG={comment}\n"));
        } else {
            text.push_str(&format!("#COMMENT={comment}\n"));
        }
        text.push_str(&format!("#PREVRUNG={prev}\n"));
        text.push_str(&format!("#NEXTRUNG={next}\n"));
        text.push_str(&format!("#NBRLINES={rows}\n"));
        for line in &matrix {
            let rendered: Vec<String> = line.iter().map(Cell::render).collect();
            text.push_str(&rendered.join(" , "));
            text.push('\n');
        }
        text
    }

    /// Encodes one element as a reference matrix cell.
    fn encode_element(
        &mut self,
        rung: &Rung,
        position: usize,
        element: &PlacedElement,
        where_: &str,
    ) -> Option<Cell> {
        let mut cell = Cell {
            etype: mapping::ELE_FREE,
            connected_with_top: element.connected_with_top,
            ..Cell::default()
        };
        match element.kind {
            ElementKind::Connection => cell.etype = mapping::ELE_CONNECTION,
            ElementKind::ContactNo => cell.etype = mapping::ELE_INPUT,
            ElementKind::ContactNc => cell.etype = mapping::ELE_INPUT_NOT,
            ElementKind::ContactRising => cell.etype = mapping::ELE_RISING_INPUT,
            ElementKind::ContactFalling => cell.etype = mapping::ELE_FALLING_INPUT,
            ElementKind::CoilOut => cell.etype = mapping::ELE_OUTPUT,
            ElementKind::CoilOutNeg => cell.etype = mapping::ELE_OUTPUT_NOT,
            ElementKind::CoilSet => cell.etype = mapping::ELE_OUTPUT_SET,
            ElementKind::CoilReset => cell.etype = mapping::ELE_OUTPUT_RESET,
            ElementKind::CoilJump => {
                cell.etype = mapping::ELE_OUTPUT_JUMP;
                cell.var_num = self.jump_target(rung, element, where_);
            }
            ElementKind::CoilCall => {
                cell.etype = mapping::ELE_OUTPUT_CALL;
                match element
                    .params
                    .first()
                    .and_then(|text| text.trim().parse::<i64>().ok())
                {
                    Some(subroutine) => cell.var_num = subroutine,
                    None => self.warn(format!(
                        "{where_}: subroutine call has no numeric parameter; exported as 0"
                    )),
                }
            }
            ElementKind::Timer { .. } => {
                let index = block_index(element, VarKind::TimerIec)?;
                cell.etype = mapping::ELE_TIMER_IEC;
                cell.var_num = i64::from(index);
            }
            ElementKind::Counter { .. } => {
                let index = block_index(element, VarKind::Counter)?;
                cell.etype = mapping::ELE_COUNTER;
                cell.var_num = i64::from(index);
            }
            ElementKind::Register { .. } => {
                let index = block_index(element, VarKind::Register)?;
                cell.etype = mapping::ELE_REGISTER;
                cell.var_num = i64::from(index);
            }
            ElementKind::Compare => {
                cell.etype = mapping::ELE_COMPAR;
                cell.var_num = self.expression_index.get(&(rung.id, position)).copied()?;
            }
            ElementKind::Operate => {
                cell.etype = mapping::ELE_OUTPUT_OPERATE;
                cell.var_num = self.expression_index.get(&(rung.id, position)).copied()?;
            }
        }
        if element.kind.is_contact() || element.kind.is_coil() {
            if matches!(element.kind, ElementKind::CoilJump | ElementKind::CoilCall) {
                return Some(cell);
            }
            let Some(var) = element.var.as_ref() else {
                self.warn(format!(
                    "{where_}: the element has no variable; element omitted"
                ));
                return None;
            };
            match encode_var(var) {
                Ok(encoded) => {
                    cell.var_type = encoded.var_type;
                    cell.var_num = encoded.var_num;
                    cell.indexed = encoded.indexed;
                }
                Err(reason) => {
                    self.warn(format!("{where_}: {reason}; element omitted"));
                    return None;
                }
            }
        }
        Some(cell)
    }

    /// Reference rung index a jump targets.
    fn jump_target(&mut self, rung: &Rung, element: &PlacedElement, where_: &str) -> i64 {
        let project = self.project;
        let owner = project
            .sections
            .iter()
            .find(|section| section.rungs.contains(&rung.id));
        let Some(text) = element.params.first() else {
            self.warn(format!("{where_}: the jump has no target; exported as 0"));
            return 0;
        };
        let text = text.trim();
        if let Ok(position) = text.parse::<usize>() {
            if let Some(target) = owner.and_then(|section| section.rungs.get(position)) {
                return i64::from(*target);
            }
            if let Ok(value) = text.parse::<i64>() {
                self.warn(format!(
                    "{where_}: jump position {position} is outside its section; the number was \
                     written unchanged"
                ));
                return value;
            }
            return 0;
        }
        let target = owner.and_then(|section| {
            section
                .rungs
                .iter()
                .copied()
                .find(|id| project.rung(*id).is_some_and(|rung| rung.label == text))
        });
        match target {
            Some(target) => i64::from(target),
            None => {
                self.warn(format!(
                    "{where_}: jump label `{text}` does not match any rung of its section; exported \
                     as 0"
                ));
                0
            }
        }
    }

    /// `#PREVRUNG`/`#NEXTRUNG` for every rung, from the section lists.
    fn compute_links(&mut self) -> BTreeMap<u32, (i64, i64)> {
        let project = self.project;
        let mut links: BTreeMap<u32, (i64, i64)> = BTreeMap::new();
        for rung in &project.rungs {
            links.insert(rung.id, (-1, -1));
        }
        let mut claimed: BTreeSet<u32> = BTreeSet::new();
        let sections: Vec<Section> = project.sections.clone();
        for section in &sections {
            if section.language != SectionLanguage::Ladder {
                continue;
            }
            for (position, id) in section.rungs.iter().enumerate() {
                if !links.contains_key(id) {
                    self.warn(format!(
                        "sections.csv: section {} references rung {id}, which does not exist",
                        section.id
                    ));
                    continue;
                }
                if !claimed.insert(*id) {
                    self.warn(format!(
                        "sections.csv: rung {id} is used by more than one section; the chain of the \
                         first one is kept"
                    ));
                    continue;
                }
                let previous = if position == 0 {
                    -1
                } else {
                    section
                        .rungs
                        .get(position - 1)
                        .map_or(-1, |id| i64::from(*id))
                };
                let next = section
                    .rungs
                    .get(position + 1)
                    .map_or(-1, |id| i64::from(*id));
                links.insert(*id, (previous, next));
            }
        }
        links
    }

    /// Renders `sections.csv`.
    fn sections_csv(&mut self) -> String {
        let project = self.project;
        let mut text = String::from("#VER=1.0\n");
        for section in &project.sections {
            let name = one_line(&section.name);
            if name != section.name {
                self.warn(format!(
                    "sections.csv: the name of section {} contains a line break; it was flattened",
                    section.id
                ));
            }
            text.push_str(&format!("#NAME{:03}={name}\n", section.id));
        }
        for section in &project.sections {
            let language = match section.language {
                SectionLanguage::Ladder => 0,
                SectionLanguage::Sfc => {
                    self.warn(format!(
                        "sections.csv: section {} is a sequential (SFC) section; its steps and \
                         transitions are not exported",
                        section.id
                    ));
                    1
                }
            };
            let subroutine = section.subroutine.map_or(-1, i64::from);
            let first = section.rungs.first().map_or(0, |id| i64::from(*id));
            let last = section.rungs.last().map_or(0, |id| i64::from(*id));
            text.push_str(&format!(
                "{:03},{language},{subroutine},{first},{last},0\n",
                section.id
            ));
        }
        text
    }

    /// Renders `symbols.csv`.
    fn symbols_csv(&mut self) -> String {
        let project = self.project;
        let mut text = String::from("#VER=1.0\n");
        for symbol in &project.symbols {
            let (variable, name, comment) = self.encode_symbol(symbol);
            text.push_str(&format!("{variable},{name},{comment}\n"));
        }
        text
    }

    /// Renders one symbol row.
    fn encode_symbol(&mut self, symbol: &Symbol) -> (String, String, String) {
        if symbol.unit.is_some() {
            self.warn(format!(
                "symbols.csv: symbol `{}` carries an engineering unit, which the format cannot \
                 express; it was dropped",
                symbol.name
            ));
        }
        let variable = match &symbol.var {
            Some(var) => match encode_var(var) {
                Ok(_) => classic_var_name(var),
                Err(reason) => {
                    self.warn(format!(
                        "symbols.csv: symbol `{}`: {reason}; exported without a variable",
                        symbol.name
                    ));
                    String::new()
                }
            },
            None => {
                self.warn(format!(
                    "symbols.csv: symbol `{}` has no variable; exported without one",
                    symbol.name
                ));
                String::new()
            }
        };
        let name = csv_field(&symbol.name);
        // A comma is harmless in the last field — the reference reads it up to
        // the end of the line — but a line break is not.
        let comment = one_line(&symbol.comment);
        if name != symbol.name || comment != symbol.comment {
            self.warn(format!(
                "symbols.csv: the symbol `{}` contains a comma or a line break; it was replaced",
                symbol.name
            ));
        }
        (variable, name, comment)
    }

    /// Renders `timers_iec.csv`.
    fn timers_csv(&self) -> String {
        let mut text = String::from("#VER=2.0\n");
        for (index, (base, preset, mode)) in &self.timers {
            text.push_str(&format!("TM{index},{base},{preset},{mode}\n"));
        }
        text
    }

    /// Renders `counters.csv`.
    fn counters_csv(&self) -> String {
        let mut text = String::from("#VER=2.0\n");
        for (index, preset) in &self.counters {
            text.push_str(&format!("C{index},{preset}\n"));
        }
        text
    }

    /// Renders `registers.csv`.
    fn registers_csv(&self) -> String {
        let mut text = String::from("#VER=1.0\n");
        for (index, mode) in &self.registers {
            text.push_str(&format!("R{index},{mode}\n"));
        }
        text
    }

    /// Renders `arithmetic_expressions.csv`.
    fn expressions_csv(&self) -> String {
        let mut text = String::from("#VER=2.0\n");
        for (index, expression) in self.expressions.iter().enumerate() {
            text.push_str(&format!("{index:04},{expression}\n"));
        }
        text
    }

    /// Renders `general.txt`, preserving the template's unmodelled keys.
    fn general(&self) -> String {
        let sizes = self.sizes();
        let defaults: Vec<(&str, String)> = vec![
            ("PERIODIC_REFRESH", self.project.scan.period_ms.to_string()),
            (
                "PERIODIC_INPUTS_REFRESH",
                self.project.scan.input_period_ms.to_string(),
            ),
            ("REAL_INPUTS_OUTPUTS_ONLY_ON_TARGET", "0".to_owned()),
            ("AUTO_ADJUST_SUMMER_WINTER_TIME", "0".to_owned()),
            ("SIZE_NBR_RUNGS", sizes.rungs.to_string()),
            ("SIZE_NBR_BITS", sizes.bits.to_string()),
            ("SIZE_NBR_WORDS", sizes.words.to_string()),
            ("SIZE_NBR_TIMERS", sizes.timers.to_string()),
            ("SIZE_NBR_MONOSTABLES", sizes.monostables.to_string()),
            ("SIZE_NBR_COUNTERS", sizes.counters.to_string()),
            ("SIZE_NBR_TIMERS_IEC", sizes.timers_iec.to_string()),
            ("SIZE_NBR_REGISTERS", sizes.registers.to_string()),
            ("SIZE_REGISTER_LIST", sizes.register_list.to_string()),
            ("SIZE_NBR_PHYS_INPUTS", sizes.phys_inputs.to_string()),
            ("SIZE_NBR_PHYS_OUTPUTS", sizes.phys_outputs.to_string()),
            ("SIZE_NBR_ARITHM_EXPR", sizes.arithm_expr.to_string()),
            ("SIZE_NBR_SECTIONS", sizes.sections.to_string()),
            ("SIZE_NBR_SYMBOLS", sizes.symbols.to_string()),
            (
                "SIZE_NBR_PHYS_WORDS_INPUTS",
                sizes.phys_words_inputs.to_string(),
            ),
            (
                "SIZE_NBR_PHYS_WORDS_OUTPUTS",
                sizes.phys_words_outputs.to_string(),
            ),
            ("MODBUS_MASTER_SERIAL_PORT", String::new()),
            ("MODBUS_MASTER_SERIAL_SPEED", "9600".to_owned()),
            ("MODBUS_MASTER_SERIAL_DATABITS", "8".to_owned()),
            ("MODBUS_MASTER_SERIAL_PARITY", "0".to_owned()),
            ("MODBUS_MASTER_SERIAL_STOPBITS", "1".to_owned()),
        ];
        let overrides: &[&str] = &[
            "PERIODIC_REFRESH",
            "PERIODIC_INPUTS_REFRESH",
            "SIZE_NBR_RUNGS",
            "SIZE_NBR_BITS",
            "SIZE_NBR_WORDS",
            "SIZE_NBR_TIMERS",
            "SIZE_NBR_MONOSTABLES",
            "SIZE_NBR_COUNTERS",
            "SIZE_NBR_TIMERS_IEC",
            "SIZE_NBR_REGISTERS",
            "SIZE_REGISTER_LIST",
            "SIZE_NBR_PHYS_INPUTS",
            "SIZE_NBR_PHYS_OUTPUTS",
            "SIZE_NBR_ARITHM_EXPR",
            "SIZE_NBR_SECTIONS",
            "SIZE_NBR_SYMBOLS",
            "SIZE_NBR_PHYS_WORDS_INPUTS",
            "SIZE_NBR_PHYS_WORDS_OUTPUTS",
        ];
        merge_key_values(self.extras.part("general.txt"), &defaults, overrides)
    }

    /// Counts the array sizes the project needs.
    fn sizes(&self) -> Sizes {
        let mut sizes = Sizes {
            rungs: self.project.rungs.len().max(MIN_RUNGS),
            sections: self.project.sections.len().max(MIN_SECTIONS),
            symbols: self.project.symbols.len().max(MIN_SYMBOLS),
            arithm_expr: self.expressions.len().max(MIN_ARITHM_EXPR),
            ..Sizes::default()
        };
        for rung in &self.project.rungs {
            for element in &rung.elements {
                if let Some(var) = element.var.as_ref() {
                    sizes.observe(var);
                }
                if matches!(element.kind, ElementKind::Register { .. }) {
                    if let Some(capacity) = element
                        .params
                        .first()
                        .and_then(|text| text.trim().parse::<u32>().ok())
                    {
                        sizes.register_list = sizes.register_list.max(capacity);
                    }
                }
            }
        }
        for symbol in &self.project.symbols {
            if let Some(var) = symbol.var.as_ref() {
                sizes.observe(var);
            }
        }
        sizes
    }

    /// Renders `project_infos.txt`, preserving the template's other keys.
    fn project_infos(&self) -> String {
        let comment = self.project.comment.replace('\n', "\\n");
        let defaults: Vec<(&str, String)> = vec![
            ("PROJECT_NAME", self.project.name.clone()),
            ("PROJECT_SITE", String::new()),
            ("PARAM_VERSION", String::new()),
            ("PARAM_AUTHOR", self.project.author.clone()),
            ("PARAM_COMPANY", String::new()),
            ("CREA_DATE", String::new()),
            ("MODIF_DATE", String::new()),
            ("PARAM_COMMENT", comment),
        ];
        let overrides: &[&str] = &["PROJECT_NAME", "PARAM_AUTHOR", "PARAM_COMMENT"];
        merge_key_values(self.extras.part("project_infos.txt"), &defaults, overrides)
    }
}

/// The `SIZE_*` hints computed from a project.
struct Sizes {
    rungs: usize,
    bits: u32,
    words: u32,
    timers: usize,
    monostables: usize,
    counters: usize,
    timers_iec: usize,
    registers: usize,
    register_list: u32,
    phys_inputs: u32,
    phys_outputs: u32,
    arithm_expr: usize,
    sections: usize,
    symbols: usize,
    phys_words_inputs: u32,
    phys_words_outputs: u32,
}

impl Default for Sizes {
    fn default() -> Self {
        Self {
            rungs: MIN_RUNGS,
            bits: MIN_BITS,
            words: MIN_WORDS,
            timers: MIN_TIMERS,
            monostables: MIN_MONOSTABLES,
            counters: MIN_COUNTERS,
            timers_iec: MIN_TIMERS_IEC,
            registers: MIN_REGISTERS,
            register_list: MIN_REGISTER_LIST,
            phys_inputs: MIN_PHYS_INPUTS,
            phys_outputs: MIN_PHYS_OUTPUTS,
            arithm_expr: MIN_ARITHM_EXPR,
            sections: MIN_SECTIONS,
            symbols: MIN_SYMBOLS,
            phys_words_inputs: MIN_PHYS_WORDS_INPUTS,
            phys_words_outputs: MIN_PHYS_WORDS_OUTPUTS,
        }
    }
}

impl Sizes {
    /// Widens the computed sizes for one variable reference.
    fn observe(&mut self, var: &VarRef) {
        let needed = var.index.saturating_add(1);
        match var.kind {
            VarKind::MemBit
            | VarKind::PhysIn
            | VarKind::PhysOut
            | VarKind::System
            | VarKind::Led => self.bits = self.bits.max(needed),
            VarKind::MemWord => self.words = self.words.max(needed),
            VarKind::PhysInWord => self.phys_words_inputs = self.phys_words_inputs.max(needed),
            VarKind::PhysOutWord => self.phys_words_outputs = self.phys_words_outputs.max(needed),
            VarKind::TimerIec => {
                self.timers_iec = self
                    .timers_iec
                    .max(usize::try_from(needed).unwrap_or(usize::MAX));
            }
            VarKind::Counter => {
                self.counters = self
                    .counters
                    .max(usize::try_from(needed).unwrap_or(usize::MAX));
            }
            VarKind::Register => {
                self.registers = self
                    .registers
                    .max(usize::try_from(needed).unwrap_or(usize::MAX));
            }
            VarKind::Step => {}
        }
        if let Some(index) = var.index_expr.as_deref() {
            self.observe(index);
        }
    }
}

/// Fills the gaps of one implicitly wired rung with connections.
fn fill_gaps(rung: &Rung, alive: &BTreeMap<(u8, u8), Cell>, matrix: &mut [Vec<Cell>]) {
    let mut extent: BTreeMap<u8, (u8, u8)> = BTreeMap::new();
    for element in &rung.elements {
        let entry = extent.entry(element.row).or_insert((u8::MAX, 0));
        entry.0 = entry.0.min(element.col);
        entry.1 = entry.1.max(element.col);
    }
    for (row, (first, last)) in extent {
        // An empty column 0 does not touch the rail: a branch that taps in
        // mid-rung must not be fed from it, so the fill starts at column 1
        // unless an element really sits in column 0.
        let start = if first == 0 { 0 } else { 1 };
        for column in start..=last {
            if alive.contains_key(&(column, row)) {
                continue;
            }
            if let Some(line) = matrix.get_mut(usize::from(row)) {
                if let Some(slot) = line.get_mut(usize::from(column)) {
                    if slot.etype == mapping::ELE_FREE {
                        slot.etype = mapping::ELE_CONNECTION;
                    }
                }
            }
        }
    }
}

/// The instance variable of a function block, when it has the right kind.
fn block_index(element: &PlacedElement, kind: VarKind) -> Option<u32> {
    element
        .var
        .as_ref()
        .filter(|var| var.kind == kind)
        .map(|var| var.index)
}

/// Assembles the exported document: regenerated parts in reference order, then
/// every passthrough part of the template.
fn assemble(
    generated: &mut Vec<(String, String)>,
    rungs: Vec<(u32, String)>,
    extras: &Document,
) -> Document {
    let mut out: Vec<(String, String)> = Vec::new();
    for name in CANONICAL_PARTS {
        if name.is_empty() {
            // The rung files sit between the expressions and the sections.
            for (id, contents) in &rungs {
                out.push((format!("rung_{id}.csv"), contents.clone()));
            }
            continue;
        }
        if let Some(position) = generated.iter().position(|(part, _)| part == name) {
            out.push(generated.remove(position));
            continue;
        }
        if let Some(contents) = extras.part(name) {
            out.push((name.to_owned(), contents.to_owned()));
        }
    }
    for (name, contents) in extras.parts() {
        if CANONICAL_PARTS.contains(&name.as_str()) {
            continue;
        }
        out.push((name.clone(), contents.clone()));
    }
    Document::from_parts(out)
}

/// Merges a `KEY=VALUE` part with the values the project owns.
///
/// Lines already in the template keep their position; the keys listed in
/// `overrides` are rewritten in place, `defaults` supplies the value of every
/// key (and the canonical order of the keys that have to be appended), and
/// every other line — including keys SoftLadder does not model — is preserved.
fn merge_key_values(
    original: Option<&str>,
    defaults: &[(&str, String)],
    overrides: &[&str],
) -> String {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut text = String::new();
    if let Some(original) = original {
        for line in original.lines() {
            match line.split_once('=') {
                Some((key, _)) => {
                    seen.insert(key.to_owned());
                    if overrides.contains(&key) {
                        if let Some((_, value)) = defaults.iter().find(|(name, _)| *name == key) {
                            text.push_str(&format!("{key}={value}\n"));
                            continue;
                        }
                    }
                    text.push_str(line);
                    text.push('\n');
                }
                None => {
                    text.push_str(line);
                    text.push('\n');
                }
            }
        }
    }
    for (key, value) in defaults {
        if !seen.contains(*key) {
            text.push_str(&format!("{key}={value}\n"));
        }
    }
    text
}

/// Removes the optional leading `=` of an operate block's value.
fn strip_assignment_marker(text: &str) -> &str {
    let trimmed = text.trim_start();
    if let Some(rest) = trimmed.strip_prefix('=') {
        if !rest.starts_with('=') {
            return rest;
        }
    }
    trimmed
}

/// The reference's `%`-spelling of a variable, with `%B`/`%W` for the memory
/// families the reference names that way.
fn classic_var_name(var: &VarRef) -> String {
    let text = var.to_string();
    match var.kind {
        VarKind::MemBit => text.replacen("%M", "%B", 1),
        VarKind::MemWord => text.replacen("%MW", "%W", 1),
        _ => text,
    }
}

/// Flattens a field the format cannot store on more than one line.
fn one_line(text: &str) -> String {
    text.replace(['\n', '\r'], " ")
}

/// Escapes the separators of a `<varname>,<symbol>,<comment>` row.
fn csv_field(text: &str) -> String {
    one_line(text).replace(',', ";")
}
