//! The Bench tab: the switches, lamps, sliders and gauges of the project's
//! simulation panel, plus the run controls.
//!
//! The *layout* comes from the project (`SimulationPanel`) and the *positions*
//! from the bench (`PanelState`), which is exactly the split
//! `docs/EDITOR.md` §"Bench" describes. Nothing is cached here.

use egui::{Color32, RichText};
use softladder_core::{ScanConfig, SimulationPanel};

use crate::app::EditorApp;
use crate::queries;

/// Lamp is on.
const ON: Color32 = Color32::from_rgb(90, 220, 120);
/// Lamp is off.
const OFF: Color32 = Color32::from_rgb(70, 76, 86);

/// Draws the run controls and the whole bench.
pub fn show(app: &mut EditorApp, ui: &mut egui::Ui) {
    let running = app.bench.state().is_scanning();
    ui.horizontal_wrapped(|ui| {
        if ui
            .add_enabled(!running, egui::Button::new("Start"))
            .clicked()
        {
            app.toggle_run();
        }
        if ui.add_enabled(running, egui::Button::new("Stop")).clicked() {
            app.toggle_run();
        }
        if ui.button("Single scan").clicked() {
            app.single_scan();
        }
    });
    ui.horizontal_wrapped(|ui| {
        let mut period = app.project().scan.period_ms;
        if ui
            .add(
                egui::DragValue::new(&mut period)
                    .range(1..=60_000)
                    .prefix("scan ")
                    .suffix(" ms"),
            )
            .changed()
        {
            let input_period_ms = app.project().scan.input_period_ms;
            app.set_scan_config(ScanConfig {
                period_ms: period,
                input_period_ms,
            });
        }
        if ui
            .button("Auto-fill from program")
            .on_hover_text("Build a widget for every physical variable the program uses")
            .clicked()
        {
            app.auto_fill_bench();
        }
    });
    ui.separator();

    let panel = app.project().simulation.clone();
    if panel.is_empty() {
        ui.label(RichText::new("This project has no bench layout yet.").weak());
        return;
    }
    let readings = app.bench.readings().to_vec();
    let mut updated: Option<SimulationPanel> = None;
    switches(app, ui, &panel, &mut updated);
    lamps(ui, &panel, &readings);
    analogs(app, ui, &panel, &mut updated);
    gauges(ui, &panel, &readings);
    if let Some(panel) = updated {
        app.set_panel(panel);
    }
}

/// The toggle switches and momentary push-buttons.
fn switches(
    app: &mut EditorApp,
    ui: &mut egui::Ui,
    panel: &SimulationPanel,
    updated: &mut Option<SimulationPanel>,
) {
    if panel.switches.is_empty() {
        return;
    }
    ui.label(RichText::new("Switches").strong());
    for (index, switch) in panel.switches.iter().enumerate() {
        let mut closed = app.bench.panel_state().is_closed(index);
        ui.horizontal(|ui| {
            let label = format!("{}  {}", switch.label, switch.var);
            if switch.momentary {
                if ui
                    .add(egui::Button::new(label).selected(closed))
                    .on_hover_text("Momentary push-button: springs back after one scan")
                    .clicked()
                {
                    app.bench.panel_state_mut().set_closed(index, true);
                    if !app.bench.state().is_scanning() {
                        app.single_scan();
                    }
                }
            } else if ui
                .toggle_value(&mut closed, label)
                .on_hover_text("Toggle switch")
                .changed()
            {
                app.bench.panel_state_mut().set_closed(index, closed);
            }
            let mut momentary = switch.momentary;
            if ui
                .checkbox(&mut momentary, "momentary")
                .on_hover_text("Change the widget type in the project's bench layout")
                .changed()
            {
                let mut next = panel.clone();
                if let Some(target) = next.switches.get_mut(index) {
                    target.momentary = momentary;
                }
                *updated = Some(next);
            }
        });
    }
}

/// The lamps, which follow the `%Q` outputs.
fn lamps(ui: &mut egui::Ui, panel: &SimulationPanel, readings: &[softladder_core::SimReading]) {
    if panel.lamps.is_empty() {
        return;
    }
    ui.add_space(4.0);
    ui.label(RichText::new("Lamps").strong());
    for (index, lamp) in panel.lamps.iter().enumerate() {
        let reading = readings.get(index);
        let on = reading.map(|r| r.value.as_bool()).unwrap_or(false);
        ui.horizontal(|ui| {
            ui.colored_label(if on { ON } else { OFF }, "⬤");
            ui.label(format!("{}  {}", lamp.label, lamp.var));
            ui.monospace(queries::value_text(reading.map(|r| r.value)));
        });
    }
}

/// The analog sliders, which drive the `%IW` inputs.
fn analogs(
    app: &mut EditorApp,
    ui: &mut egui::Ui,
    panel: &SimulationPanel,
    updated: &mut Option<SimulationPanel>,
) {
    if panel.analogs.is_empty() {
        return;
    }
    ui.add_space(4.0);
    ui.label(RichText::new("Sliders").strong());
    for (index, analog) in panel.analogs.iter().enumerate() {
        let mut value = app.bench.panel_state().analog(panel, index);
        let low = analog.min;
        let high = analog.max.max(low.saturating_add(1));
        ui.horizontal(|ui| {
            ui.label(format!("{}  {}", analog.label, analog.var));
            if ui
                .add(egui::Slider::new(&mut value, low..=high).show_value(false))
                .changed()
            {
                app.bench.panel_state_mut().set_analog(panel, index, value);
            }
            ui.monospace(value.to_string());
        });
        let mut min = analog.min;
        let mut max = analog.max;
        ui.horizontal(|ui| {
            ui.label(RichText::new("range").weak());
            let changed_min = ui.add(egui::DragValue::new(&mut min)).changed();
            let changed_max = ui.add(egui::DragValue::new(&mut max)).changed();
            if (changed_min || changed_max) && min <= max {
                let mut next = panel.clone();
                if let Some(target) = next.analogs.get_mut(index) {
                    target.min = min;
                    target.max = max;
                }
                *updated = Some(next);
            }
        });
    }
}

/// The gauges, which follow the `%QW` outputs.
fn gauges(ui: &mut egui::Ui, panel: &SimulationPanel, readings: &[softladder_core::SimReading]) {
    if panel.gauges.is_empty() {
        return;
    }
    ui.add_space(4.0);
    ui.label(RichText::new("Gauges").strong());
    for (index, gauge) in panel.gauges.iter().enumerate() {
        let reading = readings.get(panel.lamps.len() + index);
        let word = reading.map(|r| queries::value_i32(&r.value)).unwrap_or(0);
        let span = gauge.max.saturating_sub(gauge.min).max(1);
        let fraction = ((word - gauge.min) as f32 / span as f32).clamp(0.0, 1.0);
        ui.horizontal(|ui| {
            ui.label(format!("{}  {}", gauge.label, gauge.var));
            ui.add(
                egui::ProgressBar::new(fraction)
                    .desired_width(110.0)
                    .text(word.to_string()),
            );
        });
    }
}
