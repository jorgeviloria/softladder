# ADR-0002 — UI stack: egui / eframe

- **Status:** accepted
- **Date:** 2025
- **Deciders:** project owner

## Context

ClassicLadder's GTK2/GTK3 interface is the main source of its UX debt: single-selection editing, no
undo, modal configuration windows, one element-at-a-time toolbar workflow, variables scattered
across separate top-level windows, and a Linux-only stack. SoftLadder's stated goal is a better UI
and UX, so the toolkit choice is strategic, not cosmetic.

Candidates:

| Option | Pros | Cons |
| --- | --- | --- |
| **egui / eframe** | Pure Rust, single self-contained binary, same code compiles to native and WASM, immediate mode makes canvas/hit-testing/overlays straightforward, one dependency tree | Immediate mode re-lays out every frame (needs culling/caching for huge documents), less "native" widget look, no OS a11y bridging yet |
| Tauri + web frontend | Best-looking UI possible, HTML/CSS, huge ecosystem | Two languages and two state models, IPC boundary, bundling Node + Rust, heavier CI |
| iced | Retained, type-safe MVU | More ceremony for a canvas-centric CAD-like editor; smaller widget ecosystem |
| GTK4 (gtk-rs/relm4) | Closest to LinuxCNC/ClassicLadder, native Linux look | Weak on macOS/Windows, harder declarative canvas work, keeps us in the "desktop Linux only" box |

## Decision

Use **egui/eframe** for the editor, with `egui_dock` for panel layout, `egui_plot` for scopes and
`egui_extras` for tables.

## Consequences

- One binary per platform; no Node toolchain; WASM build becomes a realistic milestone (M9) that
  delivers a zero-install online editor/demo.
- The editor must be engineered for an immediate-mode cost model from the start: viewport culling,
  cached element shapes, an element spatial index for hit tests, and benchmarks in CI.
- The UI holds no authoritative state: it renders `Project` + `Runtime` and dispatches `Command`s
  (which also gives undo/redo and makes edits testable headlessly).
- Text-heavy surfaces (symbol table, cross-references) use `egui_extras` virtualized tables.
- Accessibility and native menu integration (macOS menu bar, file dialogs) are handled explicitly
  via `egui`'s a11y layer and `rfd`; they will not match a native toolkit, and that is accepted.
- Web dashboard: the monitor/dashboard server (M6) is a separate, small web surface; it does not
  depend on the desktop UI.

## Alternatives rejected

- **Tauri**: the UX ceiling is higher, but the dual-language IPC complexity and packaging burden are
  not justified for a PLC editor, and it would slow every milestone.
- **GTK4**: contradicts the cross-platform requirement and the "better UX" goal.
