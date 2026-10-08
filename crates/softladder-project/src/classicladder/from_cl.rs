//! ClassicLadder → SoftLadder: the element-level import.
//!
//! The importer reads the parts that SoftLadder models (`general.txt`,
//! `project_infos.txt`, `rung_<n>.csv`, `sections.csv`, `symbols.csv`, the
//! instance CSVs and `arithmetic_expressions.csv`), maps every cell and
//! variable through [`super::mapping`], and hands back the remaining parts as a
//! [`Document`] so the exporter can pass them through unchanged.
//!
//! Nothing here panics on hostile input: a cell, a line or a whole part that
//! cannot be understood becomes an `SL-E030` error diagnostic and is skipped.

use std::collections::{BTreeMap, BTreeSet};

use softladder_core::model::WireMode;
use softladder_core::{
    Diagnostic, ElementKind, PlacedElement, Project, Rung, ScanConfig, Section, SectionLanguage,
    Severity, Symbol, VarKind, VarRef,
};

use super::document::Document;
use super::expr_map;
use super::mapping::{self, decode_var, DecodedVar, IMPORTED_COUNTER_KIND, REGISTER_MODE_UNDEF};
use super::ImportReport;
use crate::ProjectError;

/// Default capacity of a ClassicLadder register list (`SIZE_REGISTER_LIST`).
const DEFAULT_REGISTER_CAPACITY: i64 = 500;

/// Imports a parsed ClassicLadder container into a SoftLadder project.
///
/// # Errors
///
/// The container has already been parsed by [`Document::parse`], so a part that
/// cannot be understood is reported as an `SL-E030` [`Diagnostic`] rather than
/// failing the whole import.
pub(crate) fn import(document: &Document) -> Result<ImportReport, ProjectError> {
    let mut importer = Importer::new(document);
    importer.run();
    Ok(ImportReport {
        project: importer.project,
        diagnostics: importer.diagnostics,
        extras: Document::from_parts(importer.extras),
    })
}

/// The importer's working state.
struct Importer {
    parts: Vec<(String, String)>,
    diagnostics: Vec<Diagnostic>,
    extras: Vec<(String, String)>,
    project: Project,
    register_capacity: i64,
    timers_iec: BTreeMap<u32, (i64, i64, i64)>,
    timers: BTreeMap<u32, (i64, i64)>,
    monostables: BTreeMap<u32, (i64, i64)>,
    counters: BTreeMap<u32, i64>,
    registers: BTreeMap<u32, i64>,
    expressions: BTreeMap<i64, String>,
    links: BTreeMap<u32, (i64, i64)>,
}

/// One cell of the reference matrix, as it appears in the file.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RawCell {
    etype: i64,
    connected_with_top: bool,
    var_type: i64,
    var_num: i64,
    indexed: Option<(i64, i64)>,
}

/// Finds the raw cell at `(column, row)`, if the rung had one there.
fn raw_cell(raw: &[(u8, u8, RawCell)], column: u8, row: u8) -> Option<&RawCell> {
    raw.iter()
        .find(|(candidate_column, candidate_row, _)| {
            *candidate_column == column && *candidate_row == row
        })
        .map(|(_, _, cell)| cell)
}

/// One `sections.csv` record.
#[derive(Debug, Clone)]
struct RawSection {
    index: u32,
    language: i64,
    subroutine: i64,
    first_rung: i64,
    last_rung: i64,
    line: usize,
}

/// A parsed rung file, before the sections have been resolved.
struct RawRung {
    id: u32,
    rung: Rung,
    prev: i64,
    next: i64,
}

impl Importer {
    fn new(document: &Document) -> Self {
        Self {
            parts: document.parts().to_vec(),
            diagnostics: Vec::new(),
            extras: Vec::new(),
            project: Project::new(""),
            register_capacity: DEFAULT_REGISTER_CAPACITY,
            timers_iec: BTreeMap::new(),
            timers: BTreeMap::new(),
            monostables: BTreeMap::new(),
            counters: BTreeMap::new(),
            registers: BTreeMap::new(),
            expressions: BTreeMap::new(),
            links: BTreeMap::new(),
        }
    }

    /// Runs every pass of the import.
    fn run(&mut self) {
        self.collect_extras();
        self.read_general();
        self.read_project_infos();
        self.read_timers_iec();
        self.read_legacy_timers("timers.csv", true);
        self.read_legacy_timers("monostables.csv", false);
        self.read_counters();
        self.read_registers();
        self.read_expressions();
        self.read_symbols();

        let mut rungs = self.read_rungs();
        let sections = self.read_sections();
        self.fix_jumps(&mut rungs, &sections);
        self.project.sections = sections;
        rungs.sort_by_key(|raw| raw.id);
        for raw in rungs {
            self.project.rungs.push(raw.rung);
        }
    }

    /// Records one diagnostic, naming the part and (when known) the rung.
    fn report(
        &mut self,
        severity: Severity,
        code: &'static str,
        message: String,
        rung: Option<u32>,
    ) {
        let mut diagnostic = Diagnostic::new(severity, code, message);
        if let Some(rung) = rung {
            diagnostic = diagnostic.with_rung(rung as usize);
        }
        self.diagnostics.push(diagnostic);
    }

    /// Records a warning.
    fn warn(&mut self, code: &'static str, message: String, rung: Option<u32>) {
        self.report(Severity::Warning, code, message, rung);
    }

    /// Records an `SL-E030` malformed-document error.
    fn fail(&mut self, message: String, rung: Option<u32>) {
        self.report(Severity::Error, "SL-E030", message, rung);
    }

    /// Splits the document into modelled parts and passthrough parts.
    fn collect_extras(&mut self) {
        let parts = self.parts.clone();
        for (name, content) in &parts {
            if is_regenerated(name) {
                continue;
            }
            if !is_mergeable(name) {
                self.warn(
                    "SL-W032",
                    format!(
                        "{name}: part is passed through unchanged (SoftLadder does not model it)"
                    ),
                    None,
                );
            }
            self.extras.push((name.clone(), content.clone()));
        }
    }

    /// Reads `general.txt`: the scan periods and the register list size.
    fn read_general(&mut self) {
        let Some(content) = self.part("general.txt").map(str::to_owned) else {
            self.warn(
                "SL-W030",
                "general.txt: missing, so the scan periods keep their defaults".to_owned(),
                None,
            );
            return;
        };
        for (line, text) in numbered_lines(&content) {
            if let Some(value) = value_of(text, "PERIODIC_REFRESH") {
                self.project.scan.period_ms = self.read_period(
                    line,
                    "PERIODIC_REFRESH",
                    value,
                    ScanConfig::default().period_ms,
                );
            } else if let Some(value) = value_of(text, "PERIODIC_INPUTS_REFRESH") {
                self.project.scan.input_period_ms = self.read_period(
                    line,
                    "PERIODIC_INPUTS_REFRESH",
                    value,
                    ScanConfig::default().input_period_ms,
                );
            } else if let Some(value) = value_of(text, "SIZE_REGISTER_LIST") {
                match value.trim().parse::<i64>() {
                    // ClassicLadder's `SIZE_*` hints are informational: the
                    // register capacity is kept at least at the reference
                    // minimum so that exporting the project again reproduces the
                    // same general.txt.
                    Ok(value) if value >= 0 => {
                        self.register_capacity = value.max(DEFAULT_REGISTER_CAPACITY);
                    }
                    _ => self.fail(
                        format!(
                            "general.txt line {line}: `SIZE_REGISTER_LIST={value}` is not a usable number"
                        ),
                        None,
                    ),
                }
            }
        }
    }

    /// Reads one scan period, reporting an unusable number.
    fn read_period(&mut self, line: usize, key: &str, value: &str, fallback: u32) -> u32 {
        match value.trim().parse::<u32>() {
            Ok(value) => value,
            Err(_) => {
                self.fail(
                    format!("general.txt line {line}: `{key}={value}` is not a usable number"),
                    None,
                );
                fallback
            }
        }
    }

    /// Reads `project_infos.txt`: name, author and comment.
    fn read_project_infos(&mut self) {
        let Some(content) = self.part("project_infos.txt").map(str::to_owned) else {
            return;
        };
        for text in numbered_lines(&content).map(|(_, text)| text) {
            let Some((key, value)) = text.split_once('=') else {
                continue;
            };
            match key {
                "PROJECT_NAME" => self.project.name = value.to_owned(),
                "PARAM_AUTHOR" => self.project.author = value.to_owned(),
                "PARAM_COMMENT" => self.project.comment = value.replace("\\n", "\n"),
                _ => {}
            }
        }
    }

    /// Reads `timers_iec.csv`.
    ///
    /// Newer files number every row (`TM<n>,<base>,<preset>,<mode>`); older
    /// ones spell the rows positionally (`<base>,<preset>,<mode>`) and rely on
    /// the row order, exactly like the reference loader.
    fn read_timers_iec(&mut self) {
        let Some(content) = self.part("timers_iec.csv").map(str::to_owned) else {
            return;
        };
        let mut running = 0i64;
        for (line, text) in numbered_lines(&content) {
            if text.starts_with('#') || text.starts_with(';') {
                continue;
            }
            let fields = comma_numbers(strip_letter_prefix(text, "TM"));
            let (index, base, preset, mode) = if text.starts_with("TM") {
                match fields.as_slice() {
                    [index, base, preset, mode] => (*index, *base, *preset, *mode),
                    [index, base, preset] => (*index, *base, *preset, mapping::TIMER_MODE_ON),
                    _ => {
                        self.fail(
                            format!("timers_iec.csv line {line}: cannot read `{text}`"),
                            None,
                        );
                        continue;
                    }
                }
            } else {
                let index = running;
                running = running.saturating_add(1);
                match fields.as_slice() {
                    [base, preset, mode] => (index, *base, *preset, *mode),
                    [base, preset] => (index, *base, *preset, mapping::TIMER_MODE_ON),
                    [] if text.is_empty() => (index, mapping::BASE_MINS, 0, mapping::TIMER_MODE_ON),
                    _ => {
                        self.fail(
                            format!("timers_iec.csv line {line}: cannot read `{text}`"),
                            None,
                        );
                        continue;
                    }
                }
            };
            let Ok(index) = u32::try_from(index) else {
                self.fail(
                    format!("timers_iec.csv line {line}: timer number {index} is out of range"),
                    None,
                );
                continue;
            };
            if mapping::base_millis(base).is_none() {
                self.fail(
                    format!("timers_iec.csv line {line}: unknown time base {base}"),
                    None,
                );
                continue;
            }
            self.timers_iec.insert(index, (base, preset, mode));
        }
    }

    /// Reads `timers.csv` or `monostables.csv`, whose rows come in a numbered
    /// and a legacy positional spelling.
    fn read_legacy_timers(&mut self, part: &str, timers: bool) {
        let Some(content) = self.part(part).map(str::to_owned) else {
            return;
        };
        let letter = if timers { "T" } else { "M" };
        let mut running = 0u32;
        for (line, text) in numbered_lines(&content) {
            if text.starts_with('#') || text.starts_with(';') {
                continue;
            }
            let numbered = text.starts_with(letter);
            let fields = comma_numbers(strip_letter_prefix(text, letter));
            let (index, base, preset) = if numbered {
                match fields.as_slice() {
                    [index, base, preset] => (*index, *base, *preset),
                    _ => {
                        self.fail(format!("{part} line {line}: cannot read `{text}`"), None);
                        continue;
                    }
                }
            } else {
                let index = i64::from(running);
                running = running.saturating_add(1);
                match fields.as_slice() {
                    [base, preset] => (index, *base, *preset),
                    [] if text.is_empty() => (index, mapping::BASE_MINS, 0),
                    _ => {
                        self.fail(format!("{part} line {line}: cannot read `{text}`"), None);
                        continue;
                    }
                }
            };
            let Ok(index) = u32::try_from(index) else {
                self.fail(
                    format!("{part} line {line}: block number {index} is out of range"),
                    None,
                );
                continue;
            };
            if mapping::base_millis(base).is_none() {
                self.fail(
                    format!("{part} line {line}: unknown time base {base}"),
                    None,
                );
                continue;
            }
            if timers {
                self.timers.insert(index, (base, preset));
            } else {
                self.monostables.insert(index, (base, preset));
            }
        }
    }

    /// Reads `counters.csv`, in its numbered and legacy positional spelling.
    fn read_counters(&mut self) {
        let Some(content) = self.part("counters.csv").map(str::to_owned) else {
            return;
        };
        let mut running = 0u32;
        for (line, text) in numbered_lines(&content) {
            if text.starts_with('#') || text.starts_with(';') {
                continue;
            }
            let numbered = text.starts_with('C');
            let fields = comma_numbers(strip_letter_prefix(text, "C"));
            let (index, preset) = if numbered {
                match fields.as_slice() {
                    [index, preset] => (*index, *preset),
                    _ => {
                        self.fail(
                            format!("counters.csv line {line}: cannot read `{text}`"),
                            None,
                        );
                        continue;
                    }
                }
            } else {
                let index = i64::from(running);
                running = running.saturating_add(1);
                match fields.as_slice() {
                    [preset] => (index, *preset),
                    [] if text.is_empty() => (index, 0),
                    _ => {
                        self.fail(
                            format!("counters.csv line {line}: cannot read `{text}`"),
                            None,
                        );
                        continue;
                    }
                }
            };
            let Ok(index) = u32::try_from(index) else {
                self.fail(
                    format!("counters.csv line {line}: counter number {index} is out of range"),
                    None,
                );
                continue;
            };
            self.counters.insert(index, preset);
        }
    }

    /// Reads `registers.csv`.
    fn read_registers(&mut self) {
        let Some(content) = self.part("registers.csv").map(str::to_owned) else {
            return;
        };
        for (line, text) in numbered_lines(&content) {
            if is_comment_line(text) {
                continue;
            }
            let fields = comma_numbers(strip_letter_prefix(text, "R"));
            let [index, mode] = fields.as_slice() else {
                self.fail(
                    format!("registers.csv line {line}: cannot read `{text}`"),
                    None,
                );
                continue;
            };
            let Ok(index) = u32::try_from(*index) else {
                self.fail(
                    format!("registers.csv line {line}: register number {index} is out of range"),
                    None,
                );
                continue;
            };
            self.registers.insert(index, *mode);
        }
    }

    /// Reads `arithmetic_expressions.csv`.
    ///
    /// Newer files number every row (`0000,<expr>`); older ones rely on the
    /// position of the row, counting blank lines exactly like the reference
    /// loader does.
    fn read_expressions(&mut self) {
        let Some(content) = self.part("arithmetic_expressions.csv").map(str::to_owned) else {
            return;
        };
        let mut position = 0i64;
        for (_, text) in numbered_lines(&content) {
            if text.starts_with('#') || text.starts_with(';') {
                continue;
            }
            if text.starts_with(|character: char| character.is_ascii_digit()) {
                let digits = text
                    .find(|character: char| !character.is_ascii_digit())
                    .unwrap_or(text.len());
                let index = text[..digits].parse::<i64>().unwrap_or(0);
                let expression = text.get(digits + 1..).unwrap_or("").to_owned();
                if !expression.is_empty() {
                    self.expressions.insert(index, expression);
                }
            } else {
                if !text.is_empty() {
                    self.expressions.insert(position, text.to_owned());
                }
                position = position.saturating_add(1);
            }
        }
    }

    /// Reads `symbols.csv`.
    fn read_symbols(&mut self) {
        let Some(content) = self.part("symbols.csv").map(str::to_owned) else {
            return;
        };
        for (line, text) in numbered_lines(&content) {
            if is_comment_line(text) {
                continue;
            }
            let mut fields = text.splitn(3, ',');
            let var_name = fields.next().unwrap_or("").trim();
            let name = fields.next().unwrap_or("").trim().to_owned();
            let comment = fields.next().unwrap_or("").trim().to_owned();
            if name.is_empty() && var_name.is_empty() {
                continue;
            }
            let var = if var_name.is_empty() {
                None
            } else {
                match var_name.parse::<VarRef>() {
                    Ok(var) => Some(var),
                    Err(error) => {
                        self.warn(
                            "SL-W031",
                            format!(
                                "symbols.csv line {line}: variable `{var_name}` (symbol `{name}`) is \
                                 not modelled ({error})"
                            ),
                            None,
                        );
                        None
                    }
                }
            };
            self.project.symbols.push(Symbol {
                name,
                var,
                comment,
                unit: None,
            });
        }
    }

    /// Reads and maps every `rung_<n>.csv` part.
    fn read_rungs(&mut self) -> Vec<RawRung> {
        let mut names: Vec<(u32, String)> = self
            .parts
            .iter()
            .filter_map(|(name, _)| rung_index(name).map(|index| (index, name.clone())))
            .collect();
        names.sort_by_key(|(index, _)| *index);
        let mut rungs = Vec::new();
        for (id, name) in names {
            let Some(content) = self.part(&name).map(str::to_owned) else {
                continue;
            };
            match self.read_rung(id, &name, &content) {
                Some(raw) => {
                    self.links.insert(id, (raw.prev, raw.next));
                    rungs.push(raw);
                }
                None => self.fail(format!("{name}: the rung file cannot be read"), Some(id)),
            }
        }
        rungs
    }

    /// Reads one rung file.
    fn read_rung(&mut self, id: u32, part: &str, content: &str) -> Option<RawRung> {
        let mut rung = Rung::new(id);
        // Imported rungs are wired explicitly, so a gap breaks the circuit
        // exactly as it does in ClassicLadder.
        rung.wire_mode = WireMode::Explicit;
        let mut prev = -1i64;
        let mut next = -1i64;
        let mut comment_long: Option<String> = None;
        let mut raw_cells: Vec<(u8, u8, RawCell)> = Vec::new();
        let mut row = 0u8;
        for (line, text) in numbered_lines(content) {
            if text.is_empty() || text.starts_with(';') {
                continue;
            }
            if let Some(value) = text.strip_prefix('#') {
                if let Some(value) = value.strip_prefix("VER=") {
                    let major = value
                        .split('.')
                        .next()
                        .and_then(|major| major.trim().parse::<u32>().ok());
                    if major.is_none_or(|major| major > 3) {
                        self.fail(
                            format!("{part} line {line}: unsupported rung version `{value}`"),
                            Some(id),
                        );
                    }
                } else if let Some(value) = value.strip_prefix("LABEL=") {
                    rung.label = value.trim().to_owned();
                } else if let Some(value) = value.strip_prefix("COMMENT_LONG=") {
                    comment_long = Some(value.trim().to_owned());
                } else if let Some(value) = value.strip_prefix("COMMENT=") {
                    rung.comment = value.trim().to_owned();
                } else if let Some(value) = value.strip_prefix("PREVRUNG=") {
                    prev = parse_int(value).unwrap_or(-1);
                } else if let Some(value) = value.strip_prefix("NEXTRUNG=") {
                    next = parse_int(value).unwrap_or(-1);
                }
                continue;
            }
            for (column, cell) in text.split(',').enumerate() {
                let Ok(column) = u8::try_from(column) else {
                    self.fail(
                        format!("{part} line {line}: the rung has more than 256 columns"),
                        Some(id),
                    );
                    break;
                };
                let cell = cell.trim();
                if cell.is_empty() {
                    continue;
                }
                let raw = match parse_cell(cell) {
                    Ok(raw) => raw,
                    Err(reason) => {
                        self.fail(
                            format!("{part} line {line} cell ({column},{row}): {reason}"),
                            Some(id),
                        );
                        continue;
                    }
                };
                if let Some(element) = self.map_cell(&raw, column, row, id, part, line) {
                    rung.elements.push(element);
                }
                raw_cells.push((column, row, raw));
            }
            row = row.saturating_add(1);
        }
        if let Some(comment) = comment_long {
            rung.comment = comment;
        }
        self.place_blocks_on_their_body_column(&mut rung, id, &raw_cells);
        Some(RawRung {
            id,
            rung,
            prev,
            next,
        })
    }

    /// Moves every multi-cell block onto its reference *body* column.
    ///
    /// ClassicLadder reads a block's inputs with `StateOnLeft(alive_column - 1, …)`,
    /// i.e. the power arriving at the column of the block's body, while
    /// SoftLadder reads a cell's input as `state_on_left(column, …)`. Placing the
    /// block on the body column therefore reproduces the reference's enable
    /// exactly; the columns it vacates (up to and including the reference's
    /// "alive" cell, where the block's output leaves in the original) are filled
    /// with wires so the flow still reaches the next column.
    ///
    /// Vertical links recorded on the block's body cells are materialised as
    /// linked `Connection` cells, because the reference's input tap walks the
    /// links of the body column and the corpus uses them.
    fn place_blocks_on_their_body_column(
        &mut self,
        rung: &mut Rung,
        rung_id: u32,
        raw: &[(u8, u8, RawCell)],
    ) {
        let mut occupied: Vec<(u8, u8)> = rung.elements.iter().map(|e| (e.col, e.row)).collect();
        let mut moves: Vec<(usize, u8)> = Vec::new();
        for (index, element) in rung.elements.iter().enumerate() {
            let (width, _) = mapping::block_geometry(&element.kind);
            if width <= 1 {
                continue;
            }
            if element.col < width - 1 {
                self.warn(
                    "SL-W030",
                    format!(
                        "rung_{rung_id}.csv cell ({},{}): a {width}-column block has no room for \
                         its body; it was kept where it is",
                        element.col, element.row
                    ),
                    Some(rung_id),
                );
                continue;
            }
            moves.push((index, element.col));
        }

        let mut additions: Vec<PlacedElement> = Vec::new();
        for (index, alive_column) in moves {
            let (width, height) = {
                let element = &rung.elements[index];
                mapping::block_geometry(&element.kind)
            };
            let row = rung.elements[index].row;
            let body_column = alive_column - (width - 1);
            if occupied.contains(&(body_column, row)) {
                self.warn(
                    "SL-W030",
                    format!(
                        "rung_{rung_id}.csv cell ({body_column},{row}): the block's body column is \
                         already occupied; the block was kept at column {alive_column}"
                    ),
                    Some(rung_id),
                );
                continue;
            }
            occupied.retain(|cell| *cell != (alive_column, row));
            occupied.push((body_column, row));
            let element = &mut rung.elements[index];
            element.col = body_column;
            if let Some(cell) = raw_cell(raw, body_column, row) {
                element.connected_with_top = cell.connected_with_top;
            }

            // Carry the block's output rightwards, through the cells the
            // reference keeps as the block's body and alive cells.
            for column in (body_column + 1)..=alive_column {
                let cell = (column, row);
                if occupied.contains(&cell) {
                    continue;
                }
                occupied.push(cell);
                let mut connection = PlacedElement::new(ElementKind::Connection, column, row);
                connection.connected_with_top =
                    raw_cell(raw, column, row).is_some_and(|cell| cell.connected_with_top);
                additions.push(connection);
            }

            // Links stored on the rest of the block's body must survive: the
            // reference's input tap walks them.
            for offset in 1..height {
                let body_row = row.saturating_add(offset);
                for column in body_column..=alive_column {
                    let cell = (column, body_row);
                    if occupied.contains(&cell) {
                        continue;
                    }
                    if raw_cell(raw, column, body_row).is_some_and(|cell| cell.connected_with_top) {
                        occupied.push(cell);
                        additions.push(
                            PlacedElement::new(ElementKind::Connection, column, body_row)
                                .linked_up(),
                        );
                    }
                }
            }
        }

        rung.elements.extend(additions);
        rung.elements
            .sort_by_key(|element| (element.row, element.col));
    }

    /// Maps one cell onto a SoftLadder element.
    fn map_cell(
        &mut self,
        cell: &RawCell,
        column: u8,
        row: u8,
        rung: u32,
        part: &str,
        line: usize,
    ) -> Option<PlacedElement> {
        let where_ = format!("{part} line {line} cell ({column},{row})");
        let kind = match cell.etype {
            mapping::ELE_FREE => {
                // A free cell produces nothing, except that ClassicLadder uses
                // it to carry a vertical link: that link is real wiring and has
                // to be materialised or the circuit changes shape.
                if cell.connected_with_top {
                    return Some(
                        PlacedElement::new(ElementKind::Connection, column, row).linked_up(),
                    );
                }
                return None;
            }
            mapping::ELE_UNUSABLE => return None,
            mapping::ELE_INPUT => ElementKind::ContactNo,
            mapping::ELE_INPUT_NOT => ElementKind::ContactNc,
            mapping::ELE_RISING_INPUT => ElementKind::ContactRising,
            mapping::ELE_FALLING_INPUT => ElementKind::ContactFalling,
            mapping::ELE_CONNECTION => {
                let mut element = PlacedElement::new(ElementKind::Connection, column, row);
                element.connected_with_top = cell.connected_with_top;
                return Some(element);
            }
            mapping::ELE_TIMER | mapping::ELE_MONOSTABLE => {
                let pulse = cell.etype == mapping::ELE_MONOSTABLE;
                let table = if pulse {
                    &self.monostables
                } else {
                    &self.timers
                };
                let preset = legacy_preset(table, cell.var_num);
                let name = if pulse { "ELE_MONOSTABLE" } else { "ELE_TIMER" };
                let replacement = if pulse {
                    "pulse timer"
                } else {
                    "on-delay timer"
                };
                self.warn(
                    "SL-W031",
                    format!(
                        "{where_}: deprecated `{name}` block imported as an IEC {replacement}; the \
                         legacy timer family is not modelled"
                    ),
                    Some(rung),
                );
                let var = self.block_var(cell, rung, &where_)?;
                let mut element = PlacedElement::new(
                    ElementKind::Timer {
                        mode: mapping::timer_mode(if pulse {
                            mapping::TIMER_MODE_PULSE
                        } else {
                            mapping::TIMER_MODE_ON
                        }),
                    },
                    column,
                    row,
                );
                element.var = Some(var);
                element.connected_with_top = cell.connected_with_top;
                element.params = vec![preset];
                return Some(element);
            }
            mapping::ELE_TIMER_IEC => {
                let Some(index) = u32::try_from(cell.var_num).ok() else {
                    self.warn(
                        "SL-W030",
                        format!(
                            "{where_}: timer instance {} is out of range; cell skipped",
                            cell.var_num
                        ),
                        Some(rung),
                    );
                    return None;
                };
                let (base, preset, mode) = self.timers_iec.get(&index).copied().unwrap_or((
                    mapping::BASE_100MS,
                    0,
                    mapping::TIMER_MODE_ON,
                ));
                let mut element = PlacedElement::new(
                    ElementKind::Timer {
                        mode: mapping::timer_mode(mode),
                    },
                    column,
                    row,
                );
                element.var = Some(VarRef::new(VarKind::TimerIec, index));
                element.connected_with_top = cell.connected_with_top;
                element.params = vec![mapping::timer_preset_text(base, preset)];
                return Some(element);
            }
            mapping::ELE_COUNTER => {
                let Some(index) = u32::try_from(cell.var_num).ok() else {
                    self.warn(
                        "SL-W030",
                        format!(
                            "{where_}: counter instance {} is out of range; cell skipped",
                            cell.var_num
                        ),
                        Some(rung),
                    );
                    return None;
                };
                let preset = self.counters.get(&index).copied().unwrap_or(0);
                let mut element = PlacedElement::new(
                    ElementKind::Counter {
                        kind: IMPORTED_COUNTER_KIND,
                    },
                    column,
                    row,
                );
                element.var = Some(VarRef::new(VarKind::Counter, index));
                element.connected_with_top = cell.connected_with_top;
                element.params = vec![preset.to_string()];
                return Some(element);
            }
            mapping::ELE_REGISTER => {
                let Some(index) = u32::try_from(cell.var_num).ok() else {
                    self.warn(
                        "SL-W030",
                        format!(
                            "{where_}: register instance {} is out of range; cell skipped",
                            cell.var_num
                        ),
                        Some(rung),
                    );
                    return None;
                };
                let mode = self
                    .registers
                    .get(&index)
                    .copied()
                    .unwrap_or(REGISTER_MODE_UNDEF);
                if mode == REGISTER_MODE_UNDEF {
                    self.warn(
                        "SL-W030",
                        format!(
                            "{where_}: register {index} has no mode recorded; imported as FIFO"
                        ),
                        Some(rung),
                    );
                }
                let mut element = PlacedElement::new(
                    ElementKind::Register {
                        mode: mapping::register_mode(mode),
                    },
                    column,
                    row,
                );
                element.var = Some(VarRef::new(VarKind::Register, index));
                element.connected_with_top = cell.connected_with_top;
                element.params = vec![self.register_capacity.max(0).to_string()];
                return Some(element);
            }
            mapping::ELE_COMPAR => {
                let text = self.expression_text(cell.var_num, &where_, rung)?;
                let translated = expr_map::translate_expression(&text);
                self.report_expression(&translated.reason, &translated.notes, &text, &where_, rung);
                let mut element = PlacedElement::new(ElementKind::Compare, column, row);
                element.connected_with_top = cell.connected_with_top;
                element.params = vec![translated
                    .soft
                    .unwrap_or_else(|| translated.classic.clone())];
                return Some(element);
            }
            mapping::ELE_OUTPUT_OPERATE => {
                let text = self.expression_text(cell.var_num, &where_, rung)?;
                let translated = expr_map::translate_operate(&text);
                self.report_expression(&translated.reason, &translated.notes, &text, &where_, rung);
                let params = translated
                    .params
                    .unwrap_or_else(|| vec![translated.classic.clone()]);
                let mut element = PlacedElement::new(ElementKind::Operate, column, row);
                element.connected_with_top = cell.connected_with_top;
                element.params = params;
                return Some(element);
            }
            mapping::ELE_OUTPUT => ElementKind::CoilOut,
            mapping::ELE_OUTPUT_NOT => ElementKind::CoilOutNeg,
            mapping::ELE_OUTPUT_SET => ElementKind::CoilSet,
            mapping::ELE_OUTPUT_RESET => ElementKind::CoilReset,
            mapping::ELE_OUTPUT_JUMP => {
                let mut element = PlacedElement::new(ElementKind::CoilJump, column, row);
                element.connected_with_top = cell.connected_with_top;
                element.params = vec![cell.var_num.to_string()];
                return Some(element);
            }
            mapping::ELE_OUTPUT_CALL => {
                let mut element = PlacedElement::new(ElementKind::CoilCall, column, row);
                element.connected_with_top = cell.connected_with_top;
                element.params = vec![cell.var_num.to_string()];
                return Some(element);
            }
            other => {
                self.warn(
                    "SL-W030",
                    format!(
                        "{where_}: element type {other} has no SoftLadder equivalent; cell skipped"
                    ),
                    Some(rung),
                );
                return None;
            }
        };
        let var = self.decode_cell_var(cell, rung, &where_)?;
        let mut element = PlacedElement::with_var(kind, var, column, row);
        element.connected_with_top = cell.connected_with_top;
        Some(element)
    }

    /// Reads the timer instance of a deprecated timer/monostable block.
    fn block_var(&mut self, cell: &RawCell, rung: u32, where_: &str) -> Option<VarRef> {
        match u32::try_from(cell.var_num) {
            Ok(index) => Some(VarRef::new(VarKind::TimerIec, index)),
            Err(_) => {
                self.warn(
                    "SL-W030",
                    format!(
                        "{where_}: timer instance {} is out of range; cell skipped",
                        cell.var_num
                    ),
                    Some(rung),
                );
                None
            }
        }
    }

    /// Reports the translation of an expression: `SL-W031` when it could not be
    /// translated and `SL-W030` for every approximation.
    fn report_expression(
        &mut self,
        reason: &Option<String>,
        notes: &[String],
        text: &str,
        where_: &str,
        rung: u32,
    ) {
        if let Some(reason) = reason {
            self.warn(
                "SL-W031",
                format!(
                    "{where_}: expression `{text}` cannot be translated ({reason}); its \
                     ClassicLadder text was kept"
                ),
                Some(rung),
            );
        }
        for note in notes {
            self.warn("SL-W030", format!("{where_}: {note}"), Some(rung));
        }
    }

    /// Looks up the ClassicLadder expression a compare/operate block points at.
    fn expression_text(&mut self, index: i64, where_: &str, rung: u32) -> Option<String> {
        match self.expressions.get(&index) {
            Some(text) => Some(text.clone()),
            None => {
                self.fail(
                    format!(
                        "{where_}: expression index {index} points nowhere; the block was skipped"
                    ),
                    Some(rung),
                );
                None
            }
        }
    }

    /// Reads the variable of a cell that needs one.
    fn decode_cell_var(&mut self, cell: &RawCell, rung: u32, where_: &str) -> Option<VarRef> {
        let mut var = match decode_var(cell.var_type, cell.var_num) {
            DecodedVar::Plain(var) => var,
            DecodedVar::Deprecated(var) => {
                self.warn(
                    "SL-W031",
                    format!(
                        "{where_}: variable type {}/{} belongs to a deprecated ClassicLadder \
                         family; imported as `{var}`",
                        cell.var_type, cell.var_num
                    ),
                    Some(rung),
                );
                var
            }
            DecodedVar::Unsupported => {
                self.warn(
                    "SL-W031",
                    format!(
                        "{where_}: variable type {} has no SoftLadder equivalent",
                        cell.var_type
                    ),
                    Some(rung),
                );
                self.warn(
                    "SL-W030",
                    format!("{where_}: the element needs that variable and was skipped"),
                    Some(rung),
                );
                return None;
            }
        };
        if let Some((index_type, index_num)) = cell.indexed {
            let index = match decode_var(index_type, index_num) {
                DecodedVar::Plain(index) | DecodedVar::Deprecated(index) => index,
                DecodedVar::Unsupported => {
                    self.warn(
                        "SL-W031",
                        format!(
                            "{where_}: index variable type {index_type} has no SoftLadder equivalent"
                        ),
                        Some(rung),
                    );
                    self.warn(
                        "SL-W030",
                        format!("{where_}: the element needs that index and was skipped"),
                        Some(rung),
                    );
                    return None;
                }
            };
            if cell.var_num != 0 {
                self.warn(
                    "SL-W030",
                    format!(
                        "{where_}: indexed variable has base {}, which SoftLadder cannot express; \
                         the index is used on its own",
                        cell.var_num
                    ),
                    Some(rung),
                );
            }
            var = var.with_index_var(index);
        }
        Some(var)
    }

    /// Reads `sections.csv` and resolves each section's rung list.
    fn read_sections(&mut self) -> Vec<Section> {
        let Some(content) = self.part("sections.csv").map(str::to_owned) else {
            self.warn(
                "SL-W030",
                "sections.csv: missing, so every rung was placed in one main section".to_owned(),
                None,
            );
            let mut section = Section::new(0, "Prog1");
            section.rungs = self.links.keys().copied().collect();
            return vec![section];
        };
        let mut names: BTreeMap<u32, String> = BTreeMap::new();
        let mut raw: Vec<RawSection> = Vec::new();
        for (line, text) in numbered_lines(&content) {
            // `#NAME<n>=<name>` is data, not a comment, so it is read before the
            // generic comment skip.
            if let Some(rest) = text.strip_prefix("#NAME") {
                let digits = rest
                    .find(|character: char| !character.is_ascii_digit())
                    .unwrap_or(rest.len());
                if let Ok(index) = rest[..digits].parse::<u32>() {
                    let name = rest.get(digits + 1..).unwrap_or("");
                    names.insert(index, name.trim().to_owned());
                }
                continue;
            }
            if is_comment_line(text) {
                continue;
            }
            let fields = comma_numbers(text);
            let [index, language, subroutine, first, last, _page] = fields.as_slice() else {
                self.fail(
                    format!("sections.csv line {line}: cannot read `{text}`"),
                    None,
                );
                continue;
            };
            let Ok(index) = u32::try_from(*index) else {
                self.fail(
                    format!("sections.csv line {line}: section number {index} is out of range"),
                    None,
                );
                continue;
            };
            raw.push(RawSection {
                index,
                language: *language,
                subroutine: *subroutine,
                first_rung: *first,
                last_rung: *last,
                line,
            });
        }
        let mut sections = Vec::new();
        for entry in &raw {
            let name = names
                .get(&entry.index)
                .cloned()
                .unwrap_or_else(|| format!("Section{}", entry.index));
            let mut section = Section::new(entry.index, name);
            section.language = if entry.language == 1 {
                SectionLanguage::Sfc
            } else {
                SectionLanguage::Ladder
            };
            section.subroutine = u32::try_from(entry.subroutine).ok();
            if section.language == SectionLanguage::Ladder {
                section.rungs = self.resolve_rungs(entry);
            }
            sections.push(section);
        }
        if sections.is_empty() {
            // The reference adds its default section when a project defines
            // none, so a document with an empty `sections.csv` still runs.
            self.warn(
                "SL-W030",
                "sections.csv: no section is defined, so every rung was placed in one main section"
                    .to_owned(),
                None,
            );
            let mut section = Section::new(0, "Prog1");
            section.rungs = self.links.keys().copied().collect();
            sections.push(section);
        }
        sections
    }

    /// Walks `#PREVRUNG`/`#NEXTRUNG` from a section's first rung.
    fn resolve_rungs(&mut self, entry: &RawSection) -> Vec<u32> {
        if entry.first_rung >= 0 {
            let mut rungs = Vec::new();
            let mut seen = BTreeSet::new();
            let mut current = entry.first_rung;
            let mut broken = false;
            while current != -1 {
                let Ok(id) = u32::try_from(current) else {
                    broken = true;
                    break;
                };
                if !seen.insert(id) || !self.links.contains_key(&id) {
                    broken = true;
                    break;
                }
                rungs.push(id);
                current = self.links.get(&id).map(|(_, next)| *next).unwrap_or(-1);
            }
            if !broken {
                return rungs;
            }
        }
        self.warn(
            "SL-W030",
            format!(
                "sections.csv line {}: the rung chain of section {} is broken \
                 (#PREVRUNG/#NEXTRUNG); falling back to ascending rung order",
                entry.line, entry.index
            ),
            None,
        );
        if entry.first_rung >= 0 && entry.last_rung >= 0 {
            let low = entry.first_rung.min(entry.last_rung);
            let high = entry.first_rung.max(entry.last_rung);
            let rungs: Vec<u32> = self
                .links
                .keys()
                .copied()
                .filter(|id| i64::from(*id) >= low && i64::from(*id) <= high)
                .collect();
            if !rungs.is_empty() {
                return rungs;
            }
        }
        self.links.keys().copied().collect()
    }

    /// Rewrites jump coils from reference rung indices to section positions.
    fn fix_jumps(&mut self, rungs: &mut [RawRung], sections: &[Section]) {
        for raw in rungs.iter_mut() {
            for element in &mut raw.rung.elements {
                if element.kind != ElementKind::CoilJump {
                    continue;
                }
                let Some(target) = element.params.first().and_then(|text| parse_int(text)) else {
                    continue;
                };
                let owner = sections
                    .iter()
                    .find(|section| section.rungs.contains(&raw.id));
                let position = owner.and_then(|section| {
                    section.rungs.iter().position(|id| i64::from(*id) == target)
                });
                match position {
                    Some(position) => element.params = vec![position.to_string()],
                    None => self.warn(
                        "SL-W030",
                        format!(
                            "rung_{}.csv: jump target rung {target} is not in the section that \
                             holds rung {}; the flat index was kept",
                            raw.id, raw.id
                        ),
                        Some(raw.id),
                    ),
                }
            }
        }
    }

    /// Looks up a modelled part's contents.
    fn part(&self, name: &str) -> Option<&str> {
        self.parts
            .iter()
            .find(|(part, _)| part == name)
            .map(|(_, content)| content.as_str())
    }
}

/// Preset of a deprecated timer/monostable block, in milliseconds.
fn legacy_preset(table: &BTreeMap<u32, (i64, i64)>, var_num: i64) -> String {
    let entry = u32::try_from(var_num)
        .ok()
        .and_then(|index| table.get(&index).copied());
    match entry {
        Some((base, preset)) => mapping::timer_preset_millis(base, preset),
        None => "0".to_owned(),
    }
}

/// `true` for the parts the exporter regenerates from the project.
fn is_regenerated(name: &str) -> bool {
    rung_index(name).is_some()
        || matches!(
            name,
            "sections.csv"
                | "symbols.csv"
                | "timers_iec.csv"
                | "counters.csv"
                | "registers.csv"
                | "arithmetic_expressions.csv"
        )
}

/// `true` for the parts that are partly modelled and therefore merged rather
/// than reported as a plain passthrough.
fn is_mergeable(name: &str) -> bool {
    matches!(name, "general.txt" | "project_infos.txt")
}

/// The flat rung index of a `rung_<n>.csv` part name.
fn rung_index(name: &str) -> Option<u32> {
    name.strip_prefix("rung_")?
        .strip_suffix(".csv")?
        .parse::<u32>()
        .ok()
}

/// Iterates over a part's lines, stripping the CR a CRLF file leaves behind and
/// numbering the lines from one.
fn numbered_lines(content: &str) -> impl Iterator<Item = (usize, &str)> {
    // `fgets` never reports the phantom line a trailing newline would produce,
    // so a part that ends with `\n` yields no extra empty row.
    let body = content.strip_suffix('\n').unwrap_or(content);
    body.split('\n')
        .enumerate()
        .map(|(index, line)| (index + 1, line.strip_suffix('\r').unwrap_or(line)))
        .filter(move |(_, line)| !(content.is_empty() && line.is_empty()))
}

/// `true` for the comment and version lines the reference skips.
fn is_comment_line(text: &str) -> bool {
    text.is_empty() || text.starts_with('#') || text.starts_with(';')
}

/// Reads `KEY=VALUE`, returning the value when the line starts with `KEY=`.
fn value_of<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    line.strip_prefix(key)?.strip_prefix('=')
}

/// Strips the optional block-letter prefix of an instance CSV row.
fn strip_letter_prefix<'a>(text: &'a str, prefix: &str) -> &'a str {
    if let Some(rest) = text.strip_prefix(prefix) {
        if rest.starts_with(|character: char| character.is_ascii_digit()) {
            return rest;
        }
    }
    text
}

/// Parses a signed integer, tolerating surrounding spaces.
fn parse_int(text: &str) -> Option<i64> {
    text.trim().parse::<i64>().ok()
}

/// Splits a comma-separated line into numbers, rejecting anything unparsable.
fn comma_numbers(text: &str) -> Vec<i64> {
    text.split(',')
        .map(|field| field.trim().parse::<i64>())
        .collect::<Result<Vec<i64>, _>>()
        .unwrap_or_default()
}

/// Parses one `Type-ConnectedWithTop-VarType/VarNum[IndexedVarType/IndexedVarNum]`
/// cell.
fn parse_cell(text: &str) -> Result<RawCell, String> {
    let (element, rest) = text
        .split_once('-')
        .ok_or_else(|| format!("`{text}` has no element type"))?;
    let etype = parse_int(element).ok_or_else(|| format!("`{element}` is not an element type"))?;
    let (connected, rest) = rest
        .split_once('-')
        .ok_or_else(|| format!("`{text}` has no ConnectedWithTop flag"))?;
    let connected_with_top = decode_flag(connected)
        .ok_or_else(|| format!("`{connected}` is not a ConnectedWithTop flag"))?;
    let (head, indexed) = match rest.split_once('[') {
        None => (rest, None),
        Some((head, tail)) => {
            let tail = tail
                .strip_suffix(']')
                .ok_or_else(|| format!("`{text}` has an unterminated index"))?;
            let (index_type, index_num) = tail
                .split_once('/')
                .ok_or_else(|| format!("`{text}` has a malformed index"))?;
            let index_type = parse_int(index_type)
                .ok_or_else(|| format!("`{index_type}` is not an index variable type"))?;
            let index_num = parse_int(index_num)
                .ok_or_else(|| format!("`{index_num}` is not an index variable number"))?;
            (head, Some((index_type, index_num)))
        }
    };
    let (var_type, var_num) = head
        .split_once('/')
        .ok_or_else(|| format!("`{text}` has no VarType/VarNum pair"))?;
    Ok(RawCell {
        etype,
        connected_with_top,
        var_type: parse_int(var_type)
            .ok_or_else(|| format!("`{var_type}` is not a variable type"))?,
        var_num: parse_int(var_num)
            .ok_or_else(|| format!("`{var_num}` is not a variable number"))?,
        indexed,
    })
}

/// Decodes a `ConnectedWithTop` flag: any non-zero number is `true`.
fn decode_flag(text: &str) -> Option<bool> {
    parse_int(text).map(|value| value != 0)
}
