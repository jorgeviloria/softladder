# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased] — the shift and rotate carry

### Fixed

- **`%S8` is written by the shift and rotate functions again**, closing divergence 2 of
  `testdata/known-divergences.md`. `SHL`, `SHR`, `ROL` and `ROR` report the operand's most
  significant bit for a left shift or rotate and its least significant bit for a right one —
  whatever the count, and before shifting — exactly as `arithm_eval.c` does; the last such operation
  in an expression wins, and the engine publishes it to the system bit `%S8` as soon as the
  expression is evaluated, so a later rung of the same scan reads it. A scan that runs no shift never
  touches the bit. The evaluator stays pure: it *reports* the carry (`EvalEffects`) and the engine
  writes it.

## [Unreleased] — the SFC editor

### Added

- **The sequential document.** An `Sfc` section opens a chart editor in the centre pane: the page on
  the same paper as the ladder, with a square coarse grid and one band per page (number, comment and
  a state badge from the diagnostics). Steps are squares — the initial one with a doubled border and
  an `init` chip, the active one filled in `tokens.energised` with its `%X<n>.V` time on a chip —
  and transitions are a bar crossed by the IEC mark with their condition beside it (tag over address,
  or the monospace expression, `always` when unconditional). Links are **derived from the model**
  (orthogonal runs, idle/energised strokes), never placed by hand, and a junction of more than one
  source or target is drawn with the **double bar**.
- **Fourteen undoable chart commands** in `softladder-edit`: insert/remove/move a step, set its
  number or its initial flag, insert/remove/move a transition, set its condition (parsed, refused
  with the reason), add or remove a step from its source/target sets, edit the page comment, and add
  or remove the page. Each validates before mutating, records one undo entry, replays defensively,
  and a property test proves that any sequence of them undoes and redoes exactly (it found a real
  dangling-reference bug on its first run).
- **A sequential palette** in the ribbon's Insert group (Select, Init, Step, Trans, Link, AND, OR,
  Note) that replaces the ladder's when the open section is a chart, with a ghost while placing, a
  context menu, `Del`/arrows/`Esc`, wheel zoom and pan.
- **Tree and inspector.** The project tree lists a chart's pages, steps (`0 · initial`) and
  transitions (`T0 %I0`) instead of rungs; the inspector edits the page (comment, counts), a step
  (number with inline validation, initial flag, position, live activity and timer) and a transition
  (condition with the ladder's validation and tag picker, plus per-step *deactivates*/*activates*
  checkboxes and the AND/OR badge).
- Clicking a chart diagnostic in Problems opens its page and selects the element it names.

## [Unreleased] — function block pins

### Fixed

- **A block's drawn pins are now the rows the engine reads.** The timer drew `IN` *and* `PT` as input
  pins, but the engine only reads the enable — the preset is a parameter — so `PT` could never be
  connected to anything. `ElementKind::input_pins` is now the single source of truth for how many
  rows a block reads, what each one means, and what the canvas draws (and for the band height, so no
  pin falls outside the network). A timer shows `IN`; a counter its four (`R`, `LD`, `CU`, `CD`); a
  register its three (`R`, `IN`, `OUT`).
- A block's **parameter** is labelled as a parameter (`%TM0 · PT 3000`, `%C0 · PV 5`) instead of
  looking like a pin.

## [Unreleased] — M4 sequential (engine and interoperability)

### Added

- **SFC (Grafcet) execution.** A chart lives in an `Sfc` section as a page of steps
  and transitions; the engine runs each chart once per scan in section order with
  the classic single-snapshot rule — every transition is evaluated against the
  step state at the start of the section (all of its source steps active), then
  the union of the sources is cleared and the union of the targets set, so a chain
  advances one transition per scan instead of firing through. Initial steps
  activate on `refresh()`; each step publishes `%X<n>.A` (activity) and
  `%X<n>.V` (elapsed milliseconds); AND divergences/convergences and OR branches
  are supported. `SL-W002` no longer means "skipped".
- **`sequential.csv` interoperability.** The ClassicLadder part moved from
  passthrough to regenerated: `P`/`S`/`T`/`C`/`N` records are read (translating
  array slots into the step numbers `%X<n>` addresses), unmappable records are
  reported with a location, a page no section references gets a synthesized
  section, and the exporter rewrites the part deterministically. All eight charts
  in the corpus (73 steps, 100 transitions) import, scan without errors and stay
  `import → export → import` fixed points.
- `Section::sequential_page` (additive, skipped when absent, so `schema_version`
  stays 2 and existing project bytes are unchanged) and a real
  `softladder-core::sfc` model.
- Chart diagnostics: `SL-W002` (a chart with no page, a transition naming a step
  outside its page), `SL-W011` (a transition with no condition), `SL-W001` (a
  step no transition activates) and `SL-E011` (a transition whose step does not
  exist), each naming the page, step or transition.

### Fixed

- Ladder diagnostics were attributed to the last SFC section in the project; the
  section index is now set per section.

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
