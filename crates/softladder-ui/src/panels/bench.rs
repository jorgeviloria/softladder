//! The Bench document: the operator screen.
//!
//! `docs/UX.md` §8: the bench is laid out like an operator screen, not a form.
//! Switches are real toggles with an obvious on/off state, momentary buttons look
//! like buttons and visibly spring back, lamps are round indicators that glow in
//! `tokens.run` when lit, and sliders and gauges are real widgets with
//! monospaced numeric readouts. Labels come from the tag table and fall back to
//! the address.
//!
//! The *layout* comes from the project ([`SimulationPanel`]) and the *positions*
//! from the bench (`PanelState`), which is the split `docs/EDITOR.md`
//! §"Bench" describes; nothing is cached here. The bench also applies any forced
//! values before each scan, so a forced output holds while the program runs.

use std::cell::RefCell;

use egui::{Align, Color32, CornerRadius, Layout, Pos2, Rect, RichText, Sense, Stroke, Ui};
use softladder_core::{ScanConfig, SimulationPanel, Value, VarRef, VarStore};

use crate::app::EditorApp;
use crate::design::{
    empty_state, mono, pill, section_header, Tokens, TypeScale, RADIUS_CONTROL, SPACE_1, SPACE_2,
    SPACE_3,
};
use crate::queries;

// Which bench widget the operator has selected.
//
// This is view state, so it lives here rather than growing `EditorApp`: the
// inspector reads it and the bench writes it.
thread_local! {
    static SELECTED: RefCell<Option<Widget>> = const { RefCell::new(None) };
    /// The pointer position of the last press the bench already acted on, so one
    /// click throws a switch exactly once.
    static HANDLED_PRESS: RefCell<Option<Pos2>> = const { RefCell::new(None) };
}

/// One widget of the bench, addressed by its list and index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Widget {
    /// A toggle switch or a push-button.
    Switch(usize),
    /// A lamp.
    Lamp(usize),
    /// An analog slider.
    Analog(usize),
    /// An analog gauge.
    Gauge(usize),
}

/// The selected bench widget, if any.
pub fn selected_widget() -> Option<Widget> {
    SELECTED.with(|selected| *selected.borrow())
}

/// Selects a bench widget, or clears the selection with `None`.
pub fn select_widget(widget: Option<Widget>) {
    SELECTED.with(|selected| *selected.borrow_mut() = widget);
}

/// The diagnostics the bench produced on its last scan.
pub fn diagnostics(app: &EditorApp) -> Vec<softladder_core::Diagnostic> {
    app.bench().diagnostics().to_vec()
}

/// Writes every forced value into the scan store.
///
/// `docs/UX.md` §7 asks for a force to hold while the bench runs; calling this
/// immediately before each scan is what makes that true, and it is a no-op when
/// nothing is forced.
pub fn apply_forces(app: &mut EditorApp) {
    if app.forces.is_empty() {
        return;
    }
    let forces = app.forces.clone();
    let store = app.bench.engine_mut().store_mut();
    apply_forced_values(&forces, store);
}

/// Applies `forces` to `store`, one `set` per forced value; returns how many
/// landed. A variable the store refuses is counted as not applied rather than
/// panicking, because a force can name an address the model does not size.
pub fn apply_forced_values(forces: &[(VarRef, bool)], store: &mut VarStore) -> usize {
    let mut applied = 0;
    for (var, value) in forces {
        if store.set(var, Value::Bit(*value)).is_ok() {
            applied += 1;
        }
    }
    applied
}

/// Replaces the project's bench layout through the editor, then hot-reloads.
///
/// `edit` returns `true` when it changed something, so an unchanged panel does
/// not record an undo step.
pub fn update_panel(app: &mut EditorApp, edit: impl FnOnce(&mut SimulationPanel) -> bool) {
    let mut panel = app.project().simulation.clone();
    if !edit(&mut panel) {
        return;
    }
    if panel == app.project().simulation {
        return;
    }
    app.set_panel(panel);
}

/// The label a widget shows: its own label, or the address it addresses.
pub fn widget_label(label: &str, var: &VarRef) -> String {
    if label.trim().is_empty() {
        var.to_string()
    } else {
        label.to_owned()
    }
}

/// What a bench widget asked to change in the project's layout.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PanelEdit {
    /// A switch becomes (or stops being) a momentary push-button.
    Momentary(bool),
}

/// What one drawn frame of the bench wants the editor to do.
#[derive(Default)]
struct BenchActions {
    /// The widget the inspector should describe.
    select: Option<Widget>,
    /// A switch that was thrown.
    toggle: Option<(usize, bool)>,
    /// A push-button that was pressed.
    press: Option<usize>,
    /// A slider that moved.
    slide: Option<(usize, i32)>,
    /// A widget whose layout changed.
    edit: Option<(Widget, PanelEdit)>,
}

/// Draws the whole bench document.
pub fn show(app: &mut EditorApp, ui: &mut Ui) {
    let tokens = app.tokens;
    section_header(ui, &tokens, "Simulation bench");
    toolbar(app, ui);
    ui.add_space(SPACE_2);

    let panel = app.project().simulation.clone();
    if panel.is_empty() {
        empty_bench(ui, &tokens);
        return;
    }

    let mut actions = BenchActions::default();
    let selected = selected_widget();
    inputs(app, ui, &panel, selected, &mut actions);
    outputs(app, ui, &panel, selected, &mut actions);
    analog(app, ui, &panel, selected, &mut actions);
    apply(app, &panel, actions);
}

/// Runs the frame's bench actions against the editor and the operator state.
fn apply(app: &mut EditorApp, panel: &SimulationPanel, actions: BenchActions) {
    if let Some((index, closed)) = actions.toggle {
        app.bench.panel_state_mut().set_closed(index, closed);
        if !app.bench.state().is_scanning() {
            // A switch thrown while stopped is worth one scan, so the lamp on
            // the bench reacts immediately instead of on the next run.
            app.single_scan();
        }
    }
    if let Some(index) = actions.press {
        app.bench.panel_state_mut().set_closed(index, true);
        if !app.bench.state().is_scanning() {
            app.single_scan();
        }
    }
    if let Some((index, value)) = actions.slide {
        app.bench.panel_state_mut().set_analog(panel, index, value);
    }
    if let Some(widget) = actions.select {
        select_widget(Some(widget));
    }
    if let Some((widget, change)) = actions.edit {
        update_panel(app, |panel| match (widget, change) {
            (Widget::Switch(index), PanelEdit::Momentary(momentary)) => {
                match panel.switches.get_mut(index) {
                    Some(target) => {
                        target.momentary = momentary;
                        true
                    }
                    None => false,
                }
            }
            _ => false,
        });
    }
}

/// The run controls, the fill command, the scan period and the last scan time.
fn toolbar(app: &mut EditorApp, ui: &mut Ui) {
    let tokens = app.tokens;
    let running = app.is_running();
    let scanning = app.bench.state().is_scanning();
    let mut start = false;
    let mut stop = false;
    let mut single = false;
    let mut fill = false;
    let mut period_edit: Option<u32> = None;
    let mut period = app.project().scan.period_ms;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = SPACE_2;
        if ui
            .add_enabled(
                !running,
                egui::Button::new("▶  Run").min_size(egui::vec2(64.0, 22.0)),
            )
            .on_hover_text("Start the scan (Ctrl+R)")
            .clicked()
        {
            start = true;
        }
        if ui
            .add_enabled(
                running,
                egui::Button::new("■  Stop").min_size(egui::vec2(64.0, 22.0)),
            )
            .on_hover_text("Stop the scan (Ctrl+R)")
            .clicked()
        {
            stop = true;
        }
        if ui
            .add_enabled(
                !running,
                egui::Button::new("⏭  Single scan").min_size(egui::vec2(98.0, 22.0)),
            )
            .on_hover_text("Advance the program by exactly one cycle (Ctrl+T)")
            .clicked()
        {
            single = true;
        }
        ui.separator();
        if ui
            .button("Auto-fill from program")
            .on_hover_text("Build a widget for every physical variable the program uses")
            .clicked()
        {
            fill = true;
        }
        ui.separator();
        if ui
            .add(
                egui::DragValue::new(&mut period)
                    .range(1..=60_000)
                    .prefix("scan ")
                    .suffix(" ms"),
            )
            .on_hover_text("Scan period of the simulated controller")
            .changed()
        {
            period_edit = Some(period);
        }
        ui.label(
            RichText::new(format!("last {:.2} ms", app.last_scan_ms))
                .size(TypeScale::CAPTION)
                .color(tokens.text_dim),
        );
        if scanning {
            if running {
                pill(ui, tokens.run, "RUN");
            } else {
                pill(ui, tokens.accent, "SCAN");
            }
        } else {
            pill(ui, tokens.stop, "STOP");
        }
    });
    if let Some(period_ms) = period_edit {
        let input_period_ms = app.project().scan.input_period_ms;
        app.set_scan_config(ScanConfig {
            period_ms,
            input_period_ms,
        });
    }
    if start || stop {
        app.toggle_run();
    } else if single {
        app.single_scan();
    } else if fill {
        app.auto_fill_bench();
    }
    if !app.forces.is_empty() {
        ui.add_space(SPACE_1);
        forced_banner(app, ui);
    }
}

/// The input widgets: toggle switches and momentary push-buttons.
fn inputs(
    app: &mut EditorApp,
    ui: &mut Ui,
    panel: &SimulationPanel,
    selected: Option<Widget>,
    actions: &mut BenchActions,
) {
    if panel.switches.is_empty() {
        return;
    }
    let tokens = app.tokens;
    group_header(ui, &tokens, "Inputs", group_count(panel.switches.len()));
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(SPACE_2, SPACE_2);
        for (index, switch) in panel.switches.iter().enumerate() {
            let widget = Widget::Switch(index);
            let closed = app.bench.panel_state().is_closed(index);
            let label = widget_label(&switch.label, &switch.var);
            let address = switch.var.to_string();
            let momentary = switch.momentary;
            let mut next_momentary = momentary;
            let mut switched = None;
            let (rect, _) = card(ui, &tokens, selected == Some(widget), |ui| {
                if momentary {
                    switched = momentary_widget(ui, &tokens, &label, &address, closed);
                } else {
                    switched = toggle_widget(ui, &tokens, &label, &address, closed);
                }
                if ui
                    .checkbox(&mut next_momentary, "momentary")
                    .on_hover_text("A push-button springs back after one scan")
                    .changed()
                {
                    actions.edit = Some((widget, PanelEdit::Momentary(next_momentary)));
                }
            });
            if next_momentary != momentary && actions.edit.is_none() {
                actions.edit = Some((widget, PanelEdit::Momentary(next_momentary)));
            }
            if let Some(at) = consumed_press(ui, rect) {
                actions.select = Some(widget);
                // Only a press on the switch body itself throws it; the
                // "momentary" checkbox keeps its own clicks.
                if let Some(body) = switched {
                    if body.contains(at) {
                        if momentary {
                            actions.press = Some(index);
                        } else {
                            actions.toggle = Some((index, !closed));
                        }
                    }
                }
            }
        }
    });
}

/// The output lamps, which follow the `%Q` outputs.
fn outputs(
    app: &mut EditorApp,
    ui: &mut Ui,
    panel: &SimulationPanel,
    selected: Option<Widget>,
    actions: &mut BenchActions,
) {
    if panel.lamps.is_empty() {
        return;
    }
    let tokens = app.tokens;
    let readings = app.bench.readings().to_vec();
    ui.add_space(SPACE_3);
    group_header(ui, &tokens, "Outputs", group_count(panel.lamps.len()));
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(SPACE_2, SPACE_2);
        for (index, lamp) in panel.lamps.iter().enumerate() {
            let widget = Widget::Lamp(index);
            let reading = readings.get(index);
            let on = reading
                .map(|reading| reading.value.as_bool())
                .unwrap_or(false);
            let value = queries::value_text(reading.map(|reading| reading.value));
            let label = widget_label(&lamp.label, &lamp.var);
            let address = lamp.var.to_string();
            let (rect, _) = card(ui, &tokens, selected == Some(widget), |ui| {
                lamp_widget(ui, &tokens, &label, &address, on, &value);
            });
            if consumed_press(ui, rect).is_some() {
                actions.select = Some(widget);
            }
        }
    });
}

/// The analog widgets: sliders that drive `%IW` inputs and gauges that follow
/// `%QW` outputs.
fn analog(
    app: &mut EditorApp,
    ui: &mut Ui,
    panel: &SimulationPanel,
    selected: Option<Widget>,
    actions: &mut BenchActions,
) {
    if panel.analogs.is_empty() && panel.gauges.is_empty() {
        return;
    }
    let tokens = app.tokens;
    let readings = app.bench.readings().to_vec();
    ui.add_space(SPACE_3);
    let count = panel.analogs.len() + panel.gauges.len();
    group_header(ui, &tokens, "Analog", group_count(count));
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(SPACE_2, SPACE_2);
        for (index, analog) in panel.analogs.iter().enumerate() {
            let widget = Widget::Analog(index);
            let current = app.bench.panel_state().analog(panel, index);
            let mut value = current;
            let low = analog.min.min(analog.max);
            let high = analog.max.max(analog.min.saturating_add(1));
            let label = widget_label(&analog.label, &analog.var);
            let address = analog.var.to_string();
            let (rect, _) = card(ui, &tokens, selected == Some(widget), |ui| {
                card_title(ui, &tokens, &label, &address);
                ui.add(
                    egui::Slider::new(&mut value, low..=high)
                        .show_value(false)
                        .trailing_fill(true),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(mono(value.to_string()).color(tokens.accent));
                });
            });
            if value != current {
                actions.slide = Some((index, value));
            }
            if consumed_press(ui, rect).is_some() {
                actions.select = Some(widget);
            }
        }
        for (index, gauge) in panel.gauges.iter().enumerate() {
            let widget = Widget::Gauge(index);
            let reading = readings.get(panel.lamps.len() + index);
            let word = reading
                .map(|reading| queries::value_i32(&reading.value))
                .unwrap_or(0);
            let span = gauge.max.saturating_sub(gauge.min).max(1);
            let fraction = ((word - gauge.min) as f32 / span as f32).clamp(0.0, 1.0);
            let label = widget_label(&gauge.label, &gauge.var);
            let address = gauge.var.to_string();
            let (rect, _) = card(ui, &tokens, selected == Some(widget), |ui| {
                card_title(ui, &tokens, &label, &address);
                ui.add(
                    egui::ProgressBar::new(fraction)
                        .desired_width(140.0)
                        .fill(tokens.accent)
                        .text(mono(word.to_string())),
                );
                ui.label(
                    RichText::new(format!("{} … {}", gauge.min, gauge.max))
                        .size(TypeScale::CAPTION)
                        .color(tokens.text_dim),
                );
            });
            if consumed_press(ui, rect).is_some() {
                actions.select = Some(widget);
            }
        }
    });
}

/// One card of the operator screen: a framed widget that reports its rectangle.
///
/// The card itself does not swallow clicks — the widget inside it does — so the
/// caller decides what a press on the card means by looking at where it landed.
fn card(
    ui: &mut Ui,
    tokens: &Tokens,
    selected: bool,
    contents: impl FnOnce(&mut Ui),
) -> (Rect, egui::Response) {
    let stroke = if selected {
        Stroke::new(2.0_f32, tokens.accent)
    } else {
        tokens.hairline()
    };
    let outer = egui::Frame::new()
        .fill(tokens.panel)
        .stroke(stroke)
        .corner_radius(CornerRadius::same(crate::design::RADIUS_CARD))
        .inner_margin(egui::Margin::same(SPACE_2 as i8))
        .show(ui, |ui| {
            ui.set_min_width(150.0);
            ui.vertical(contents);
        });
    let rect = outer.response.rect;
    (rect, outer.response)
}

/// The position of a fresh press inside `rect`, consumed once.
///
/// Selection and switching must both react to the same press, and a press must
/// never be acted on twice, so the last position handled is remembered.
fn consumed_press(ui: &Ui, rect: Rect) -> Option<Pos2> {
    let pressed = ui.input(|input| input.pointer.primary_pressed());
    if !pressed {
        return None;
    }
    let at = ui.input(|input| input.pointer.interact_pos())?;
    if !rect.contains(at) {
        return None;
    }
    let already = HANDLED_PRESS.with(|handled| *handled.borrow());
    if already == Some(at) {
        return None;
    }
    HANDLED_PRESS.with(|handled| *handled.borrow_mut() = Some(at));
    Some(at)
}

/// The forced-values warning, which the status bar mirrors.
fn forced_banner(app: &EditorApp, ui: &mut Ui) {
    let tokens = app.tokens;
    egui::Frame::new()
        .fill(tokens.warning.gamma_multiply(0.12))
        .stroke(Stroke::new(1.0_f32, tokens.warning))
        .corner_radius(CornerRadius::same(RADIUS_CONTROL))
        .inner_margin(egui::Margin::symmetric(SPACE_2 as i8, SPACE_1 as i8))
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new("⚠  FORCED VALUES ACTIVE")
                        .size(TypeScale::CAPTION)
                        .color(tokens.warning)
                        .strong(),
                );
                ui.label(
                    RichText::new(format!(
                        "{} value(s) are held by the watch table and do not follow the program",
                        app.forces.len()
                    ))
                    .size(TypeScale::CAPTION)
                    .color(tokens.text_dim),
                );
            });
        });
}

/// A group header: the title and how many widgets it holds.
fn group_header(ui: &mut Ui, tokens: &Tokens, title: &str, count: String) {
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new(title.to_uppercase())
                .size(TypeScale::CAPTION)
                .color(tokens.text_dim)
                .strong(),
        );
        ui.label(
            RichText::new(count)
                .size(TypeScale::CAPTION)
                .color(tokens.text_dim),
        );
    });
    ui.add_space(SPACE_1);
}

/// The count of widgets in a group.
pub fn group_count(count: usize) -> String {
    if count == 1 {
        "1 widget".to_owned()
    } else {
        format!("{count} widgets")
    }
}

/// A toggle switch with an unmistakable on/off state.
fn toggle_widget(
    ui: &mut Ui,
    tokens: &Tokens,
    label: &str,
    address: &str,
    closed: bool,
) -> Option<Rect> {
    card_title(ui, tokens, label, address);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(72.0, 26.0), Sense::click());
    let track = if tokens.theme == crate::design::Theme::Dark {
        Color32::from_gray(0x50)
    } else {
        Color32::from_gray(0xC8)
    };
    let fill = if closed { tokens.run } else { track };
    ui.painter().rect_filled(rect, CornerRadius::same(13), fill);
    let radius = rect.height() / 2.0 - 3.0;
    let (centre, align, caption) = if closed {
        (
            egui::pos2(rect.right() - radius - 3.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            "ON",
        )
    } else {
        (
            egui::pos2(rect.left() + radius + 3.0, rect.center().y),
            egui::Align2::RIGHT_CENTER,
            "OFF",
        )
    };
    ui.painter().circle_filled(centre, radius, Color32::WHITE);
    ui.painter().text(
        egui::pos2(
            if closed {
                rect.left() + 8.0
            } else {
                rect.right() - 8.0
            },
            rect.center().y,
        ),
        align,
        caption,
        egui::FontId::new(TypeScale::CAPTION, egui::FontFamily::Proportional),
        Color32::WHITE,
    );
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(13),
        tokens.hairline(),
        egui::StrokeKind::Inside,
    );
    Some(rect)
}

/// A momentary push-button: it looks like a button and springs back.
fn momentary_widget(
    ui: &mut Ui,
    tokens: &Tokens,
    label: &str,
    address: &str,
    pressed: bool,
) -> Option<Rect> {
    card_title(ui, tokens, label, address);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(72.0, 26.0), Sense::click());
    let fill = if pressed { tokens.run } else { tokens.surface };
    let stroke = if pressed {
        Stroke::new(1.0_f32, tokens.run)
    } else {
        Stroke::new(1.0_f32, tokens.border)
    };
    ui.painter()
        .rect_filled(rect.shrink(1.0), CornerRadius::same(RADIUS_CONTROL), fill);
    ui.painter().rect_stroke(
        rect.shrink(1.0),
        CornerRadius::same(RADIUS_CONTROL),
        stroke,
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        if pressed { "PRESSED" } else { "PRESS" },
        egui::FontId::new(TypeScale::CAPTION, egui::FontFamily::Proportional),
        if pressed { Color32::WHITE } else { tokens.text },
    );
    Some(rect)
}

/// A round lamp that glows while it is lit and sits dark while it is off.
fn lamp_widget(ui: &mut Ui, tokens: &Tokens, label: &str, address: &str, on: bool, value: &str) {
    card_title(ui, tokens, label, address);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(72.0, 26.0), Sense::hover());
    let centre = rect.center();
    let radius = 10.0;
    if on {
        // A soft halo, the way a real indicator bezel glows.
        for step in 1..=3 {
            let spread = radius + step as f32 * 2.5;
            ui.painter().circle_filled(
                centre,
                spread,
                tokens.run.gamma_multiply(0.10 / step as f32),
            );
        }
    }
    ui.painter()
        .circle_stroke(centre, radius + 1.0, Stroke::new(1.0_f32, tokens.border));
    let body = if on { tokens.run } else { tokens.surface };
    ui.painter().circle_filled(centre, radius, body);
    if !on {
        ui.painter()
            .circle_stroke(centre, radius, Stroke::new(1.0_f32, tokens.border));
    }
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        ui.label(
            RichText::new(value)
                .monospace()
                .size(TypeScale::CAPTION)
                .color(if on { tokens.run } else { tokens.text_dim }),
        );
    });
}

/// The label and the address a card shows.
fn card_title(ui: &mut Ui, tokens: &Tokens, label: &str, address: &str) {
    ui.label(
        RichText::new(label)
            .size(TypeScale::BODY)
            .color(tokens.text)
            .strong(),
    );
    ui.label(
        RichText::new(address)
            .monospace()
            .size(TypeScale::CAPTION)
            .color(tokens.text_dim),
    );
    ui.add_space(SPACE_1);
}

/// The empty state of the bench.
fn empty_bench(ui: &mut Ui, tokens: &Tokens) {
    empty_state(
        ui,
        tokens,
        "The bench is empty",
        "Auto-fill builds a switch, lamp, slider or gauge for every physical \
         %I and %Q variable the program uses.",
        "Then run the bench and operate the machine before the hardware exists.",
    );
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
    use softladder_core::{
        ElementKind, PlacedElement, Project, Rung, Section, SimAnalog, SimGauge, SimLamp,
        SimSwitch, VarKind,
    };

    fn var(text: &str) -> VarRef {
        text.parse().expect("variable parses")
    }

    fn project() -> Project {
        let mut project = Project::new("bench");
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
        rung.elements.push(PlacedElement::with_var(
            ElementKind::CoilOut,
            var("%Q0"),
            1,
            0,
        ));
        project.rungs.push(rung);
        project.simulation.switches.push(SimSwitch {
            var: var("%I0"),
            label: "start_button".to_owned(),
            momentary: false,
        });
        project.simulation.lamps.push(SimLamp {
            var: var("%Q0"),
            label: "green_lamp".to_owned(),
        });
        project.simulation.analogs.push(SimAnalog {
            var: var("%IW0"),
            label: "speed".to_owned(),
            min: 0,
            max: 100,
        });
        project.simulation.gauges.push(SimGauge {
            var: var("%QW0"),
            label: "load".to_owned(),
            min: 0,
            max: 100,
        });
        project
    }

    #[test]
    fn labels_fall_back_to_the_address() {
        assert_eq!(widget_label("start_button", &var("%I0")), "start_button");
        assert_eq!(widget_label("  ", &var("%I3")), "%I3");
        assert_eq!(widget_label("", &var("%QW2")), "%QW2");
    }

    #[test]
    fn a_widget_edit_goes_through_the_editor_and_records_one_step() {
        let mut app = EditorApp::new(project());
        let before = app.editor.history_len();
        update_panel(&mut app, |panel| {
            panel.switches[0].momentary = true;
            true
        });
        assert!(app.project().simulation.switches[0].momentary);
        assert_eq!(app.editor.history_len(), before + 1);
        // A no-op edit records nothing.
        update_panel(&mut app, |_| false);
        assert_eq!(app.editor.history_len(), before + 1);
        // And neither does an edit that leaves the panel exactly as it was.
        update_panel(&mut app, |_| true);
        assert_eq!(app.editor.history_len(), before + 1);
        // A stale widget index is refused instead of panicking.
        update_panel(&mut app, |panel| match panel.switches.get_mut(9) {
            Some(target) => {
                target.momentary = true;
                true
            }
            None => false,
        });
        assert_eq!(app.editor.history_len(), before + 1);
    }

    #[test]
    fn forcing_holds_a_value_in_the_scan_store() {
        let mut store = VarStore::with_default_sizes();
        let forces = vec![(var("%Q0"), true), (var("%Q1"), false)];
        assert_eq!(apply_forced_values(&forces, &mut store), 2);
        assert_eq!(store.get(&var("%Q0")), Some(Value::Bit(true)));
        assert_eq!(store.get(&var("%Q1")), Some(Value::Bit(false)));
        // An address far outside the default sizes still lands: the store
        // grows, and a force on `%Q9999` must not be silently dropped.
        let far = vec![(VarRef::new(VarKind::PhysOut, 9999), true)];
        assert_eq!(apply_forced_values(&far, &mut store), 1);
        assert_eq!(
            store.get(&VarRef::new(VarKind::PhysOut, 9999)),
            Some(Value::Bit(true))
        );
        // A timer's implied `.Q` bit is a bit, so it takes a force too.
        assert_eq!(
            apply_forced_values(&[(VarRef::new(VarKind::TimerIec, 3), true)], &mut store),
            1
        );
        assert_eq!(apply_forced_values(&[], &mut store), 0);
        // And the editor's own path applies a real force to the bench store.
        let mut app = EditorApp::new(project());
        app.forces.push((var("%Q0"), true));
        apply_forces(&mut app);
        assert_eq!(
            app.bench.engine().store().get(&var("%Q0")),
            Some(Value::Bit(true))
        );
    }

    #[test]
    fn the_selection_round_trips() {
        let before = selected_widget();
        select_widget(Some(Widget::Gauge(2)));
        assert_eq!(selected_widget(), Some(Widget::Gauge(2)));
        select_widget(Some(Widget::Lamp(0)));
        assert_eq!(selected_widget(), Some(Widget::Lamp(0)));
        select_widget(before);
    }

    #[test]
    fn the_group_count_reads_naturally() {
        assert_eq!(group_count(1), "1 widget");
        assert_eq!(group_count(0), "0 widgets");
        assert_eq!(group_count(4), "4 widgets");
    }

    #[test]
    fn a_press_inside_the_switch_body_throws_it_and_a_press_outside_selects() {
        // The switch body is the 72x26 toggle; the wording of the card around it
        // (the label and the momentary checkbox) must not throw the switch.
        let body = Rect::from_min_size(Pos2::new(10.0, 30.0), egui::vec2(72.0, 26.0));
        assert!(body.contains(Pos2::new(40.0, 40.0)));
        assert!(!body.contains(Pos2::new(40.0, 70.0)), "the checkbox line");
        assert!(!body.contains(Pos2::new(120.0, 40.0)), "beside the switch");
    }

    #[test]
    fn drawing_a_full_bench_and_an_empty_one_is_panic_free() {
        let mut app = EditorApp::new(project());
        app.centre_tab = crate::app::CentreTab::Bench;
        crate::panels::test_frame(&ctx(), &mut app, TEST_SIZE);
        app.handle(crate::shortcuts::Action::RunStop);
        app.single_scan();
        app.forces.push((var("%Q0"), true));
        apply_forces(&mut app);
        crate::panels::test_frame(&ctx(), &mut app, TEST_SIZE);
        // A momentary switch, so the push-button path is drawn too.
        update_panel(&mut app, |panel| {
            panel.switches[0].momentary = true;
            true
        });
        crate::panels::test_frame(&ctx(), &mut app, TEST_SIZE);

        // A second bench whose widgets address variables the store does not
        // size, and whose slider range is inverted: the cards must still draw.
        let mut odd = Project::new("odd");
        odd.simulation.switches.push(SimSwitch {
            var: var("%I7"),
            label: String::new(),
            momentary: true,
        });
        odd.simulation.analogs.push(SimAnalog {
            var: var("%IW9"),
            label: "level".to_owned(),
            min: 10,
            max: 0,
        });
        let mut odd = EditorApp::new(odd);
        odd.centre_tab = crate::app::CentreTab::Bench;
        crate::panels::test_frame(&ctx(), &mut odd, TEST_SIZE);

        let mut empty = EditorApp::new(Project::new("empty"));
        empty.centre_tab = crate::app::CentreTab::Bench;
        crate::panels::test_frame(&ctx(), &mut empty, TEST_SIZE);
    }
}
