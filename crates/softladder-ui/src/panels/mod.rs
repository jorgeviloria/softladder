//! The editor's panels.
//!
//! Each submodule draws one region of `docs/EDITOR.md`'s layout and routes every
//! change through methods on [`crate::app::EditorApp`], which in turn only calls
//! `softladder-edit`. `bench`, `watch`, `problems` and `symbols` are the tabs of
//! the right-hand panel.

pub mod bench;
pub mod left;
pub mod palette;
pub mod problems;
pub mod properties;
pub mod right;
pub mod shortcuts;
pub mod status;
pub mod symbols;
pub mod watch;
