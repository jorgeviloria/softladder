//! The PLC tag table.
//!
//! Every vendor tool centres on tags: a name, a type, an address and a comment,
//! and the ladder shows the name. SoftLadder's model already stores exactly that
//! (a [`softladder_core::Symbol`] bound to a [`softladder_core::VarRef`]); this
//! document promotes it from a dialog to a first-class table. See `docs/UX.md` §6.

use egui::{RichText, Ui};
use softladder_core::{Symbol, VarKind, VarRef};

use crate::app::{EditorApp, SymbolDraft};
use crate::design::{mono, section_header, TypeScale, SPACE_1};
use crate::panels::table::{self, Column, Spec, Width};

/// The data-type name a variable kind maps to, the way a tag table shows it.
pub fn data_type(var: &VarRef) -> &'static str {
    match var.kind {
        VarKind::MemBit | VarKind::PhysIn | VarKind::PhysOut | VarKind::System | VarKind::Led => {
            "Bool"
        }
        VarKind::MemWord | VarKind::PhysInWord | VarKind::PhysOutWord => "Int",
        VarKind::TimerIec => match var.accessor {
            // `%TM0.V` (elapsed) and `%TM0.P` (preset) are words; `%TM0` and
            // `%TM0.Q` are the done bit, exactly as the engine reads them.
            Some(softladder_core::Accessor::Value | softladder_core::Accessor::Preset) => "Int",
            Some(_) => "Bool",
            None => "Timer",
        },
        VarKind::Counter => match var.accessor {
            Some(softladder_core::Accessor::Value | softladder_core::Accessor::Preset) => "Int",
            Some(_) => "Bool",
            None => "Counter",
        },
        VarKind::Register => "Register",
        VarKind::Step => "Step",
        VarKind::Pid => match var.accessor {
            // Setpoint, process value and control output are reals; `%PID0`
            // and `%PID0.Q` are the enabled bit, as the engine reads them.
            Some(
                softladder_core::Accessor::Value
                | softladder_core::Accessor::Preset
                | softladder_core::Accessor::Out,
            ) => "Real",
            Some(_) => "Bool",
            None => "PID",
        },
    }
}

/// How many places in the program reference `var`.
fn used_by(app: &EditorApp, var: &VarRef) -> usize {
    app.project()
        .sections
        .iter()
        .flat_map(|section| section.rungs.iter())
        .filter_map(|rung_id| app.project().rung(*rung_id))
        .flat_map(|rung| rung.elements.iter())
        .filter(|element| element.var.as_ref() == Some(var))
        .count()
}

/// Draws the tag table.
pub fn show(app: &mut EditorApp, ui: &mut Ui) {
    tokens_and_title(app, ui);
    let tokens = app.tokens;

    let mut add: Option<Symbol> = None;
    let symbols: Vec<Symbol> = app.project().symbols.clone();
    let prepared = prepare(app, &symbols);
    let selected = app.selected_tag;
    // The cell callback and the row menu both record actions, and they cannot
    // mutably borrow the same slot at once, so the slot sits in a `RefCell`.
    let actions = std::cell::RefCell::new(TagActions::default());

    add_bar(app, ui, &mut add);
    let columns = columns();
    table::show(
        ui,
        &tokens,
        Spec {
            id: "plc-tags",
            columns: &columns,
            rows: symbols.len(),
        },
        |index| {
            prepared
                .get(index)
                .map(|row| row.haystack.clone())
                .unwrap_or_default()
        },
        |index, column| {
            prepared
                .get(index)
                .and_then(|row| row.keys.get(column).cloned())
                .unwrap_or_default()
        },
        |index| (selected == Some(index)).then_some(tokens.accent_soft),
        |ui, index, column| {
            let Some(symbol) = symbols.get(index) else {
                return;
            };
            draw_cell(app, ui, &tokens, index, column, symbol, &actions);
        },
        |ui, index| row_menu(ui, symbols.get(index), index, &actions),
    );
    let TagActions {
        remove,
        rename,
        watch,
    } = actions.into_inner();

    ui.add_space(SPACE_1);
    ui.label(
        RichText::new(
            "Tags name the addresses; the ladder, the watch table and the bench show the name \
             and fall back to the address.",
        )
        .size(TypeScale::CAPTION)
        .color(tokens.text_dim),
    );

    if let Some(index) = remove {
        let mut symbols = app.project().symbols.clone();
        if index < symbols.len() {
            symbols.remove(index);
            let _ = app.editor.set_symbols(symbols);
            app.selected_tag = None;
        }
    }
    if let Some((index, updated)) = rename {
        let mut symbols = app.project().symbols.clone();
        if let Some(slot) = symbols.get_mut(index) {
            *slot = updated;
            if let Err(error) = app.editor.set_symbols(symbols) {
                app.note(&error.to_string());
            }
        }
    }
    if let Some(symbol) = add {
        let mut symbols = app.project().symbols.clone();
        symbols.push(symbol);
        let _ = app.editor.set_symbols(symbols);
        app.tag_draft = (String::new(), String::new(), String::new());
    }
    if let Some(var) = watch {
        if !app.watch.iter().any(|row| row.var == var) {
            app.watch.push(crate::panels::watch::row_for(var.clone()));
        }
        app.centre_tab = crate::app::CentreTab::Watch;
        app.note(&format!("watching {var}"));
    }
}

/// What the table asked for while it was drawing.
#[derive(Default)]
struct TagActions {
    /// A row to delete.
    remove: Option<usize>,
    /// A row whose name or comment changed.
    rename: Option<(usize, Symbol)>,
    /// A variable to open in the watch table.
    watch: Option<VarRef>,
}

/// One tag, reduced to the strings the filter and the sort need.
struct Prepared {
    /// Name, type, address and comment, joined.
    haystack: String,
    /// One sort key per column.
    keys: [String; 5],
}

/// Builds the filter and sort text before the table borrows the editor.
fn prepare(app: &EditorApp, symbols: &[Symbol]) -> Vec<Prepared> {
    symbols
        .iter()
        .map(|symbol| {
            let address = symbol
                .var
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_else(|| "—".to_owned());
            let kind = data_type_of(symbol);
            let used = symbol
                .var
                .as_ref()
                .map(|var| used_by(app, var))
                .unwrap_or(0);
            Prepared {
                haystack: format!("{} {kind} {address} {}", symbol.name, symbol.comment),
                keys: [
                    symbol.name.clone(),
                    kind.to_owned(),
                    address,
                    symbol.comment.clone(),
                    used.to_string(),
                ],
            }
        })
        .collect()
}

/// The bar above the table: name, address, comment and Add.
fn add_bar(app: &mut EditorApp, ui: &mut Ui, add: &mut Option<Symbol>) {
    let tokens = app.tokens;
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("＋")
                .size(TypeScale::BODY)
                .color(tokens.text_dim),
        );
        ui.add(
            egui::TextEdit::singleline(&mut app.tag_draft.0)
                .hint_text("name")
                .desired_width(120.0),
        );
        ui.add(
            egui::TextEdit::singleline(&mut app.tag_draft.1)
                .hint_text("%I0")
                .desired_width(90.0)
                .font(egui::TextStyle::Monospace),
        );
        ui.add(
            egui::TextEdit::singleline(&mut app.tag_draft.2)
                .hint_text("comment")
                .desired_width(160.0),
        );
        let can_add = !app.tag_draft.1.trim().is_empty();
        if ui
            .add_enabled(can_add, egui::Button::new("Add"))
            .on_hover_text("Add a tag for the typed address")
            .clicked()
        {
            match app.tag_draft.1.trim().parse::<VarRef>() {
                Ok(var) => {
                    let name = if app.tag_draft.0.trim().is_empty() {
                        format!("tag{}", app.project().symbols.len() + 1)
                    } else {
                        app.tag_draft.0.clone()
                    };
                    *add = Some(Symbol {
                        name,
                        var: Some(var),
                        comment: app.tag_draft.2.clone(),
                        unit: None,
                    });
                }
                Err(_) => app.note(&format!("`{}` is not an address", app.tag_draft.1.trim())),
            }
        }
    });
    ui.add_space(SPACE_1);
}

/// One cell of a tag row.
fn draw_cell(
    app: &mut EditorApp,
    ui: &mut Ui,
    tokens: &crate::design::Tokens,
    index: usize,
    column: usize,
    symbol: &Symbol,
    actions: &std::cell::RefCell<TagActions>,
) {
    let address = symbol
        .var
        .as_ref()
        .map(ToString::to_string)
        .unwrap_or_else(|| "—".to_owned());
    let selected = app.selected_tag == Some(index);
    match column {
        0 => {
            if let Some(var) = symbol.var.clone() {
                let drag_id = ui.make_persistent_id(("tag-drag", index));
                ui.dnd_drag_source(drag_id, crate::panels::watch::WatchDrag { var }, |ui| {
                    ui.label(
                        RichText::new("⠿")
                            .size(TypeScale::CAPTION)
                            .color(tokens.text_dim),
                    )
                    .on_hover_text("Drag onto Watch & force");
                });
            }
            let mut draft = SymbolDraft::from_symbol(symbol);
            let response = ui.add(
                egui::TextEdit::singleline(&mut draft.name)
                    .id_source(("tag-name", index))
                    .desired_width(ui.available_width())
                    .text_color(if selected { tokens.accent } else { tokens.text }),
            );
            if response.gained_focus() {
                app.selected_tag = Some(index);
            }
            if response.lost_focus() && draft.name != symbol.name {
                draft.var = address;
                if let Ok(updated) = draft.to_symbol() {
                    actions.borrow_mut().rename = Some((index, updated));
                }
            }
        }
        1 => {
            ui.label(mono(data_type_of(symbol)));
        }
        2 => {
            ui.label(mono(address));
        }
        3 => {
            let mut draft = SymbolDraft::from_symbol(symbol);
            let response = ui.add(
                egui::TextEdit::singleline(&mut draft.comment)
                    .id_source(("tag-comment", index))
                    .desired_width(ui.available_width()),
            );
            if response.changed() {
                draft.var = address;
                if let Ok(updated) = draft.to_symbol() {
                    actions.borrow_mut().rename = Some((index, updated));
                }
            }
        }
        _ => {
            let count = symbol
                .var
                .as_ref()
                .map(|var| used_by(app, var))
                .unwrap_or(0);
            let used = ui
                .add(
                    egui::Label::new(
                        RichText::new(count.to_string())
                            .size(TypeScale::CAPTION)
                            .color(if count == 0 {
                                tokens.warning
                            } else {
                                tokens.text_dim
                            }),
                    )
                    .sense(egui::Sense::click()),
                )
                .on_hover_text("Watch this tag");
            if used.clicked() {
                actions.borrow_mut().watch = symbol.var.clone();
            }
        }
    }
}

/// The row menu: watch the tag, or delete it.
fn row_menu(
    ui: &mut Ui,
    symbol: Option<&Symbol>,
    index: usize,
    actions: &std::cell::RefCell<TagActions>,
) {
    let bound = symbol.and_then(|symbol| symbol.var.clone());
    if ui
        .add_enabled(bound.is_some(), egui::Button::new("Watch"))
        .clicked()
    {
        actions.borrow_mut().watch = bound;
        ui.close_menu();
    }
    if ui.button("Delete").clicked() {
        actions.borrow_mut().remove = Some(index);
        ui.close_menu();
    }
}

/// The type name of a symbol's variable.
fn data_type_of(symbol: &Symbol) -> &'static str {
    symbol.var.as_ref().map_or("—", data_type)
}

/// The document title: the table name and how many tags there are.
fn tokens_and_title(app: &mut EditorApp, ui: &mut Ui) {
    let count = app.project().symbols.len();
    let unbound = app
        .project()
        .symbols
        .iter()
        .filter(|symbol| symbol.var.is_none())
        .count();
    let subtitle = if unbound == 0 {
        format!("{count} tag(s); every tag is bound to an address")
    } else {
        format!("{count} tag(s); {unbound} without an address")
    };
    section_header(ui, &app.tokens, "PLC tags");
    ui.label(
        RichText::new(subtitle)
            .size(TypeScale::CAPTION)
            .color(app.tokens.text_dim),
    );
    ui.add_space(SPACE_1);
}

/// The columns of the tag table.
fn columns() -> [Column; 5] {
    [
        Column {
            title: "Name",
            width: Width::Fixed(120.0),
        },
        Column {
            title: "Type",
            width: Width::Fixed(72.0),
        },
        Column {
            title: "Address",
            width: Width::Fixed(88.0),
        },
        Column {
            title: "Comment",
            width: Width::Flex {
                weight: 1.0,
                min: 80.0,
            },
        },
        Column {
            title: "Used by",
            width: Width::Fixed(64.0),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use softladder_core::{Accessor, Project, VarKind};

    fn var(text: &str) -> VarRef {
        text.parse().expect("variable parses")
    }

    #[test]
    fn data_types_follow_the_variable_kind() {
        assert_eq!(data_type(&var("%I0")), "Bool");
        assert_eq!(data_type(&var("%Q3")), "Bool");
        assert_eq!(data_type(&var("%M7")), "Bool");
        assert_eq!(data_type(&var("%MW0")), "Int");
        assert_eq!(data_type(&var("%QW1")), "Int");
        assert_eq!(data_type(&var("%IW2")), "Int");
        // `%TM0` with no accessor *is* the done bit, so a bare timer reads as a
        // bit; `%TM0.T` (a whole timer) is not addressable, which is why the
        // type follows the reference rather than the storage class.
        assert_eq!(data_type(&var("%TM0")), "Bool");
        assert_eq!(data_type(&var("%TM0.V")), "Int");
        assert_eq!(data_type(&var("%TM0.Q")), "Bool", "the done bit is a bit");
        assert_eq!(data_type(&var("%C1")), "Bool", "bare `%C1` is the done bit");
        assert_eq!(data_type(&var("%C1.V")), "Int");
        assert_eq!(data_type(&var("%C1.Q")), "Bool");
        assert_eq!(data_type(&var("%R0")), "Register");
        assert_eq!(data_type(&var("%X2")), "Step");
    }

    #[test]
    fn an_unbound_tag_has_no_type() {
        let symbol = Symbol {
            name: "spare".to_owned(),
            var: None,
            comment: String::new(),
            unit: None,
        };
        assert_eq!(data_type_of(&symbol), "—");
        let bound = Symbol {
            name: "start".to_owned(),
            var: Some(VarRef::new(VarKind::PhysIn, 0).with_accessor(Accessor::Done)),
            comment: String::new(),
            unit: None,
        };
        // `%I0.Q` is not a plain input; the type still comes from the kind.
        assert_eq!(data_type_of(&bound), "Bool");
    }

    #[test]
    fn a_tag_row_that_does_not_parse_is_refused_with_the_reason() {
        // The tag document keeps the same contract as the old Symbols tab: an
        // address that does not parse is reported and nothing is written.
        let mut app = crate::app::EditorApp::new(Project::new("tags"));
        app.symbols = vec![SymbolDraft {
            name: "start".to_owned(),
            var: "%I0".to_owned(),
            comment: "start button".to_owned(),
        }];
        app.apply_symbols();
        assert_eq!(app.project().symbols.len(), 1);
        assert_eq!(app.symbols_error, None);

        app.symbols.push(SymbolDraft {
            name: "oops".to_owned(),
            var: "%nonsense".to_owned(),
            comment: String::new(),
        });
        let history = app.editor.history_len();
        app.apply_symbols();
        assert!(app.symbols_error.is_some());
        assert_eq!(app.editor.history_len(), history, "nothing was written");
        assert_eq!(app.project().symbols.len(), 1);
    }

    #[test]
    fn the_table_counts_usage_of_a_variable() {
        use softladder_core::{ElementKind, PlacedElement, Project, Rung, Section};
        let mut project = Project::new("tags");
        let mut rung = Rung::new(1);
        rung.elements.push(PlacedElement::with_var(
            ElementKind::ContactNo,
            var("%I0"),
            0,
            0,
        ));
        rung.elements.push(PlacedElement::with_var(
            ElementKind::CoilOut,
            var("%Q0"),
            1,
            0,
        ));
        project.rungs.push(rung);
        let mut section = Section::new(1, "Main");
        section.rungs.push(1);
        project.sections.push(section);

        let app = crate::app::EditorApp::new(project);
        assert_eq!(used_by(&app, &var("%I0")), 1);
        assert_eq!(used_by(&app, &var("%Q0")), 1);
        assert_eq!(used_by(&app, &var("%MW123")), 0);
    }
}
