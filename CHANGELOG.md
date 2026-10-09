# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased] — UI/UX redesign

The editor's interface was rebuilt against the tools people actually program PLCs
with (TIA Portal, Studio 5000, CODESYS, TwinCAT, GX Works, Sysmac Studio, Control
Expert); `docs/UX.md` is the spec and records what is deliberately not copied.

### Added

- A design system (`softladder-ui::design`): one light "paper" theme and a dark
  one, a 4/8/12/16/24 spacing scale, a 11/12/13/20 type scale, corner radii,
  hairlines, pills, cards and empty states — plus contrast tests.
- `softladder-ui::symbols`: the IEC glyphs drawn as schematics (contacts, edge
  contacts, coils, blocks with named pins, expression boxes), shared by the
  canvas and the palette so an element looks the same everywhere, including a
  live (energised) style.
- The application shell: a ribbon with labelled command groups (History, File,
  Online, View, Insert), a menu bar, document tabs with an accent underline, a
  project tree, a context-sensitive inspector and a status bar of badges and
  pills.
- The ladder document: networks with `Network <n>`, title, wrapped comment and a
  state badge; tag names over addresses; a fitted camera; live power flow with
  inline values; hover, selection, a ghost while placing, pan/zoom and a context
  menu.
- PLC tags as a first-class document (name, type, address, comment, "used by"),
  the watch & force table (format, modify, force, with a warning banner while a
  force is active), an operator-screen simulation bench (toggles, spring-back
  push-buttons, glowing lamps, sliders, gauges) and a diagnostics table.
- **Headless screenshots**: `crates/softladder-ui/tests/ui_shots.rs` rasterises a
  real frame (tessellated shapes plus the font atlas) into PNGs under
  `target/ui-shots/`, so the interface can be reviewed and iterated without a
  display server. Dev-only: nothing is added to the shipped binary.
- `EditorApp::{set_variable, variable}` so scripts and the harness can set bench
  inputs without a mouse.
- The editor binary takes a project path (`softladder-editor line.slprj`), the
  way every industrial tool does, and says in the status bar when a path cannot
  be opened. Without an argument it still loads `examples/traffic_light.slprj`
  relative to the working directory.

### Changed

- The palette is grouped by family (Tool, Bit logic, Coils, Timers, Counters,
  Data, Program control) with icons, labels, tooltips and an armed state, instead
  of two rows of raw glyph text.
- The left pane is a project tree instead of a flat list that dumped whole
  comments; the right pane is an inspector instead of a duplicate tab.

## [Unreleased] — M3 ClassicLadder compatibility

### Added

- Real ClassicLadder import and export, validated against the 41 example projects
  upstream ships (272 rungs, 9 273 elements): every file imports, `import →
  export → import` is a fixed point, a second export is byte-identical, and the
  parts SoftLadder does not model survive byte for byte. The
  `softladder-project::classicladder` layer reads every part of a `.clp`/`.clprj`/`.clprjz` container (rungs with
  their dense cell matrices, sections, symbols, IEC timer/counter/register and
  arithmetic-expression tables, project info and general parameters) into a
  `Project`, and writes a document the reference implementation can load back.
- Parts SoftLadder does not model (Modbus and serial configuration, IO mapping,
  events, spy variables, alarm slots, sequential pages) are returned as an
  `extras` template and written back byte for byte, so an import/export cycle
  never loses them.
- Import/export diagnostics with locations: `SL-E030` (malformed document),
  `SL-W030` (element or structure approximated), `SL-W031` (variable family or
  expression not modelled), `SL-W032` (part passed through) and `SL-W033`
  (feature that cannot be exported).
- `Rung::wire_mode`: `Explicit` makes a gap in a live row break the circuit,
  which is ClassicLadder's behaviour and what the importer sets on every rung it
  reads; `Implicit` keeps the editor's forgiving behaviour. See
  [ADR-0007](docs/adr/0007-wire-modes.md).
- Expression library: `ABS`, `MIN`, `MAX`, `AVG`, `POW`, `SHL`, `SHR`, `ROL`,
  `ROR` with the ClassicLadder aliases (`MINI`, `MAXI`, `MOY`), hexadecimal
  literals in the `$8000` spelling, and `&`/`|` accepted as `AND`/`OR`.
- `scripts/fetch_corpus.sh` now also works offline from a local ClassicLadder
  checkout, and CI fetches the corpus so the compatibility tests actually run.
- `testdata/known-divergences.md`: every behavioural difference the corpus exposed,
  with what each side does and why.

### Fixed

- A timer's one-minute base was a 60-minute base (`TIME_BASE_MINS` is 60 000 ms in
  the reference), so an imported off-delay with a two-minute preset ran for two
  hours.
- Re-applying a timer's preset on every scan — which is how an HMI edit is picked
  up — was treated as a preset *change* and restarted a running off delay, so a
  TOF block dropped its output the moment its input fell.
- Imported multi-cell blocks are placed on the column whose power the reference
  taps for their inputs, with the columns they vacate wired through and the
  vertical links of their body cells preserved; before this a block imported from
  the corpus was never enabled.

## [Unreleased] — M2 editor

### Added

- `softladder-edit`: the editor's logic without a window — `Editor` (project +
  bounded 1000-entry undo/redo history + dirty flag + file path + Problems list),
  the `Command` vocabulary (18 variants, each with a human label), and `Bench`
  (a `Runtime` plus the simulation panel's operator positions, with `step`,
  `start`/`stop`, `run_one_cycle`, readings and hot `reload`).
- Simulation bench: `SimulationPanel` (switches, push-buttons, lamps, sliders and
  gauges) is part of the project, while the operator's positions live in runtime
  state, so saving a program never records that somebody left a switch closed.
  `SimulationPanel::auto_fill` mirrors every `%I`/`%Q`/`%IW`/`%QW` the program
  uses and labels each widget with the bound symbol.
- Full egui editor: element palette, pan/zoom rung canvas with live power-flow
  indication, sections and rung list, element properties with variable
  validation, Bench / Watch / Problems / Symbols panels, menu bar, status bar,
  unsaved-changes prompt and native file dialogs.
- New diagnostics `SL-W020` (a bench widget addresses the wrong variable kind or
  has an inverted range) and the `ReplaceElement` command, so dropping a palette
  element on an occupied cell is a single undo step.
- `docs/EDITOR.md`: the editor's UX contract (layout, shortcuts, bench, live
  indication) and what is deliberately left for later milestones.

### Fixed

- Power flow: vertical links are per column (a link no longer merges the rows of
  the columns to its right, which used to let a parallel branch bypass a series
  stop contact) and empty cells in a live row now carry `state_on_left`, so the
  shared power of a merge column also reaches the empty cells of that column.

## [Unreleased] — M1 core

### Added

- `docs/SEMANTICS.md`: normative specification of ladder execution (grid, power
  flow, every element, scan structure, diagnostics) derived from the observable
  behaviour of the reference implementation.
- Schema v2 of `.slprj`: variable references carry an `accessor`
  (`%TM0.Q`, `%TM0.V`, `%TM0.P`, `%C1.D/.V/.P/.E/.F`, `%R0.E/.F/.I/.O/.S`,
  `%X2.A/.V`, `%MW0.3`) instead of the v1 `bit` field, plus a
  direct v1 -> v2 migration chain (`MIGRATIONS`, gzip-aware, idempotent,
  snapshot-tested) and rejection of future schema versions.
- `Symbol::var`: symbols bind to a variable, with backwards-compatible
  deserialization.
- `PlacedElement::connected_with_top`: vertical links that build parallel
  branches and their merge points.
- `softladder-core::lint`: structural diagnostics (`SL-E003`, `SL-E005`,
  `SL-E007`, `SL-E009`, `SL-W001`, `SL-W002`) shared by the CLI and, later, the
  editor and the monitor.
- Deterministic simulated time: `softladder-runtime::Clock`
  (`Simulated`/`Realtime`), `Runtime::run_cycles`, `ScanSummary`, and
  `softladder run` defaulting to simulated time with byte-identical `--json`
  output across runs. `--real-time` opts back into wall-clock pacing.

### Changed

- The scan engine is a full column-major power-flow evaluator: parallel branches
  with vertical merges, implicit horizontal wires in live rows, serial coils,
  per-cell edge detection, compare/operate blocks, IEC timers with three modes
  and time bases (100 ms / 1 s / 60 min), counters with reset/preset/up/down
  input rows and wrap flags, FIFO/LIFO registers, jumps by rung index or label
  and recursive subroutine calls with a 25-frame limit.
- `examples/traffic_light.slprj` is a v2 document and uses a real vertical link
  for its self-holding rung; the CLI `lint` reports the core diagnostics.

## [Unreleased] — M0 skeleton

### Added

- Cargo workspace with the `softladder-core`, `softladder-project`,
  `softladder-runtime`, `softladder-io`, `softladder-monitor`, `softladder-ui`
  and `softladder-cli` crates.
- `softladder-core`: `%`-notation variable references with ClassicLadder
  aliases, ladder-logic project model, expression tokenizer/Pratt parser and
  evaluator, deterministic scan engine with TON/TOF/TP timers, up/down counters
  and edge detection.
- `softladder-project`: deterministic pretty-JSON project persistence
  (`.slprj`), gzip-compressed projects (`.slprjz`) and a parser/serializer for
  the ClassicLadder `_FILES_CLASSICLADDER` text container.
- `softladder-runtime`: runtime state machine, scan statistics and a
  flight-recorder ring buffer.
- `softladder-io`: I/O image, driver trait and in-memory simulation driver.
- `softladder-monitor`: serde-tagged monitoring request/response protocol.
- `softladder-ui`: `eframe`/`egui` editor skeleton (sections panel, rung
  canvas, watch table, status bar).
- `softladder-cli`: `run`, `lint`, `import` and `export` subcommands.
- `examples/traffic_light.slprj`: example project used by the tests and by the
  UI at start-up.
- GitHub Actions CI running `cargo fmt --check`, `cargo clippy -D warnings` and
  the workspace test suite on Linux, macOS and Windows.

### Not yet implemented

- `softladder-project::classicladder` element-level `.clprj` <-> `Project`
  conversion (planned for M3).
- Sequential Function Chart engine (planned for M4).
- Modbus, GPIO and HAL I/O drivers (planned for M5/M7).
- Monitoring server transport (planned for M6).
