//! The SoftLadder desktop editor: an `eframe`/`egui` front end over
//! [`softladder_edit`].
//!
//! The crate is deliberately thin. [`softladder_edit::Editor`] owns the project,
//! the file, the undo history and the Problems list; [`softladder_edit::Bench`]
//! owns the running program and the operator's bench positions. This crate draws
//! them and turns input into [`softladder_edit::Command`]s and method calls, and
//! holds no authoritative state of its own:
//!
//! * [`app::EditorApp`] — view state plus every action, independent of the
//!   window, which is why the actions are unit-tested.
//! * [`canvas`] — the rung canvas: grid, power rail, elements, live indication.
//! * [`sfc`] — the sequential (SFC/Grafcet) document: pages, steps, transitions,
//!   their derived wiring and the live chart.
//! * [`palette`] — the element palette and the defaults a placed element gets.
//! * [`layout`] — cell ↔ pixel mapping and the pan/zoom camera (pure).
//! * [`shortcuts`] — keyboard → action mapping (pure).
//! * [`queries`] — rung/problem/variable/power-flow queries (pure).
//! * [`fileops`] — native open/save dialogs.
//! * [`panels`] — the left, right, central-strip and status regions.
//!
//! [`SoftLadderApp`] is the `eframe::App` wrapper the binary runs. The start-up
//! project is `examples/traffic_light.slprj`, loaded relative to the working
//! directory with a silent fallback to an empty project.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

use std::path::{Path, PathBuf};

use eframe::egui;
use softladder_core::Project;
use softladder_edit::Editor;
use softladder_project::native;

pub mod app;
pub mod canvas;
pub mod design;
pub mod fileops;
pub mod layout;
pub mod palette;
pub mod panels;
pub mod queries;
pub mod sfc;
pub mod shell;
pub mod shortcuts;
pub mod symbols;
pub mod watch;

pub use app::{CentreTab, EditorApp, RightTab, SymbolDraft, Tool};
pub use design::{Theme, Tokens};
pub use watch::{ValueFormat, WatchRow};

/// Project opened at start-up, relative to the current working directory.
pub const STARTUP_PROJECT: &str = "examples/traffic_light.slprj";

/// The project named on the command line, if there is one.
pub fn project_from_arguments() -> Option<PathBuf> {
    let mut arguments = std::env::args_os().skip(1);
    let first = arguments.next()?;
    // Ignore a lone flag such as `--help`: only a path is meaningful here.
    let path = PathBuf::from(first);
    if path.to_string_lossy().starts_with('-') {
        return None;
    }
    Some(path)
}

/// Loads the start-up project, falling back to an empty project.
pub fn load_startup_project() -> Project {
    native::load(Path::new(STARTUP_PROJECT)).unwrap_or_default()
}

/// The `eframe` application the `softladder-editor` binary runs.
pub struct SoftLadderApp {
    app: EditorApp,
}

impl SoftLadderApp {
    /// Creates the editor and loads [`STARTUP_PROJECT`] when it exists.
    pub fn new(_context: &eframe::CreationContext<'_>) -> Self {
        Self::with_project(load_startup_project())
    }

    /// Creates the editor around the project named on the command line.
    ///
    /// A project passed as an argument is what every industrial tool accepts, and
    /// it is also how the editor opens a file when it is launched from a
    /// directory that has no `examples/` next to it. A path that cannot be read
    /// falls back to the start-up project and says so in the status bar.
    pub fn open(_context: &eframe::CreationContext<'_>, path: Option<PathBuf>) -> Self {
        let Some(path) = path else {
            return Self::with_project(load_startup_project());
        };
        match Editor::open(&path) {
            Ok(editor) => {
                let mut app = EditorApp::with_editor(editor);
                app.note(&format!("opened {}", path.display()));
                Self { app }
            }
            Err(error) => {
                let mut app = EditorApp::new(load_startup_project());
                app.note(&format!("cannot open {}: {error}", path.display()));
                Self { app }
            }
        }
    }

    /// Creates the editor around an explicit project.
    pub fn with_project(project: Project) -> Self {
        Self {
            app: EditorApp::new(project),
        }
    }

    /// The editor logic, for embedding and tests.
    pub fn editor_app(&self) -> &EditorApp {
        &self.app
    }

    /// The editor logic, mutably.
    pub fn editor_app_mut(&mut self) -> &mut EditorApp {
        &mut self.app
    }

    /// The project currently loaded in the editor.
    pub fn project(&self) -> &Project {
        self.app.project()
    }

    /// Advances the bench by one cycle when it is running.
    ///
    /// The frame loop calls this indirectly through
    /// [`EditorApp::update`]; it stays public so a headless caller can drive the
    /// simulation exactly like the window does.
    pub fn advance(&mut self) {
        if self.app.bench().state().is_scanning() {
            self.app.bench_step();
        }
    }
}

impl eframe::App for SoftLadderApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.app.update(ctx, frame);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use softladder_core::ElementKind;

    #[test]
    fn the_startup_constant_points_at_the_example() {
        assert_eq!(STARTUP_PROJECT, "examples/traffic_light.slprj");
        let project = load_startup_project();
        // The example exists in the repository, but the loader must also cope
        // with being run from another directory.
        assert!(project.schema_version >= 2 || project.sections.is_empty());
    }

    #[test]
    fn a_window_free_app_answers_the_public_questions() {
        let mut project = Project::new("ui test");
        project
            .sections
            .push(softladder_core::Section::new(1, "Main"));
        let mut app = SoftLadderApp::with_project(project);
        assert_eq!(app.project().name, "ui test");
        assert_eq!(app.editor_app().selected_section, 0);
        app.advance();
        app.editor_app_mut()
            .handle(shortcuts::Action::Pick(ElementKind::ContactNo));
        assert_eq!(app.editor_app().tool, Tool::Place(ElementKind::ContactNo));
    }
}
