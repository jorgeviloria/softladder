//! Native file dialogs and the small path helpers around them.
//!
//! Only the two dialog entry points touch the windowing system (through `rfd`);
//! the extension and label helpers are pure and unit-tested. No `cfg(target_os)`
//! code lives here: `rfd` owns every platform difference.

use std::path::{Path, PathBuf};

/// File extensions a SoftLadder project can use.
pub const PROJECT_EXTENSIONS: [&str; 2] = ["slprj", "slprjz"];

/// Extension appended to a save path that carries none.
pub const DEFAULT_EXTENSION: &str = "slprj";

/// Asks the user for a project to open.
pub fn open_dialog() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("Open project")
        .add_filter("SoftLadder project", &PROJECT_EXTENSIONS)
        .pick_file()
}

/// Asks the user where to write the project.
///
/// `current` seeds the dialog's directory and `suggested_name` its file name, so
/// *Save as* starts next to (and named like) the file that is open.
pub fn save_dialog(current: Option<&Path>, suggested_name: &str) -> Option<PathBuf> {
    let mut dialog = rfd::FileDialog::new()
        .set_title("Save project")
        .add_filter("SoftLadder project", &PROJECT_EXTENSIONS)
        .set_file_name(suggested_name);
    if let Some(directory) = current.and_then(Path::parent) {
        dialog = dialog.set_directory(directory);
    }
    dialog.save_file().map(with_extension)
}

/// Ensures `path` ends in a project extension.
pub fn with_extension(path: PathBuf) -> PathBuf {
    if path.extension().is_some() {
        return path;
    }
    let mut path = path;
    path.set_extension(DEFAULT_EXTENSION);
    path
}

/// Name to seed a *Save as* dialog with, derived from the open path.
pub fn suggested_name(current: Option<&Path>, project_name: &str) -> String {
    if let Some(name) = current.and_then(Path::file_name).and_then(|n| n.to_str()) {
        if !name.is_empty() {
            return name.to_owned();
        }
    }
    let trimmed = project_name.trim();
    if trimmed.is_empty() {
        format!("untitled.{DEFAULT_EXTENSION}")
    } else {
        format!("{trimmed}.{DEFAULT_EXTENSION}")
    }
}

/// How a path is shown in the status bar.
pub fn file_label(path: Option<&Path>) -> String {
    match path {
        Some(path) => path.display().to_string(),
        None => "untitled".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_save_path_without_an_extension_gets_one() {
        assert_eq!(
            with_extension(PathBuf::from("/tmp/plant")),
            PathBuf::from("/tmp/plant.slprj")
        );
        assert_eq!(
            with_extension(PathBuf::from("/tmp/plant.slprj")),
            PathBuf::from("/tmp/plant.slprj")
        );
        assert_eq!(
            with_extension(PathBuf::from("plant.SLPRJZ")),
            PathBuf::from("plant.SLPRJZ"),
            "an existing extension is never replaced"
        );
        assert_eq!(
            with_extension(PathBuf::from("archive.tar.gz")),
            PathBuf::from("archive.tar.gz")
        );
    }

    #[test]
    fn the_suggested_name_prefers_the_open_file() {
        assert_eq!(
            suggested_name(Some(Path::new("/tmp/plant.slprj")), "ignored"),
            "plant.slprj"
        );
        assert_eq!(suggested_name(None, "traffic light"), "traffic light.slprj");
        assert_eq!(suggested_name(None, "   "), "untitled.slprj");
    }

    #[test]
    fn the_file_label_names_an_unsaved_project() {
        assert_eq!(file_label(None), "untitled");
        assert_eq!(
            file_label(Some(Path::new("/tmp/plant.slprj"))),
            "/tmp/plant.slprj"
        );
    }

    #[test]
    fn the_project_extensions_are_the_ones_the_loader_accepts() {
        assert!(PROJECT_EXTENSIONS.contains(&DEFAULT_EXTENSION));
        assert_eq!(PROJECT_EXTENSIONS.len(), 2);
    }
}
