//! The editor's panels.
//!
//! Each submodule draws one region of `docs/UX.md` §3's shell and routes every
//! change through methods on [`crate::app::EditorApp`], which in turn only calls
//! `softladder-edit`:
//!
//! * [`left`] — the project tree.
//! * [`right`] — the context-sensitive inspector.
//! * [`palette`] — the instruction palette, drawn inside the ribbon.
//! * [`properties`] — the strip under the ribbon for the selected element.
//! * [`bench`], [`watch`], [`problems`], [`tags`] — the centre documents.
//! * [`status`] — the status bar.
//! * [`icons`] — the shared interface glyphs.

pub mod bench;
pub mod icons;
pub mod left;
pub mod palette;
pub mod problems;
pub mod properties;
pub mod right;
pub mod shortcuts;
pub mod status;
pub mod tags;
pub mod watch;

/// Runs one real frame of the interface on a headless [`egui::Context`].
///
/// The screenshot harness and every panel test need the same thing: a context
/// with a screen rectangle, because panels ask for the available space. Keeping
/// it here means a panel test is three lines long.
///
/// Test-only, so the shipped binary carries no harness.
#[cfg(test)]
pub fn test_frame(ctx: &egui::Context, app: &mut crate::app::EditorApp, size: egui::Vec2) {
    let input = || egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
        ..egui::RawInput::default()
    };
    // Two passes: the first builds the font atlas and lays the panels out.
    let _ = ctx.run(input(), |ctx| app.draw(ctx));
    let _ = ctx.run(input(), |ctx| app.draw(ctx));
}
