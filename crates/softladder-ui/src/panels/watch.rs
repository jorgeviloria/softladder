//! The Watch & force document: the commissioning table.
//!
//! `docs/UX.md` §7: **Address/Tag**, **Type**, **Value** (live, monospaced),
//! **Format**, **Modify value** and a per-row **Force** toggle, with a warning
//! while any force is active (the status bar shows it too). Monitoring runs while
//! the bench runs; the values update in place.
//!
//! Adding a row accepts either a variable spelling (`%I0`, `%MW10`) or the name
//! of a tag in the PLC tag table, and refuses anything it cannot resolve with the
//! reason next to the field — never a modal. [`parse_watch_target`] is that rule,
//! and it is unit-tested without a window.

use egui::{RichText, Stroke, Ui};
use softladder_core::{Project, Severity, Symbol, VarRef};

use crate::app::EditorApp;
use crate::design::{
    empty_state, mono, pill, section_header, Tokens, TypeScale, RADIUS_CONTROL, SPACE_1, SPACE_2,
};
use crate::panels::tags::data_type;
use crate::watch::{ValueFormat, WatchRow};

/// The typed target of the "+ add" row, resolved to a variable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchTarget {
    /// A variable the table can monitor.
    Resolved(VarRef),
    /// The text names a tag that has no address bound to it.
    Unbound(String),
    /// The text is neither a variable nor a known tag.
    Unknown(String),
}

impl WatchTarget {
    /// The variable, when the text resolved.
    pub fn var(&self) -> Option<&VarRef> {
        match self {
            WatchTarget::Resolved(var) => Some(var),
            _ => None,
        }
    }

    /// The message to show next to the field, when the text did not resolve.
    pub fn error(&self) -> Option<String> {
        match self {
            WatchTarget::Resolved(_) => None,
            WatchTarget::Unbound(name) => Some(format!(
                "`{name}` is a tag with no address; bind it in PLC tags"
            )),
            WatchTarget::Unknown(text) => Some(format!("`{text}` is not an address or a tag name")),
        }
    }
}

/// The tag whose name is exactly `name`, ignoring case.
fn symbol_named<'a>(project: &'a Project, name: &str) -> Option<&'a Symbol> {
    project
        .symbols
        .iter()
        .find(|symbol| symbol.name.eq_ignore_ascii_case(name))
}

/// Resolves typed text to a variable, by tag name first and by spelling second.
pub fn parse_watch_target(project: &Project, text: &str) -> WatchTarget {
    let text = text.trim();
    if text.is_empty() {
        return WatchTarget::Unknown(String::new());
    }
    if let Some(symbol) = symbol_named(project, text) {
        return match symbol.var.clone() {
            Some(var) => WatchTarget::Resolved(var),
            None => WatchTarget::Unbound(symbol.name.clone()),
        };
    }
    match text.parse::<VarRef>() {
        Ok(var) => WatchTarget::Resolved(var),
        Err(_) => WatchTarget::Unknown(text.to_owned()),
    }
}

/// The tag bound to `var`, if the project declares one.
fn symbol_for<'a>(project: &'a Project, var: &VarRef) -> Option<&'a Symbol> {
    project
        .symbols
        .iter()
        .find(|symbol| symbol.var.as_ref() == Some(var))
}

/// The format a row starts in: a bit column reads as `TRUE`/`FALSE`, and a word
/// column as a signed number, so a freshly added row shows something meaningful.
pub fn default_format(var: &VarRef) -> ValueFormat {
    if var.is_bit() {
        ValueFormat::Bool
    } else {
        ValueFormat::Signed
    }
}

/// A row monitoring `var`, in the format its type suggests.
pub fn row_for(var: VarRef) -> WatchRow {
    let mut row = WatchRow::new(var);
    row.format = default_format(&row.var);
    row
}

/// Adds `var` to `rows` unless it is already there; returns whether it was added.
pub fn push_row(rows: &mut Vec<WatchRow>, var: VarRef) -> bool {
    if rows.iter().any(|row| row.var == var) {
        return false;
    }
    rows.push(row_for(var));
    true
}

/// Whether `var` is currently forced, and to which value.
pub fn forced_value(app: &EditorApp, var: &VarRef) -> Option<bool> {
    app.forces
        .iter()
        .find(|(forced, _)| forced == var)
        .map(|(_, value)| *value)
}

/// Adds or removes a force on `var`.
pub fn toggle_force(app: &mut EditorApp, var: &VarRef, value: bool) {
    if let Some(slot) = app.forces.iter_mut().find(|(forced, _)| forced == var) {
        slot.1 = value;
    } else {
        app.forces.push((var.clone(), value));
    }
}

/// Removes the force on `var`, if there is one.
pub fn release_force(app: &mut EditorApp, var: &VarRef) {
    app.forces.retain(|(forced, _)| forced != var);
}

/// The diagnostics the watch document surfaces: the bench's own, plus the
/// simulation-panel warnings the editor already carries.
pub fn watch_problems(app: &EditorApp) -> Vec<softladder_core::Diagnostic> {
    let mut problems: Vec<softladder_core::Diagnostic> = app
        .editor()
        .problems()
        .iter()
        .filter(|diagnostic| diagnostic.code.starts_with("SL-W020"))
        .cloned()
        .collect();
    for diagnostic in app.bench().diagnostics() {
        if !problems.contains(diagnostic) {
            problems.push(diagnostic.clone());
        }
    }
    problems
}

/// What one frame of the table asked for.
#[derive(Default)]
struct WatchActions {
    /// A format picker changed.
    format: Option<(usize, ValueFormat)>,
    /// The modify text as it was typed this frame.
    modify_draft: Option<(usize, String)>,
    /// A modify value was applied.
    modify: Option<usize>,
    /// A force was switched on or off.
    force: Option<(usize, bool)>,
    /// A row was removed.
    remove: Option<usize>,
    /// The typed address or tag of the "+ add" row.
    add: bool,
    /// The add came from the tag picker, so its draft is a valid spelling.
    picked: bool,
}

// The "+ add" row's own state: the typed text and the reason it was refused.
//
// Both are view state, so they live here instead of growing `EditorApp`, and the
// reason survives the frame that produced it.
thread_local! {
    static DRAFT: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
    static ADD_ERROR: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// The text typed into the "+ add" row.
pub fn add_draft() -> String {
    DRAFT.with(|draft| draft.borrow().clone())
}

/// Replaces the text of the "+ add" row.
pub fn set_add_draft(text: impl Into<String>) {
    DRAFT.with(|draft| *draft.borrow_mut() = text.into());
}

/// The reason the last add was refused, if it was.
pub fn add_error() -> Option<String> {
    ADD_ERROR.with(|error| error.borrow().clone())
}

// Rows waiting to be added to the table.
//
// The document owns the table's state, so a caller outside the panel (the
// screenshot harness, or an automation script) asks for rows through `preload`
// instead of reaching into the editor.
thread_local! {
    static PENDING: std::cell::RefCell<Vec<WatchRow>> = const { std::cell::RefCell::new(Vec::new()) };
    static PENDING_FORCES: std::cell::RefCell<Vec<(VarRef, bool)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Queues `rows` and `forces` to be added the next time the document is drawn.
pub fn preload(rows: Vec<WatchRow>, forces: Vec<(VarRef, bool)>) {
    PENDING.with(|pending| pending.borrow_mut().extend(rows));
    PENDING_FORCES.with(|pending| pending.borrow_mut().extend(forces));
}

/// Moves the queued rows into the editor's table.
fn drain_pending(app: &mut EditorApp) {
    let rows = PENDING.with(|pending| std::mem::take(&mut *pending.borrow_mut()));
    for row in rows {
        let var = row.var.clone();
        if push_row(&mut app.watch, var) {
            if let Some(target) = app.watch.last_mut() {
                target.format = row.format;
            }
        }
    }
    let forces = PENDING_FORCES.with(|pending| std::mem::take(&mut *pending.borrow_mut()));
    for (var, value) in forces {
        if !app.forces.iter().any(|(forced, _)| forced == &var) {
            app.forces.push((var, value));
        }
    }
}

/// Draws the watch and force table.
pub fn show(app: &mut EditorApp, ui: &mut Ui) {
    drain_pending(app);
    let tokens = app.tokens;
    section_header(ui, &tokens, "Watch & force");
    let mut actions = WatchActions::default();

    row_counts(app, ui);
    if app.watch.is_empty() {
        empty_state(
            ui,
            &tokens,
            "Nothing is being watched",
            "Type an address such as %MW10 or a tag name into the field below, \
             or add the element under the cursor from the inspector.",
            "Values update while the bench runs; Modify writes a value and Force holds it.",
        );
    } else {
        table(app, ui, &mut actions);
    }
    add_row(app, ui, &mut actions);
    errors(app, ui);

    apply(app, actions);
}

/// The summary line: how many rows are watched, modified or forced.
fn row_counts(app: &EditorApp, ui: &mut Ui) {
    let tokens = app.tokens;
    let forced = app.forces.len();
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new(format!("{} row(s)", app.watch.len()))
                .size(TypeScale::CAPTION)
                .color(tokens.text_dim),
        );
        if forced > 0 {
            pill(ui, tokens.warning, &format!("{forced} forced"));
        } else {
            ui.label(
                RichText::new("no forces active")
                    .size(TypeScale::CAPTION)
                    .color(tokens.text_dim),
            );
        }
    });
    ui.add_space(SPACE_1);
}

/// One row of the table, read out of the editor so the panel never holds a
/// borrow of it while it draws (or while it mutates).
struct RowView {
    /// The tag name bound to the variable, when there is one.
    name: Option<String>,
    /// The variable spelling.
    address: String,
    /// The data type name.
    data_type: &'static str,
    /// The live value, already formatted.
    value: String,
    /// The format the value was formatted with.
    format: ValueFormat,
    /// The text in the modify column.
    modify: String,
    /// The forced value, while a force is active.
    force: Option<bool>,
    /// The variable the row watches.
    var: VarRef,
}

impl RowView {
    /// The label the row shows: the tag name, or the spelling.
    fn label(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.address)
    }
}

/// Reads the rows out of the editor and the live store.
fn views(app: &EditorApp) -> Vec<RowView> {
    let store = app.bench.engine().store();
    let project = app.project();
    app.watch
        .iter()
        .map(|row| RowView {
            name: symbol_for(project, &row.var).map(|symbol| symbol.name.clone()),
            address: row.var.to_string(),
            data_type: data_type(&row.var),
            value: row.format.display(store.get(&row.var)),
            format: row.format,
            modify: row.modify.clone(),
            force: forced_value(app, &row.var).or(row.force),
            var: row.var.clone(),
        })
        .collect()
}

/// The table itself.
fn table(app: &mut EditorApp, ui: &mut Ui, actions: &mut WatchActions) {
    let tokens = app.tokens;
    let live = app.bench.state().is_scanning();
    let rows = views(app);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .max_height((ui.available_height() - 80.0).max(120.0))
        .show(ui, |ui| {
            egui::Grid::new("watch_table")
                .num_columns(6)
                .spacing([SPACE_2, SPACE_1])
                .striped(true)
                .min_col_width(72.0)
                .show(ui, |ui| {
                    for title in [
                        "Address / Tag",
                        "Type",
                        "Value",
                        "Format",
                        "Modify",
                        "Force",
                    ] {
                        ui.label(
                            RichText::new(title)
                                .size(TypeScale::CAPTION)
                                .color(tokens.text_dim)
                                .strong(),
                        );
                    }
                    ui.end_row();

                    for (index, row) in rows.iter().enumerate() {
                        ui.vertical(|ui| {
                            ui.label(
                                RichText::new(row.label())
                                    .size(TypeScale::BODY)
                                    .color(tokens.text),
                            );
                            if row.name.is_some() {
                                ui.label(
                                    RichText::new(&row.address)
                                        .monospace()
                                        .size(TypeScale::CAPTION)
                                        .color(tokens.text_dim),
                                );
                            }
                        });
                        ui.label(
                            RichText::new(row.data_type)
                                .size(TypeScale::CAPTION)
                                .color(tokens.text_dim),
                        );
                        // The live value: monospaced, and in the live colour
                        // while the bench is actually scanning.
                        ui.label(mono(row.value.clone()).color(if live {
                            tokens.energised
                        } else {
                            tokens.text
                        }));
                        format_picker(ui, index, row.format, actions);
                        modify_field(ui, index, &row.modify, actions);
                        force_controls(ui, &tokens, index, row, actions);
                        ui.end_row();
                    }
                });
        });
}

/// The format picker of one row.
fn format_picker(ui: &mut Ui, index: usize, current: ValueFormat, actions: &mut WatchActions) {
    let mut format = current;
    egui::ComboBox::from_id_salt(("watch-format", index))
        .selected_text(format.label())
        .width(74.0)
        .show_ui(ui, |ui| {
            for option in ValueFormat::ALL {
                if ui
                    .selectable_value(&mut format, option, option.label())
                    .clicked()
                {
                    actions.format = Some((index, option));
                }
            }
        });
}

/// The modify field of one row, with its apply button.
fn modify_field(ui: &mut Ui, index: usize, current: &str, actions: &mut WatchActions) {
    let mut modify = current.to_owned();
    ui.horizontal(|ui| {
        let response = ui.add(
            egui::TextEdit::singleline(&mut modify)
                .desired_width(84.0)
                .hint_text("value")
                .font(egui::TextStyle::Monospace),
        );
        if response.changed() {
            actions.modify_draft = Some((index, modify));
        }
        let enter = response.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
        if ui
            .small_button("↵")
            .on_hover_text("Write the value into the scan store")
            .clicked()
            || enter
        {
            actions.modify = Some(index);
        }
    });
}

/// The per-row force toggle and its remove button.
fn force_controls(
    ui: &mut Ui,
    tokens: &Tokens,
    index: usize,
    row: &RowView,
    actions: &mut WatchActions,
) {
    ui.horizontal(|ui| {
        let mut armed = row.force.is_some();
        let caption = if armed { "FORCED" } else { "force" };
        if ui
            .toggle_value(&mut armed, caption)
            .on_hover_text("Hold this value while the bench runs (click again to release)")
            .changed()
        {
            actions.force = Some((index, armed));
        }
        if let Some(value) = row.force {
            ui.label(
                RichText::new(if value { "= 1" } else { "= 0" })
                    .monospace()
                    .size(TypeScale::CAPTION)
                    .color(tokens.warning),
            );
        }
        if ui
            .small_button("✕")
            .on_hover_text("Remove this row")
            .clicked()
        {
            actions.remove = Some(index);
        }
        let _ = &row.var;
    });
}

/// The "+ add" row, which accepts a typed address or tag name.
fn add_row(app: &mut EditorApp, ui: &mut Ui, actions: &mut WatchActions) {
    let tokens = app.tokens;
    ui.add_space(SPACE_1);
    let mut draft = add_draft();
    let mut pick: Option<String> = None;
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("＋")
                .size(TypeScale::BODY)
                .color(tokens.text_dim),
        );
        let response = ui.add(
            egui::TextEdit::singleline(&mut draft)
                .hint_text("%MW10 or a tag name")
                .desired_width(180.0)
                .font(egui::TextStyle::Monospace),
        );
        let enter = response.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
        if ui.button("Add").clicked() || enter {
            actions.add = true;
        }
        // A tag picker, for the operator who does not remember the spelling.
        ui.menu_button("Tags…", |ui| {
            let mut any = false;
            for symbol in &app.project().symbols {
                let Some(var) = symbol.var.clone() else {
                    continue;
                };
                any = true;
                if ui.button(format!("{}   {var}", symbol.name)).clicked() {
                    pick = Some(var.to_string());
                    ui.close_menu();
                }
            }
            if !any {
                ui.label(RichText::new("No bound tags in this project").weak());
            }
        });
    });
    if let Some(var) = pick {
        draft = var;
        actions.add = true;
        actions.picked = true;
    }
    set_add_draft(draft);
}

/// The warning banner and the reason a typed value was refused.
fn errors(app: &mut EditorApp, ui: &mut Ui) {
    let tokens = app.tokens;
    if !app.forces.is_empty() {
        egui::Frame::new()
            .fill(tokens.warning.gamma_multiply(0.12))
            .stroke(Stroke::new(1.0_f32, tokens.warning))
            .corner_radius(egui::CornerRadius::same(RADIUS_CONTROL))
            .inner_margin(egui::Margin::symmetric(SPACE_2 as i8, SPACE_1 as i8))
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new("WARNING")
                            .size(TypeScale::CAPTION)
                            .color(tokens.warning)
                            .strong(),
                    );
                    ui.label(
                        RichText::new(format!(
                            "{} value(s) are forced and do not follow the program. \
                             Release a force with its row toggle.",
                            app.forces.len()
                        ))
                        .size(TypeScale::CAPTION)
                        .color(tokens.text),
                    );
                });
            });
    }
    if let Some(message) = add_error() {
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new(message)
                    .size(TypeScale::CAPTION)
                    .color(tokens.error),
            );
        });
    }
    let problems = watch_problems(app);
    for diagnostic in problems.iter().take(4) {
        let colour = match diagnostic.severity {
            Severity::Error => tokens.error,
            Severity::Warning => tokens.warning,
            Severity::Info => tokens.text_dim,
        };
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new(diagnostic.code)
                    .monospace()
                    .size(TypeScale::CAPTION)
                    .color(colour)
                    .strong(),
            );
            ui.label(
                RichText::new(&diagnostic.message)
                    .size(TypeScale::CAPTION)
                    .color(tokens.text_dim),
            );
        });
    }
}

/// Applies everything the table asked for.
fn apply(app: &mut EditorApp, actions: WatchActions) {
    if let Some((index, format)) = actions.format {
        if let Some(row) = app.watch.get_mut(index) {
            row.format = format;
        }
    }
    if let Some((index, text)) = actions.modify_draft {
        if let Some(row) = app.watch.get_mut(index) {
            row.modify = text;
        }
    }
    if let Some(index) = actions.remove {
        if index < app.watch.len() {
            let row = app.watch.remove(index);
            release_force(app, &row.var);
        }
    }
    if let Some(index) = actions.modify {
        write_modify(app, index);
    }
    if let Some((index, armed)) = actions.force {
        force(app, index, armed);
    }
    if actions.add {
        add(app);
    }
}

/// Writes the typed modify value into the scan store, refusing nonsense.
fn write_modify(app: &mut EditorApp, index: usize) {
    let Some(row) = app.watch.get(index).cloned() else {
        return;
    };
    match row.format.parse(&row.modify) {
        Ok(value) => {
            let var = row.var.clone();
            match app.bench.engine_mut().store_mut().set(&var, value) {
                Ok(()) => app.note(&format!("{var} := {}", row.format.display(Some(value)))),
                Err(error) => app.note(&format!("{var} cannot be written: {error}")),
            }
        }
        Err(error) => app.note(&error),
    }
}

/// Turns a row's force on or off.
fn force(app: &mut EditorApp, index: usize, armed: bool) {
    let Some(row) = app.watch.get(index).cloned() else {
        return;
    };
    if armed {
        // Force the value the operator typed, or the value the store holds.
        let value = row
            .format
            .parse(&row.modify)
            .map(|value| value.as_bool())
            .unwrap_or_else(|_| {
                app.bench
                    .engine()
                    .store()
                    .get(&row.var)
                    .map(|value| value.as_bool())
                    .unwrap_or(false)
            });
        toggle_force(app, &row.var, value);
        if let Some(target) = app.watch.get_mut(index) {
            target.force = Some(value);
        }
        let var = row.var.clone();
        app.note(&format!(
            "{var} forced {}",
            if value { "TRUE" } else { "FALSE" }
        ));
    } else {
        release_force(app, &row.var);
        if let Some(target) = app.watch.get_mut(index) {
            target.force = None;
        }
        let var = row.var.clone();
        app.note(&format!("{var} released"));
    }
}

/// Adds the typed address or tag to the table.
fn add(app: &mut EditorApp) {
    let draft = add_draft();
    let target = parse_watch_target(app.project(), &draft);
    match target {
        WatchTarget::Resolved(var) => {
            if push_row(&mut app.watch, var.clone()) {
                app.note(&format!("watching {var}"));
            } else {
                app.note(&format!("{var} is already in the watch table"));
            }
            set_add_draft(String::new());
            ADD_ERROR.with(|error| *error.borrow_mut() = None);
        }
        other => {
            let message = other
                .error()
                .unwrap_or_else(|| "that is not a variable".to_owned());
            app.note(&message);
            ADD_ERROR.with(|error| *error.borrow_mut() = Some(message));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The window the headless frames are laid out in.
    const TEST_SIZE: egui::Vec2 = egui::vec2(1440.0, 900.0);

    /// A context for the headless frames, shared by the tests of this module.
    fn ctx() -> egui::Context {
        egui::Context::default()
    }
    use softladder_core::{ElementKind, PlacedElement, Project, Rung, Section, Symbol};

    fn var(text: &str) -> VarRef {
        text.parse().expect("variable parses")
    }

    fn project() -> Project {
        let mut project = Project::new("watch");
        let mut main = Section::new(1, "Main");
        main.rungs.push(1);
        project.sections.push(main);
        let mut rung = Rung::new(1);
        rung.elements.push(PlacedElement::with_var(
            ElementKind::ContactNo,
            var("%I0"),
            0,
            0,
        ));
        project.rungs.push(rung);
        project.symbols.push(Symbol {
            name: "start_button".to_owned(),
            var: Some(var("%I0")),
            comment: "start".to_owned(),
            unit: None,
        });
        project.symbols.push(Symbol {
            name: "spare".to_owned(),
            var: None,
            comment: String::new(),
            unit: None,
        });
        project
    }

    #[test]
    fn a_typed_address_or_tag_name_resolves_to_a_variable() {
        let project = project();
        assert_eq!(
            parse_watch_target(&project, "%MW10"),
            WatchTarget::Resolved(var("%MW10"))
        );
        assert_eq!(
            parse_watch_target(&project, "  %I0  "),
            WatchTarget::Resolved(var("%I0"))
        );
        // A tag name wins over the address spelling, and is case-insensitive.
        assert_eq!(
            parse_watch_target(&project, "start_button"),
            WatchTarget::Resolved(var("%I0"))
        );
        assert_eq!(
            parse_watch_target(&project, "START_BUTTON"),
            WatchTarget::Resolved(var("%I0"))
        );
    }

    #[test]
    fn an_unresolvable_typing_is_refused_with_a_reason() {
        let project = project();
        assert_eq!(
            parse_watch_target(&project, "spare"),
            WatchTarget::Unbound("spare".to_owned())
        );
        let unbound = parse_watch_target(&project, "spare");
        assert!(unbound.var().is_none());
        assert!(unbound
            .error()
            .is_some_and(|message| message.contains("no address")));

        let unknown = parse_watch_target(&project, "%nonsense");
        assert!(unknown.var().is_none());
        assert!(unknown
            .error()
            .is_some_and(|message| message.contains("not an address")));

        assert!(parse_watch_target(&project, "").var().is_none());
        assert!(parse_watch_target(&project, "   ").var().is_none());
    }

    #[test]
    fn a_row_starts_in_a_format_that_fits_its_type() {
        assert_eq!(default_format(&var("%I0")), ValueFormat::Bool);
        assert_eq!(default_format(&var("%Q3")), ValueFormat::Bool);
        assert_eq!(default_format(&var("%MW10")), ValueFormat::Signed);
        assert_eq!(default_format(&var("%TM0.V")), ValueFormat::Signed);
        assert_eq!(row_for(var("%MW10")).format, ValueFormat::Signed);
        assert_eq!(row_for(var("%I0")).format, ValueFormat::Bool);
    }

    #[test]
    fn a_row_is_added_once_and_never_twice() {
        let mut rows: Vec<WatchRow> = Vec::new();
        assert!(push_row(&mut rows, var("%I0")));
        assert!(!push_row(&mut rows, var("%I0")));
        assert!(push_row(&mut rows, var("%I1")));
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].var, var("%I0"));
    }

    #[test]
    fn forcing_records_the_value_and_releasing_removes_it() {
        let mut app = EditorApp::new(project());
        toggle_force(&mut app, &var("%Q0"), true);
        assert_eq!(forced_value(&app, &var("%Q0")), Some(true));
        assert_eq!(app.forces.len(), 1);
        // Forcing the same variable again replaces the value.
        toggle_force(&mut app, &var("%Q0"), false);
        assert_eq!(forced_value(&app, &var("%Q0")), Some(false));
        assert_eq!(app.forces.len(), 1);
        release_force(&mut app, &var("%Q0"));
        assert_eq!(forced_value(&app, &var("%Q0")), None);
        assert!(app.forces.is_empty());
        // Releasing something that is not forced is a no-op.
        release_force(&mut app, &var("%Q7"));
        assert!(app.forces.is_empty());
    }

    #[test]
    fn writing_a_modify_value_goes_through_the_store() {
        let mut app = EditorApp::new(project());
        app.watch.push(WatchRow {
            var: var("%MW10"),
            format: ValueFormat::Signed,
            modify: "42".to_owned(),
            force: None,
        });
        write_modify(&mut app, 0);
        assert_eq!(
            app.bench.engine().store().get(&var("%MW10")),
            Some(softladder_core::Value::Word(42))
        );
        assert!(app.status().contains("42"));

        // Nonsense is refused with the reason, and changes nothing.
        app.watch[0].modify = "not a number".to_owned();
        write_modify(&mut app, 0);
        assert!(app.status().contains("not a signed integer"));
        // A stale index is a no-op, not a panic.
        write_modify(&mut app, 99);
    }

    #[test]
    fn adding_a_row_reports_what_happened() {
        let mut app = EditorApp::new(project());
        set_add_draft("%I2");
        add(&mut app);
        assert_eq!(app.watch.len(), 1);
        assert!(app.status().contains("%I2"));
        // The draft is cleared on success.
        assert!(add_draft().is_empty());

        // A tag with no address is refused, and the reason is kept for the UI.
        set_add_draft("spare");
        add(&mut app);
        assert_eq!(app.watch.len(), 1);
        assert!(add_error().is_some_and(|message| message.contains("no address")));

        // An unparseable spelling is refused too.
        set_add_draft("%?");
        add(&mut app);
        assert_eq!(app.watch.len(), 1);
        assert!(add_error().is_some_and(|message| message.contains("not an address")));

        // And so is a duplicate.
        set_add_draft("%I2");
        add(&mut app);
        assert_eq!(app.watch.len(), 1);
        assert!(app.status().contains("already"));
        set_add_draft("");
    }

    #[test]
    fn the_watch_document_reports_the_simulation_warnings() {
        let mut app = EditorApp::new(project());
        // A panel that addresses a variable of the wrong width is an SL-W020.
        let mut panel = app.project().simulation.clone();
        panel.switches.push(softladder_core::SimSwitch {
            var: var("%Q3"),
            label: "wrong width".to_owned(),
            momentary: false,
        });
        update_panel_through_editor(&mut app, panel);
        let problems = watch_problems(&app);
        assert!(
            problems
                .iter()
                .any(|diagnostic| diagnostic.code == "SL-W020"),
            "the simulation warning is surfaced: {problems:?}"
        );
        assert!(problems
            .iter()
            .all(|diagnostic| diagnostic.code.starts_with("SL-W0")));
    }

    /// Replaces the panel without reaching into the editor directly.
    fn update_panel_through_editor(app: &mut EditorApp, panel: softladder_core::SimulationPanel) {
        let _ = app.editor.set_panel(panel);
    }

    #[test]
    fn preloaded_rows_and_forces_reach_the_table() {
        let ctx = egui::Context::default();
        let mut app = EditorApp::new(project());
        app.centre_tab = crate::app::CentreTab::Watch;
        let mut row = WatchRow::new(var("%MW10"));
        row.format = ValueFormat::Signed;
        preload(vec![row], vec![(var("%MW10"), true)]);
        crate::panels::test_frame(&ctx, &mut app, TEST_SIZE);
        assert_eq!(app.watch.len(), 1);
        assert_eq!(app.watch[0].format, ValueFormat::Signed);
        assert_eq!(forced_value(&app, &var("%MW10")), Some(true));
        // A second frame does not add the row again, and a repeat of the same
        // force does not duplicate it either.
        crate::panels::test_frame(&ctx, &mut app, TEST_SIZE);
        assert_eq!(app.watch.len(), 1);
        assert_eq!(app.forces.len(), 1);
    }

    #[test]
    fn drawing_the_watch_table_is_panic_free() {
        let mut app = EditorApp::new(project());
        app.centre_tab = crate::app::CentreTab::Watch;
        crate::panels::test_frame(&ctx(), &mut app, TEST_SIZE);
        app.watch.push(WatchRow::new(var("%MW10")));
        app.forces.push((var("%MW10"), true));
        crate::panels::test_frame(&ctx(), &mut app, TEST_SIZE);
        let mut empty = EditorApp::new(Project::new("empty"));
        empty.centre_tab = crate::app::CentreTab::Watch;
        crate::panels::test_frame(&ctx(), &mut empty, TEST_SIZE);
    }
}
