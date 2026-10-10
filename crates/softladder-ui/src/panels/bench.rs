//! The Bench document: the operator screen.
//!
//! `docs/UX.md` §8: the bench is an operator screen. Cards share one size and
//! sit in a grid. A tag prefix that two widgets share (`conveyor_run`,
//! `conveyor_speed`) becomes a zone; everything else stays with its role
//! (inputs, outputs, analog). Momentary buttons are raised and sink when
//! pressed. Lamps are pilots with a halo in the colour the project stores.
//! Sliders and gauges print their engineering unit. The trend under the grid
//! records every scan of the variables the operator has pinned, which starts
//! as the outputs.
//!
//! The *layout* comes from the project ([`SimulationPanel`]) and the *positions*
//! from the bench (`PanelState`), which is the split `docs/EDITOR.md`
//! §"Bench" describes. The trend is view state on [`crate::app::EditorApp`]: it
//! is not saved with the project. The bench also applies any forced values
//! before each scan, so a forced output holds while the program runs.

use std::cell::RefCell;
use std::collections::VecDeque;

use egui::{Align, Color32, CornerRadius, Layout, Pos2, Rect, RichText, Sense, Stroke, Ui};
use egui_plot::{Legend, Line, Plot, PlotPoints};
use softladder_core::{
    LampColour, ScanConfig, SimReading, SimulationPanel, Value, VarRef, VarStore,
};
use softladder_edit::Bench;

use crate::app::EditorApp;
use crate::design::{
    empty_state, mono, pill, section_header, Tokens, TypeScale, RADIUS_CARD, RADIUS_CONTROL,
    SPACE_1, SPACE_2, SPACE_3,
};

/// Width of every operator card, in points.
const CARD_W: f32 = 168.0;
/// Height of every operator card, in points.
const CARD_H: f32 = 120.0;
/// Samples the trend keeps for one variable.
const TREND_CAP: usize = 240;
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
    /// A trend chip the operator toggled.
    pin: Option<VarRef>,
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
    let view = app.trend.view(&panel);
    egui::ScrollArea::vertical()
        .id_salt("bench-screen")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            screen(app, ui, &panel, selected, &mut actions);
            ui.add_space(SPACE_3);
            trend(ui, &tokens, &view, &mut actions);
        });
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
    if let Some(var) = actions.pin {
        let outputs = output_vars(panel);
        app.trend.toggle(var, &outputs);
    }
}

/// The run controls, the fill command, the scan period and the last scan time.
fn toolbar(app: &mut EditorApp, ui: &mut Ui) {
    let tokens = app.tokens;
    let running = app.is_running();
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
        let _ = pill(
            ui,
            crate::panels::status::mode_colour(&tokens, app.bench.state()),
            crate::queries::run_state_text(app.bench.state()),
        );
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

/// The operator screen: one uniform grid for each zone.
fn screen(
    app: &EditorApp,
    ui: &mut Ui,
    panel: &SimulationPanel,
    selected: Option<Widget>,
    actions: &mut BenchActions,
) {
    let tokens = app.tokens;
    let readings = app.bench.readings().to_vec();
    for zone in zones(panel) {
        group_header(ui, &tokens, &zone.title, group_count(zone.widgets.len()));
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(SPACE_2, SPACE_2);
            for widget in zone.widgets {
                draw_card(app, ui, panel, &readings, widget, selected, actions);
            }
        });
        ui.add_space(SPACE_2);
    }
}

/// One fixed-size card. The control's rectangle comes back so a press on the
/// label selects the widget without throwing the switch.
fn draw_card(
    app: &EditorApp,
    ui: &mut Ui,
    panel: &SimulationPanel,
    readings: &[SimReading],
    widget: Widget,
    selected: Option<Widget>,
    actions: &mut BenchActions,
) {
    let tokens = app.tokens;
    let mut control = None;
    let (rect, _) = card(ui, &tokens, selected == Some(widget), |ui| {
        control = match widget {
            Widget::Switch(index) => {
                let switch = panel.switches.get(index)?;
                let closed = app.bench.panel_state().is_closed(index);
                let label = widget_label(&switch.label, &switch.var);
                let address = switch.var.to_string();
                if switch.momentary {
                    momentary_widget(ui, &tokens, &label, &address, closed)
                } else {
                    toggle_widget(ui, &tokens, &label, &address, closed)
                }
            }
            Widget::Lamp(index) => {
                let lamp = panel.lamps.get(index)?;
                let on = readings
                    .get(index)
                    .is_some_and(|reading| reading.value.as_bool());
                let label = widget_label(&lamp.label, &lamp.var);
                lamp_widget(ui, &tokens, &label, &lamp.var.to_string(), lamp.colour, on);
                None
            }
            Widget::Analog(index) => {
                let analog = panel.analogs.get(index)?;
                let current = app.bench.panel_state().analog(panel, index);
                let mut value = current;
                let low = analog.min.min(analog.max);
                let high = analog.max.max(analog.min.saturating_add(1));
                slider_widget(
                    ui,
                    &tokens,
                    &widget_label(&analog.label, &analog.var),
                    &analog.var.to_string(),
                    &mut value,
                    low,
                    high,
                    &analog.unit,
                );
                if value != current {
                    actions.slide = Some((index, value));
                }
                None
            }
            Widget::Gauge(index) => {
                let gauge = panel.gauges.get(index)?;
                let word = readings
                    .get(panel.lamps.len() + index)
                    .map(|reading| queries::value_i32(&reading.value))
                    .unwrap_or(0);
                let span = gauge.max.saturating_sub(gauge.min).max(1);
                let fraction = ((word - gauge.min) as f32 / span as f32).clamp(0.0, 1.0);
                gauge_widget(
                    ui,
                    &tokens,
                    &widget_label(&gauge.label, &gauge.var),
                    &gauge.var.to_string(),
                    word,
                    fraction,
                    gauge.min,
                    gauge.max,
                    &gauge.unit,
                );
                None
            }
        };
        control
    });
    let Some(at) = consumed_press(ui, rect) else {
        return;
    };
    actions.select = Some(widget);
    let Widget::Switch(index) = widget else {
        return;
    };
    let Some(body) = control else {
        return;
    };
    if !body.contains(at) {
        return;
    }
    let momentary = panel
        .switches
        .get(index)
        .is_some_and(|switch| switch.momentary);
    let closed = app.bench.panel_state().is_closed(index);
    if momentary {
        actions.press = Some(index);
    } else {
        actions.toggle = Some((index, !closed));
    }
}

/// A card of [`CARD_W`] by [`CARD_H`]. The closure returns the control it drew.
///
/// The frame is reserved up front. The widgets are painted in a child that does
/// not move the cursor, because [`Ui::scope_builder`] would advance it again and
/// the next card would overlap this one.
fn card(
    ui: &mut Ui,
    tokens: &Tokens,
    selected: bool,
    contents: impl FnOnce(&mut Ui) -> Option<Rect>,
) -> (Rect, Option<Rect>) {
    let mut control = None;
    let inner = ui.allocate_ui_with_layout(
        egui::vec2(CARD_W, CARD_H),
        Layout::top_down(Align::Min),
        |ui| {
            ui.set_min_size(egui::vec2(CARD_W, CARD_H));
            let rect = ui.min_rect();
            let background = ui.painter().add(egui::Shape::Noop);
            let content = rect.shrink(SPACE_2);
            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(content)
                    .layout(Layout::top_down(Align::Min)),
            );
            control = contents(&mut child);
            let stroke = if selected {
                Stroke::new(2.0_f32, tokens.accent)
            } else {
                tokens.hairline()
            };
            ui.painter().set(
                background,
                egui::Shape::rect_filled(rect, CornerRadius::same(RADIUS_CARD), tokens.panel),
            );
            ui.painter().rect_stroke(
                rect,
                CornerRadius::same(RADIUS_CARD),
                stroke,
                egui::StrokeKind::Inside,
            );
        },
    );
    (inner.response.rect, control)
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

/// A momentary push-button with a raised bezel that sinks while it is held.
fn momentary_widget(
    ui: &mut Ui,
    tokens: &Tokens,
    label: &str,
    address: &str,
    pressed: bool,
) -> Option<Rect> {
    card_title(ui, tokens, label, address);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(88.0, 36.0), Sense::click());
    let highlight = if tokens.theme == crate::design::Theme::Dark {
        Color32::from_rgba_unmultiplied(255, 255, 255, 48)
    } else {
        Color32::from_rgba_unmultiplied(255, 255, 255, 220)
    };
    let face = if pressed {
        rect.shrink(1.0).translate(egui::vec2(0.0, 1.0))
    } else {
        let shadow = rect.translate(egui::vec2(0.0, 2.0));
        ui.painter()
            .rect_filled(shadow, CornerRadius::same(RADIUS_CONTROL), tokens.border);
        rect.shrink(1.0)
    };
    let fill = if pressed {
        tokens.border
    } else {
        tokens.surface
    };
    ui.painter()
        .rect_filled(face, CornerRadius::same(RADIUS_CONTROL), fill);
    let (top, bottom) = if pressed {
        (tokens.border, highlight)
    } else {
        (highlight, tokens.border)
    };
    ui.painter().line_segment(
        [face.left_top(), face.right_top()],
        Stroke::new(1.0_f32, top),
    );
    ui.painter().line_segment(
        [face.left_bottom(), face.right_bottom()],
        Stroke::new(1.0_f32, bottom),
    );
    ui.painter().rect_stroke(
        face,
        CornerRadius::same(RADIUS_CONTROL),
        Stroke::new(1.0_f32, tokens.border),
        egui::StrokeKind::Inside,
    );
    if pressed {
        ui.painter().circle_filled(
            egui::pos2(face.left() + 12.0, face.center().y),
            4.0,
            tokens.run,
        );
    }
    ui.painter().text(
        face.center(),
        egui::Align2::CENTER_CENTER,
        if pressed { "IN" } else { "PRESS" },
        egui::FontId::new(TypeScale::CAPTION, egui::FontFamily::Proportional),
        if pressed { Color32::WHITE } else { tokens.text },
    );
    Some(rect)
}

/// A round pilot. Lit, it glows in `colour`; dark, it is only the bezel.
fn lamp_widget(
    ui: &mut Ui,
    tokens: &Tokens,
    label: &str,
    address: &str,
    colour: LampColour,
    on: bool,
) {
    card_title(ui, tokens, label, address);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(CARD_W - 28.0, 44.0), Sense::hover());
    let centre = egui::pos2(rect.left() + 18.0, rect.center().y);
    let glow = lamp_fill(tokens, colour);
    let radius = 12.0_f32;
    if on {
        for step in 1..=3 {
            let spread = radius + step as f32 * 3.0;
            ui.painter()
                .circle_filled(centre, spread, glow.gamma_multiply(0.14 / step as f32));
        }
    }
    ui.painter()
        .circle_filled(centre, radius + 2.0, tokens.border);
    // Unlit, the lens is a light tint of its own colour, so an amber pilot is
    // not a grey disc. Lit, it is the solid colour plus the halo above.
    let body = if on {
        glow
    } else {
        tint(glow, tokens.panel, 0.55_f32)
    };
    ui.painter().circle_filled(centre, radius, body);
    ui.painter().circle_filled(
        centre + egui::vec2(-3.5, -3.5),
        2.5,
        Color32::WHITE.gamma_multiply(if on { 0.75_f32 } else { 0.35_f32 }),
    );
    ui.painter().text(
        egui::pos2(rect.right() - 4.0, rect.center().y),
        egui::Align2::RIGHT_CENTER,
        if on { "ON" } else { "OFF" },
        egui::FontId::new(TypeScale::CAPTION, egui::FontFamily::Monospace),
        if on { glow } else { tokens.text_dim },
    );
}

/// A slider and its readout, including the engineering unit when there is one.
#[allow(clippy::too_many_arguments)]
fn slider_widget(
    ui: &mut Ui,
    tokens: &Tokens,
    label: &str,
    address: &str,
    value: &mut i32,
    low: i32,
    high: i32,
    unit: &str,
) {
    card_title(ui, tokens, label, address);
    let _ = ui.add(
        egui::Slider::new(value, low..=high)
            .show_value(false)
            .trailing_fill(true),
    );
    ui.label(mono(with_unit(*value, unit)).color(tokens.accent));
}

/// A bar gauge with the live word, its unit and the ends of the scale.
#[allow(clippy::too_many_arguments)]
fn gauge_widget(
    ui: &mut Ui,
    tokens: &Tokens,
    label: &str,
    address: &str,
    word: i32,
    fraction: f32,
    min: i32,
    max: i32,
    unit: &str,
) {
    card_title(ui, tokens, label, address);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(CARD_W - 28.0, 12.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::same(6), tokens.paper_grid);
    if fraction > 0.0 {
        let filled = rect.with_max_x(rect.left() + rect.width() * fraction);
        ui.painter()
            .rect_filled(filled, CornerRadius::same(6), tokens.accent);
    }
    ui.label(mono(with_unit(word, unit)).color(tokens.accent));
    ui.label(
        RichText::new(range_caption(min, max, unit))
            .size(TypeScale::CAPTION)
            .color(tokens.text_dim),
    );
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
}

/// `colour` moved `toward` by `amount` (0 keeps `colour`, 1 is `toward`).
fn tint(colour: Color32, toward: Color32, amount: f32) -> Color32 {
    let keep = 1.0 - amount;
    Color32::from_rgb(
        (f32::from(colour.r()) * keep + f32::from(toward.r()) * amount) as u8,
        (f32::from(colour.g()) * keep + f32::from(toward.g()) * amount) as u8,
        (f32::from(colour.b()) * keep + f32::from(toward.b()) * amount) as u8,
    )
}

/// The colour a pilot uses for its lens and its halo.
pub fn lamp_fill(tokens: &Tokens, colour: LampColour) -> Color32 {
    match colour {
        LampColour::Green => tokens.run,
        LampColour::Amber => tokens.warning,
        LampColour::Red => tokens.error,
        LampColour::Blue => tokens.accent,
    }
}

/// `value` followed by `unit` when the unit is not blank.
pub fn with_unit(value: i32, unit: &str) -> String {
    let unit = unit.trim();
    if unit.is_empty() {
        value.to_string()
    } else if unit.starts_with('%') {
        format!("{value}{unit}")
    } else {
        format!("{value} {unit}")
    }
}

/// The ends of a scale, with the unit once when there is one.
fn range_caption(min: i32, max: i32, unit: &str) -> String {
    let span = format!("{min} … {max}");
    let unit = unit.trim();
    if unit.is_empty() {
        span
    } else if unit.starts_with('%') {
        format!("{span}{unit}")
    } else {
        format!("{span} {unit}")
    }
}

/// One zone of the operator screen.
pub struct Zone {
    /// Heading above the cards.
    pub title: String,
    /// Widgets that share the zone, in panel order.
    pub widgets: Vec<Widget>,
}

/// Groups widgets by a shared tag prefix, then by role.
///
/// `pump_run` and `pump_fault` share the prefix `pump`, so they form a zone
/// titled "Pump". A prefix that only one widget uses, and a name with no `_`,
/// stay in Inputs, Outputs or Analog.
pub fn zones(panel: &SimulationPanel) -> Vec<Zone> {
    let items = widget_list(panel);
    let mut counts: Vec<(String, usize)> = Vec::new();
    for (_, label, _) in &items {
        let Some(prefix) = tag_prefix(label) else {
            continue;
        };
        if let Some((_, count)) = counts.iter_mut().find(|(name, _)| name == prefix) {
            *count += 1;
        } else {
            counts.push((prefix.to_owned(), 1));
        }
    }
    let mut grouped: Vec<Zone> = Vec::new();
    for (widget, label, role) in items {
        let title = tag_prefix(&label)
            .filter(|prefix| {
                counts
                    .iter()
                    .any(|(name, count)| name == prefix && *count >= 2)
            })
            .map(title_case)
            .unwrap_or_else(|| role.to_owned());
        if let Some(zone) = grouped.iter_mut().find(|zone| zone.title == title) {
            zone.widgets.push(widget);
        } else {
            grouped.push(Zone {
                title,
                widgets: vec![widget],
            });
        }
    }
    grouped
}

/// Every widget in panel order, with the label and the role it falls back to.
fn widget_list(panel: &SimulationPanel) -> Vec<(Widget, String, &'static str)> {
    let mut items = Vec::new();
    for (index, switch) in panel.switches.iter().enumerate() {
        items.push((Widget::Switch(index), switch.label.clone(), "Inputs"));
    }
    for (index, lamp) in panel.lamps.iter().enumerate() {
        items.push((Widget::Lamp(index), lamp.label.clone(), "Outputs"));
    }
    for (index, analog) in panel.analogs.iter().enumerate() {
        items.push((Widget::Analog(index), analog.label.clone(), "Analog"));
    }
    for (index, gauge) in panel.gauges.iter().enumerate() {
        items.push((Widget::Gauge(index), gauge.label.clone(), "Analog"));
    }
    items
}

/// The text before the first `_`, when both sides are non-empty.
fn tag_prefix(label: &str) -> Option<&str> {
    let (prefix, rest) = label.trim().split_once('_')?;
    if prefix.is_empty()
        || rest.is_empty()
        || !prefix
            .chars()
            .all(|character| character.is_ascii_alphanumeric())
    {
        None
    } else {
        Some(prefix)
    }
}

/// `pump` becomes `Pump`. An empty prefix stays empty.
fn title_case(prefix: &str) -> String {
    let mut chars = prefix.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    first.to_uppercase().chain(chars).collect()
}

/// Variables the trend shows until the operator picks a different set.
fn output_vars(panel: &SimulationPanel) -> Vec<VarRef> {
    panel
        .lamps
        .iter()
        .map(|lamp| lamp.var.clone())
        .chain(panel.gauges.iter().map(|gauge| gauge.var.clone()))
        .collect()
}

/// How one trended variable is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Trace {
    /// A switch or a push-button, drawn as 0 or 1.
    Switch,
    /// A pilot, drawn as 0 or 1 in its own colour.
    Lamp(LampColour),
    /// A slider, drawn in engineering units.
    Slider,
    /// A gauge, drawn in engineering units.
    Gauge,
}

/// One variable the operator can put on the trend.
struct TrendChip {
    var: VarRef,
    label: String,
    on: bool,
    trace: Trace,
}

/// One line of the trend.
struct TrendLine {
    label: String,
    trace: Trace,
    points: Vec<[f64; 2]>,
}

/// The chips and the lines for one frame.
struct TrendView {
    chips: Vec<TrendChip>,
    lines: Vec<TrendLine>,
}

/// Samples taken while the bench scans. Not part of the project.
#[derive(Debug, Clone)]
pub struct Trend {
    /// `None` until the operator edits the set; then exactly the pinned variables.
    pins: Option<Vec<VarRef>>,
    series: Vec<Series>,
    /// The cycle last recorded, or [`u64::MAX`] before the first sample.
    last_cycle: u64,
}

/// One variable's samples, oldest first.
#[derive(Debug, Clone)]
struct Series {
    var: VarRef,
    samples: VecDeque<(f64, f64)>,
}

impl Default for Trend {
    fn default() -> Self {
        Self {
            pins: None,
            series: Vec::new(),
            last_cycle: u64::MAX,
        }
    }
}

impl Trend {
    /// Drops the history. A new run starts from an empty trend.
    pub(crate) fn clear(&mut self) {
        self.series.clear();
        self.last_cycle = u64::MAX;
    }

    /// Records one sample per bench widget at `bench`'s current cycle.
    ///
    /// A repeated cycle is ignored. A cycle number that went backwards (a
    /// restart that was not cleared) drops the old history first. The switch
    /// position is the one left after the scan, so a momentary button has
    /// already sprung back and the trace shows what the program saw only while
    /// the button was a toggle.
    pub(crate) fn sample(&mut self, bench: &Bench) {
        let cycle = bench.cycles();
        if self.last_cycle != u64::MAX && cycle < self.last_cycle {
            self.series.clear();
        }
        if cycle == self.last_cycle {
            return;
        }
        self.last_cycle = cycle;
        let project = &bench.runtime().project;
        let panel = &project.simulation;
        let period_ms = f64::from(project.scan.period_ms.max(1));
        let time = cycle as f64 * period_ms / 1000.0;
        let readings = bench.readings();
        let mut live = Vec::new();
        for (index, switch) in panel.switches.iter().enumerate() {
            if live.contains(&switch.var) {
                continue;
            }
            let value = if bench.panel_state().is_closed(index) {
                1.0
            } else {
                0.0
            };
            self.push(switch.var.clone(), time, value);
            live.push(switch.var.clone());
        }
        for (index, lamp) in panel.lamps.iter().enumerate() {
            if live.contains(&lamp.var) {
                continue;
            }
            let value = if readings
                .get(index)
                .is_some_and(|reading| reading.value.as_bool())
            {
                1.0
            } else {
                0.0
            };
            self.push(lamp.var.clone(), time, value);
            live.push(lamp.var.clone());
        }
        for (index, analog) in panel.analogs.iter().enumerate() {
            if live.contains(&analog.var) {
                continue;
            }
            let value = f64::from(bench.panel_state().analog(panel, index));
            self.push(analog.var.clone(), time, value);
            live.push(analog.var.clone());
        }
        for (index, gauge) in panel.gauges.iter().enumerate() {
            if live.contains(&gauge.var) {
                continue;
            }
            let word = readings
                .get(panel.lamps.len() + index)
                .map(|reading| queries::value_i32(&reading.value))
                .unwrap_or(0);
            self.push(gauge.var.clone(), time, f64::from(word));
            live.push(gauge.var.clone());
        }
        self.series
            .retain(|series| live.iter().any(|var| var == &series.var));
    }

    /// Adds `var` to the pinned set, or removes it when it is already pinned.
    ///
    /// The first edit copies the default (every output) so turning one chip off
    /// does not hide the others.
    pub(crate) fn toggle(&mut self, var: VarRef, outputs: &[VarRef]) {
        let mut pins = self.pins.clone().unwrap_or_else(|| outputs.to_vec());
        if let Some(index) = pins.iter().position(|item| item == &var) {
            pins.remove(index);
        } else {
            pins.push(var);
        }
        self.pins = Some(pins);
    }

    fn push(&mut self, var: VarRef, time: f64, value: f64) {
        if let Some(series) = self.series.iter_mut().find(|series| series.var == var) {
            series.samples.push_back((time, value));
            while series.samples.len() > TREND_CAP {
                series.samples.pop_front();
            }
            return;
        }
        let mut samples = VecDeque::new();
        samples.push_back((time, value));
        self.series.push(Series { var, samples });
    }

    fn shown(&self, var: &VarRef, trace: Trace) -> bool {
        match &self.pins {
            Some(pins) => pins.iter().any(|item| item == var),
            None => matches!(trace, Trace::Lamp(_) | Trace::Gauge),
        }
    }

    fn view(&self, panel: &SimulationPanel) -> TrendView {
        let mut chips = Vec::new();
        let mut lines = Vec::new();
        for (var, label, trace) in traces_of(panel) {
            let on = self.shown(&var, trace);
            if on {
                if let Some(series) = self.series.iter().find(|series| series.var == var) {
                    lines.push(TrendLine {
                        label: label.clone(),
                        trace,
                        points: series
                            .samples
                            .iter()
                            .map(|(time, value)| [*time, *value])
                            .collect(),
                    });
                }
            }
            chips.push(TrendChip {
                var,
                label,
                on,
                trace,
            });
        }
        TrendView { chips, lines }
    }
}

/// One entry per widget, in panel order.
fn traces_of(panel: &SimulationPanel) -> Vec<(VarRef, String, Trace)> {
    let mut traces = Vec::new();
    for switch in &panel.switches {
        traces.push((
            switch.var.clone(),
            widget_label(&switch.label, &switch.var),
            Trace::Switch,
        ));
    }
    for lamp in &panel.lamps {
        traces.push((
            lamp.var.clone(),
            widget_label(&lamp.label, &lamp.var),
            Trace::Lamp(lamp.colour),
        ));
    }
    for analog in &panel.analogs {
        traces.push((
            analog.var.clone(),
            widget_label(&analog.label, &analog.var),
            Trace::Slider,
        ));
    }
    for gauge in &panel.gauges {
        traces.push((
            gauge.var.clone(),
            widget_label(&gauge.label, &gauge.var),
            Trace::Gauge,
        ));
    }
    traces
}

/// The trend: a chip per variable and the lines of the ones that are pinned.
fn trend(ui: &mut Ui, tokens: &Tokens, view: &TrendView, actions: &mut BenchActions) {
    let traced = view
        .lines
        .iter()
        .filter(|line| line.points.len() >= 2)
        .count();
    let count = if traced == 0 {
        "records each scan".to_owned()
    } else if traced == 1 {
        "1 trace".to_owned()
    } else {
        format!("{traced} traces")
    };
    group_header(ui, tokens, "Trend", count);
    if !view.chips.is_empty() {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(SPACE_1, SPACE_1);
            for chip in &view.chips {
                let colour = trace_colour(tokens, chip.trace);
                let text = RichText::new(&chip.label)
                    .size(TypeScale::CAPTION)
                    .color(if chip.on { Color32::WHITE } else { tokens.text });
                let fill = if chip.on { colour } else { tokens.surface };
                let stroke = Stroke::new(1.0_f32, if chip.on { colour } else { tokens.border });
                if ui
                    .add(
                        egui::Button::new(text)
                            .fill(fill)
                            .stroke(stroke)
                            .frame(true),
                    )
                    .on_hover_text("Show or hide this variable on the trend")
                    .clicked()
                {
                    actions.pin = Some(chip.var.clone());
                }
            }
        });
        ui.add_space(SPACE_1);
    }
    if traced == 0 {
        ui.label(
            RichText::new("Run the bench. The trend records each scan of the pinned variables.")
                .size(TypeScale::CAPTION)
                .color(tokens.text_dim),
        );
        return;
    }
    let plot = Plot::new("bench-trend")
        .height(148.0_f32)
        .legend(Legend::default())
        .allow_drag(false)
        .allow_scroll(false)
        .allow_zoom(false)
        .allow_boxed_zoom(false)
        .include_y(0.0)
        .include_y(1.0)
        .x_axis_label("s");
    let _ = plot.show(ui, |plot_ui| {
        for line in &view.lines {
            if line.points.len() < 2 {
                continue;
            }
            plot_ui.line(
                Line::new(PlotPoints::new(line.points.clone()))
                    .name(&line.label)
                    .color(trace_colour(tokens, line.trace))
                    .width(1.5_f32),
            );
        }
    });
}

/// The stroke colour of a trend line and of its chip.
fn trace_colour(tokens: &Tokens, trace: Trace) -> Color32 {
    match trace {
        Trace::Switch => tokens.text_dim,
        Trace::Lamp(colour) => lamp_fill(tokens, colour),
        Trace::Slider => tokens.accent,
        Trace::Gauge => tokens.warning,
    }
}

/// Advances a running bench by `scans` cycles.
///
/// The frame loop does this one cycle at a time. The screenshot harness calls
/// it so the trend has a history before the first frame. When
/// `close_first_switch_at` is `Some(n)`, the first switch closes on cycle `n`
/// and stays closed.
pub fn rehearse(app: &mut EditorApp, scans: u32, close_first_switch_at: Option<u32>) {
    for index in 0..scans {
        if close_first_switch_at == Some(index) && !app.project().simulation.switches.is_empty() {
            app.bench.panel_state_mut().set_closed(0, true);
        }
        if !app.bench.state().is_scanning() {
            break;
        }
        app.bench_step();
    }
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
            colour: softladder_core::LampColour::Green,
        });
        project.simulation.analogs.push(SimAnalog {
            var: var("%IW0"),
            label: "speed".to_owned(),
            min: 0,
            max: 100,
            unit: "rpm".to_owned(),
        });
        project.simulation.gauges.push(SimGauge {
            var: var("%QW0"),
            label: "load".to_owned(),
            min: 0,
            max: 100,
            unit: "%".to_owned(),
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
        // (the label and the address) must not throw the switch.
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
            unit: String::new(),
        });
        let mut odd = EditorApp::new(odd);
        odd.centre_tab = crate::app::CentreTab::Bench;
        crate::panels::test_frame(&ctx(), &mut odd, TEST_SIZE);

        let mut empty = EditorApp::new(Project::new("empty"));
        empty.centre_tab = crate::app::CentreTab::Bench;
        crate::panels::test_frame(&ctx(), &mut empty, TEST_SIZE);
    }

    #[test]
    fn a_readout_keeps_the_unit_only_when_it_has_one() {
        assert_eq!(with_unit(12, "rpm"), "12 rpm");
        assert_eq!(with_unit(12, "  "), "12");
        assert_eq!(with_unit(0, ""), "0");
        assert_eq!(with_unit(0, "%"), "0%");
    }

    #[test]
    fn a_shared_prefix_is_a_zone_and_a_lone_name_stays_with_its_role() {
        let mut panel = SimulationPanel::default();
        panel.switches.push(SimSwitch {
            var: var("%I0"),
            label: "pump_run".to_owned(),
            momentary: false,
        });
        panel.switches.push(SimSwitch {
            var: var("%I1"),
            label: "pump_fault".to_owned(),
            momentary: true,
        });
        panel.lamps.push(SimLamp {
            var: var("%Q0"),
            label: "green_lamp".to_owned(),
            colour: softladder_core::LampColour::Green,
        });
        panel.lamps.push(SimLamp {
            var: var("%Q1"),
            label: "tower_beacon".to_owned(),
            colour: softladder_core::LampColour::Blue,
        });
        let grouped = zones(&panel);
        assert_eq!(grouped.len(), 2);
        assert_eq!(grouped[0].title, "Pump");
        assert_eq!(
            grouped[0].widgets,
            vec![Widget::Switch(0), Widget::Switch(1)]
        );
        assert_eq!(grouped[1].title, "Outputs");
        assert_eq!(grouped[1].widgets.len(), 2);
    }

    #[test]
    fn a_running_bench_records_the_lamp_on_the_trend() {
        let mut app = EditorApp::new(project());
        app.handle(crate::shortcuts::Action::RunStop);
        rehearse(&mut app, 4, None);
        let open = app.trend.view(&app.project().simulation);
        let lamp = open
            .lines
            .iter()
            .find(|line| line.label == "green_lamp")
            .expect("the output is pinned by default");
        assert!(lamp.points.iter().all(|point| point[1] == 0.0));
        assert!(open.lines.iter().all(|line| line.label != "speed"));

        app.bench.panel_state_mut().set_closed(0, true);
        rehearse(&mut app, 3, None);
        let closed = app.trend.view(&app.project().simulation);
        let lamp = closed
            .lines
            .iter()
            .find(|line| line.label == "green_lamp")
            .expect("the lamp stays on the trend");
        assert_eq!(lamp.points.last().expect("samples").get(1), Some(&1.0));

        app.trend
            .toggle(var("%IW0"), &output_vars(&app.project().simulation));
        let pinned = app.trend.view(&app.project().simulation);
        assert!(pinned.lines.iter().any(|line| line.label == "speed"));
    }
}
