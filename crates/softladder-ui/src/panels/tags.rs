//! The PLC tag table.
//!
//! Every vendor tool centres on tags: a name, a type, an address and a comment,
//! and the ladder shows the name. SoftLadder's model already stores exactly that
//! (a [`softladder_core::Symbol`] bound to a [`softladder_core::VarRef`]); this
//! document promotes it from a dialog to a first-class table. See `docs/UX.md` §6.

use egui::{Align, Layout, RichText, Ui};
use softladder_core::{Symbol, VarKind, VarRef};

use crate::app::{EditorApp, SymbolDraft};
use crate::design::{mono, section_header, TypeScale, SPACE_1, SPACE_2};

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

    // A header row, then one row per tag: the table look every PLC tool has.
    let mut add: Option<Symbol> = None;
    let mut remove: Option<usize> = None;
    let mut rename: Option<(usize, Symbol)> = None;
    let mut watch: Option<VarRef> = None;

    egui::Grid::new("plc_tags")
        .num_columns(5)
        .spacing([SPACE_2, SPACE_1])
        .striped(true)
        .show(ui, |ui| {
            for title in ["Name", "Type", "Address", "Comment", "Used by"] {
                ui.label(
                    RichText::new(title)
                        .size(TypeScale::CAPTION)
                        .color(tokens.text_dim)
                        .strong(),
                );
            }
            ui.end_row();

            let symbols: Vec<Symbol> = app.project().symbols.clone();
            for (index, symbol) in symbols.iter().enumerate() {
                let address = symbol
                    .var
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_else(|| "—".to_owned());
                let selected = app.selected_tag == Some(index);
                let mut remove_row = false;
                let mut row_draft = SymbolDraft::from_symbol(symbol);

                let name_response = ui.add(
                    egui::TextEdit::singleline(&mut row_draft.name)
                        .desired_width(110.0)
                        .text_color(if selected { tokens.accent } else { tokens.text }),
                );
                if name_response.gained_focus() {
                    app.selected_tag = Some(index);
                }
                ui.label(mono(data_type_of(symbol)));
                ui.label(mono(address.clone()));
                let comment = ui.add(
                    egui::TextEdit::singleline(&mut row_draft.comment).desired_width(f32::INFINITY),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
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
                        .on_hover_text("Watch this tag in the Watch & force document");
                    if used.clicked() && symbol.var.is_some() {
                        watch = symbol.var.clone();
                    }
                    if ui
                        .small_button("✕")
                        .on_hover_text("Delete this tag")
                        .clicked()
                    {
                        remove_row = true;
                    }
                });
                ui.end_row();

                if comment.changed() {
                    row_draft.var = address.clone();
                    if let Ok(updated) = row_draft.to_symbol() {
                        rename = Some((index, updated));
                    }
                }
                // A renamed tag is applied through the editor's symbol table,
                // which validates every row before it writes anything.
                if name_response.lost_focus() && row_draft.name != symbol.name {
                    row_draft.var = address.clone();
                    if let Ok(updated) = row_draft.to_symbol() {
                        rename = Some((index, updated));
                    }
                }
                if remove_row {
                    remove = Some(index);
                }
            }

            // The "add a tag" row.
            ui.label("＋");
            ui.label(
                RichText::new("Bool")
                    .size(TypeScale::CAPTION)
                    .color(tokens.text_dim),
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
                    .desired_width(f32::INFINITY),
            );
            let can_add = !app.tag_draft.1.trim().is_empty();
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .add_enabled(can_add, egui::Button::new("Add"))
                    .on_hover_text("Add a tag for the typed address")
                    .clicked()
                {
                    let name = app.tag_draft.0.clone();
                    let var = app.tag_draft.1.clone();
                    let comment = app.tag_draft.2.clone();
                    if let Ok(var) = var.trim().parse::<VarRef>() {
                        let name = if name.trim().is_empty() {
                            format!("tag{}", app.project().symbols.len() + 1)
                        } else {
                            name
                        };
                        add = Some(Symbol {
                            name,
                            var: Some(var),
                            comment,
                            unit: None,
                        });
                    }
                }
                ui.add(
                    egui::TextEdit::singleline(&mut app.tag_draft.0)
                        .hint_text("name")
                        .desired_width(120.0),
                );
            });
            ui.end_row();
        });

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
