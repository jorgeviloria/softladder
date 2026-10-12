# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased] — stale rung tails are not broken chains

### Fixed

- **A dangling final `NEXTRUNG` warned as a broken chain.** The reference's usual stale tail
  pointer (last rung pointing past the last file) now ends the walk silently when every
  resolvable rung was collected; cycles, missing first rungs and mid-chain holes still warn
  and fall back to ascending order.

## [Unreleased] — passthrough parts survive, quieter Problems

### Added

- **Unmodeled ClassicLadder parts travel inside the project.** `Project.extras` carries them
  (skipped when empty, no schema bump), the importer fills it, native save/load keeps it, and
  export falls back to it as the template — so `import → save → export` preserves them without
  the original container. Passthrough `SL-W032` notices stay in the CLI import log but leave
  the editor's Problems list, where they nagged forever about nothing actionable; the open
  status names how many parts were preserved.

## [Unreleased] — operate `=` imports, `.clp` opens

### Fixed

- **`3pumps.clprj` imported with 14 errors.** Its operate cells spell assignment with a bare
  `=` (`@200/0@=@200/0@+1`), which the importer kept verbatim instead of translating — and the
  verbatim text then failed lint and scan. A single depth-zero `=` now splits target from value
  in operate cells (the reference's own spelling); `:=` still works and round-trips.
- **`.clp` files did not open.** The legacy extension is the same container and the importer
  always accepted it — only the editor's filter and extension gate said no. Both take
  `.clp` now.
- **`SL-W001` cried wolf on linked rows.** A row fed through a vertical link from a railed row
  was flagged anyway; the lint now follows the first cell's link upwards, as the docs always
  said. `3pumps` went from 9 warnings to 1 (a genuinely empty rung).

## [Unreleased] — open ClassicLadder, rung capacity, no drag-pan

### Added

- **Open ClassicLadder projects in the editor.** `File → Open` lists `.clprj`/`.clprjz`
  next to native projects; a container imports as an unsaved project (saving asks for a
  native path) with its import report merged into Problems.
- **A rung's grid capacity is editable.** Empty Columns/Rows in the rung editor mean
  automatic sizing as before; a number floors the sheet, so a fixed 12 × 8 rung draws
  exactly 12 × 8. New `SetRungSize` undoable command; additive format fields, no schema
  bump.

### Removed

- **Drag-pan is gone from ladder and SFC.** The middle button and space-drag no longer move
  the sheet, which read as drift; the wheel scrolls between networks and the zoom lives
  behind `Ctrl/Cmd` + wheel, the buttons and the shortcuts.

## [Unreleased] — the example counter counts again

### Fixed

- **The traffic-light counter never counted.** Its `%I1` pulse entered on row 0, the counter's
  `R` (reset) row, while count-up lives on row 2 (`R/LD/CU/CD` read rows 0–3) — so every
  vehicle held reset instead of counting. The contact now feeds the `CU` row, the unused
  `R`/`LD`/`CD` rows are tied off with the always-false `spare_off` bit (an unwired row is a
  well-deserved `SL-W001`), and the `.sltest` example pulses `%I1` five times asserting
  `%C0.V` 1–5 and `%C0.D`.

## [Unreleased] — the bench floats over the ladder

### Changed

- **The bench is a window, not a tab.** The simulation panel floats over the document (open by
  default, toggled from Tools, the View menu or the project tree, closable with `×`) so the
  operator screen stays visible while the ladder runs beside it.

## [Unreleased] — PID loops in the engine (M9, slice one)

### Added

- **`Pid` control block (engine only).** One enable row, gains and period from parameters,
  `%PID<n>.P`/`.V`/`.O` for setpoint/process/output and bare `%PID<n>` for enabled: positional
  PID on a fixed sample time, bumpless start, frozen output while disabled, anti-windup into
  `i32` range. The editor drawing, palette and ClassicLadder mapping are the next slice.

## [Unreleased] — the counter spells like ClassicLadder

### Changed

- **Counter pins read `R/P/U/D`, and `CV` is no label.** The reference draws single input
  letters and the value as bare `current/preset` numbers; the canvas matches (the value text
  inside the box already did). No engine change: order and rows are untouched.

## [Unreleased] — blocks drive every output row, like ClassicLadder

### Changed

- **Counter and register flags are wires again.** Verified against the reference (`CalcTypeCounter`,
  register block): counters drive empty/done/full on rows `y`/`y+1`/`y+2`, registers empty/full on
  `y`/`y+1`, with the same input rows and apply order — so imported rungs chaining off a secondary
  row now run as written. Rows with no flag read false; `%C0.V` stays variable-only. Closes
  divergence 1 (`testdata/known-divergences.md`); the timer's second-row `Running` output stays out.

## [Unreleased] — one selection box, on the grid

### Changed

- **Hover, selection and ghost share one rectangle**: the full cell, so the three outlines always
  agree with each other and the grid instead of hugging different boxes. Verified against the
  headless screenshots pixel by pixel.

## [Unreleased] — the grid meets the components

### Changed

- **Grid lines run through cells, not between them.** Verticals cross cell centres and horizontals
  run on the row wires, from the same layout the hit-testing uses — so the grid meets every
  symbol and its power flow instead of drifting past them.

## [Unreleased] — the scroll stops at the rungs

### Changed

- **No scrolling onto empty paper.** The camera clamps to the created networks every frame:
  the first band stops at the top, the last at the bottom, and a short section pins instead
  of drifting. Smaller documents no longer slide around at all.

## [Unreleased] — expression boxes show the whole text

### Changed

- **Compare/operate boxes stretch over empty cells** (up to four columns) so their expression
  reads whole instead of clipping at one cell. Only longer expressions still elide, with the
  full text behind the hover tooltip.

## [Unreleased] — the ladder fills the window

### Changed

- **Adaptive column pitch.** The sheet fills the window: columns take their natural width while
  it fits and shrink, never past tag-name readability, on narrower windows — instead of shrinking
  the whole drawing with the zoom. Framing a section only moves the view now; the zoom stays
  where the user left it.

## [Unreleased] — no sideways drift on the documents

### Changed

- **The wheel only moves along the sheet.** Trackpads report a horizontal component on almost
  every vertical scroll, which drifted the ladder sideways under the pointer; raw horizontal
  scroll is now ignored and only an explicit `Shift` + wheel moves sideways, on ladder and SFC.

## [Unreleased] — ladder scroll and honest block pins

### Changed

- **The wheel scrolls.** Rolling the wheel moves between networks (`Shift` + wheel goes
  sideways) instead of zooming; the zoom lives behind `Ctrl/Cmd` + wheel, the ribbon buttons
  and the shortcuts. Same gesture language on the SFC document.
- **A block shows only its real connections.** The flow-out (`Q`, or `E` for a register) keeps
  the single stub on the right, now sitting on the first row's wire where the rung continues;
  the readouts (`ET`, `CV`, `F`) stay as labels with no stub, since they are read as variables
  and can never be wired.

## [Unreleased] — Input fuzzing (M8, slice four)

### Added

- **No-panic fuzzing over every input format.** Proptest suites feed arbitrary text and bytes
  to the expression parser, `VarRef`, the native `.slprj` loader and the whole ClassicLadder
  pipeline (`import_bytes`), plus hostile fragments (index tricks, truncated gzip, split
  terminators). Parsed expressions also display, re-parse to the same tree and evaluate on an
  empty store without panicking. Coverage-guided libFuzzer targets stay future (nightly-only).

## [Unreleased] — Incident replay (M8, slice three)

### Added

- **Record and exactly replay a run.** `record` drives a scenario and samples watched outputs
  per scan; `replay` re-applies the recorded inputs and reports every divergence with cycle,
  variable, expected and actual. Identical projects replay clean; a rewired rung diverges on
  its first affected cycle. JSON round-trips, so a recording is emailable evidence.

## [Unreleased] — `softladder test` (M8, slice two)

### Added

- **`softladder test <project> <scenario>` for CI.** Loads the pair, runs the timeline and
  prints one line per failure plus a summary; exit `0` when everything held, `3` on failures,
  `1` on load errors. Scenario files use the hand-written form (`"%I0"`, `true`, numbers);
  the structured serde form still decodes. Ships with `examples/traffic_light.sltest`
  (start/stop seal-in, 5 assertions, green).

## [Unreleased] — `.sltest` scenarios (M8, slice one)

### Added

- **Deterministic scenario runner.** `Scenario` (input steps + output assertions keyed by scan
  cycle, JSON `.sltest` files via `load_scenario`) runs on a fresh `Runtime` with one scan
  period per cycle: steps apply before their scan, assertions check after. Refused writes are
  `SetupError`, value differences are `Mismatch` with cycle, variable, expected and actual.
  The `softladder test` subcommand is the next slice.

## [Unreleased] — `softladder serve` (M6, slice seven)

### Added

- **Scan while serving both protocols.** `serve` ticks on a wall-clock period while the monitor
  TCP protocol and the dashboard HTTP page share one `MonitoredRuntime` (audited sessions per
  connection, `--token id=role` bearers, loopback binds by default). Finite `--cycles` keep it
  CI-friendly; the summary reports scans and connections per protocol.

## [Unreleased] — Monitor web dashboard (M6, slice six)

### Added

- **Minimal dashboard over plain HTTP, `std` only.** `GET /` serves an offline-friendly page
  polling `/api/status`, `/api/vars` and `/api/alarms`; `POST /api/set|force|release|ack`
  mutates through the same audited sessions and roles as the TCP protocol. Bodies use the
  friendly scalar form. Unknown bearers get `403`, unknown paths `404`. WebSocket and TLS
  stay for later slices.

## [Unreleased] — Monitor end to end (M6, slice five)

### Added

- **Loopback acceptance test: server plus real runtime.** `monitor_loopback` runs a lamp rung and
  a jump to nowhere behind the JSON server: auth, status, force, read-back, alarm list, ack and
  re-alarm after more faulty scans (a live fault clears the ack by design), with the force
  holding throughout. The M6 wire loop is closed; dashboard and TLS stay open.

## [Unreleased] — Monitor alarm journal (M6, slice four)

### Added

- **Alarms with acknowledgement, end to end.** `AlarmJournal` holds the active alarms (repeat
  occurrences refresh instead of duplicating, and un-ack), ordered errors-first; `Alarm.acked`
  travels on the wire; `AckAlarm` (operator+) acknowledges by journal id with `unknown_alarm`
  for ghosts. `MonitoredRuntime` raises every scan diagnostic into the journal and serves it,
  so a jump to nowhere shows up in `GetAlarms` and acks from the session.

## [Unreleased] — Monitor TCP server (M6, slice three)

### Added

- **Line-delimited JSON server, `std` only.** One request per line, one response back, in order;
  the first line authenticates the bearer (answered `Ok`, or `unknown_token` + close). Each
  connection runs its own `Session`, connections are served sequentially, malformed lines answer
  `bad_request` without closing, and silent clients hit a read timeout. Covered by loopback
  tests (round-trip, unknown bearer, viewer force denial, bad lines, sequential connections).
  WebSocket/TLS stay for later slices.

## [Unreleased] — Monitor runtime target (M6, slice two)

### Added

- **`MonitorTarget` for the real runtime.** `MonitoredRuntime` wraps `Runtime` with a force map:
  monitor writes land in the store (and lose to the program on the next scan, like the bench's
  modify column), forces re-apply before every tick so they hold, and writes to a forced
  variable are refused until it is released. Alarms still read empty; the journal is next.

## [Unreleased] — Monitor roles and session (M6, slice one)

### Added

- **Transport-free monitor core.** `Role` (`viewer`/`operator`/`engineer`) gates every `Request`
  exactly as documented; `Token` carries the bearer and its role; `Session` authorizes before
  touching the `MonitorTarget`, so refused calls answer `unauthorized` and never reach the
  runtime; every served mutation appends an `AuditEntry` (token, role, variable, previous
  value). The TCP/WebSocket transports are the next slice.

## [Unreleased] — Modbus master driver (M5, slice four)

### Added

- **Polling Modbus TCP master as an `IoDriver`.** `ModbusMasterDriver` reads discrete inputs and
  input registers into the image and pushes coils and holding registers out of it, all through
  the configured `ModbusMap` and chunked to the protocol limits. Any socket, timeout, protocol
  or slave-exception failure surfaces as `IoError::Driver`; retries stay in the driver,
  fail-safe stays in the runtime. Covered by loopback tests against the in-memory slave,
  including a 3000-channel chunked poll.

## [Unreleased] — Modbus RTU framing (M5, slice three)

### Added

- **RTU framing over the same PDU, pure and dependency-free.** CRC-16/Modbus plus request,
  response and exception codecs (`[unit | PDU | CRC-lo | CRC-hi]`) that reuse the TCP PDU codec,
  so both transports agree on every function code by construction. The serial line itself
  (device, baud rate, inter-frame silence) waits for hardware; `SlaveStore::handle` already
  answers the decoded requests.

## [Unreleased] — Modbus TCP (M5, slice two)

### Added

- **Blocking Modbus TCP master and slave over the configured map, `std` only.** Pure MBAP/PDU
  codec for `01`/`02`/`03`/`04`/`05`/`06`/`0F`/`10` plus exception frames; `TcpMaster` polls with a
  per-call connection, timeout and retries (exceptions are returned, never retried); `SlaveStore`
  answers reads/writes against the wire side of `ModbusMap` and syncs both ways with `IoImage`.
  Unmapped addresses answer `IllegalDataAddress`. Covered by a loopback integration test (master
  against a threaded slave) with no new dependencies. RTU framing is the next slice.

## [Unreleased] — IO mapping (M5, slice one)

### Added

- **Scriptable simulation driver.** `SimScript` is a deterministic input timeline (scan cycle →
  channel changes) and `ScriptPlayer` applies each event exactly once while a run advances, so the
  same script and program always produce the same outputs. Out-of-range channels are skipped, never
  a panic. This is the timeline the future `.sltest` harness will drive.
- **Configurable Modbus address map.** `ModbusMap` binds `%I`/`%Q` to coils/discrete inputs from a
  base address and `%IW`/`%QW` to register runs with kind (`I16`/`U16`/`F32`), scale, offset and
  word order (`value = raw * scale + offset`, clamped). `ModbusMap::fixed` keeps the ClassicLadder
  zero-based mirroring as the default. Pure encode/decode helpers, no socket yet: the TCP/RTU
  transport with polling, retries and supervision is the next slice.

## [Unreleased] — operator bench (UI/UX U5)

### Added

- **The bench is an operator screen.** Cards share one size. A tag prefix that two widgets share
  becomes a zone; the rest stay in Inputs, Outputs or Analog.
- **Pilot colour and engineering units.** A lamp is green, amber, red or blue, with a halo when it
  is lit. A slider or a gauge prints its unit. Auto-fill guesses the colour from the tag name and
  copies the unit from the symbol. Both fields are optional in the project file.
- **A trend under the cards.** It records each scan. Outputs start pinned; a chip shows or hides a
  variable. Starting a run clears the history.

### Changed

- A momentary push-button is drawn raised and sinks while it is held. Whether it is momentary is
  edited in the inspector, not on the card.

## [Unreleased] — data tables (UI/UX U4)

### Added

- **One data table** for the tag list, the watch table and the problems list: a fixed header,
  zebra rows, a filter, click-to-sort columns and a per-row menu.
- **Drag a variable onto Watch & force**, from a tag's grip, from the ladder (drop it on the Watch
  tab or the project tree) or from the cell menu. A forced row is amber, and its control is a lock.

### Changed

- The project tree's tooltip carries the full name and comment. Errors and warnings are separate
  badges; the trailing number is what the row contains.
- The tag and watch add bars sit above the rows, so they stay reachable while the list scrolls.

## [Unreleased] — schematic ladder (UI/UX U3)

### Changed

- **One mode pill.** The status bar no longer pairs an "Offline" badge with a `SCAN` pill. The
  simulator is `SIM RUN`, `SIM SCAN`, `SIM STOP`, `FREEZE` or `LOAD`, and the same word is on the
  bench toolbar. The menu shows the project name, and a chart section's document tab reads `SFC`.
- **The ladder closes on a return rail.** Each coil wires to a neutral bus just past the last coil.
  The bus stays the idle colour: it is the common, not the power. A conducting contact or an
  energised coil is filled, not only stroked.
- **A function block stays inside its network.** The band reserves the padding the box draws above
  its first pin and below its last, so a counter is not cut by the next network's rule. The
  parameter (`PT 3000`, `PV 5`) sits on its own line under the instance name.
- **Labels and value chips scale with the zoom**, and the chip sits beside the address instead of
  on it.

## [Unreleased] — professional shell (UI/UX U1–U2, part one)

### Added

- **Inter and JetBrains Mono** are embedded (SIL OFL 1.1, see `NOTICE`) and replace egui's default
  fonts: Inter for the interface, JetBrains Mono for addresses and expressions, plus semibold
  families for emphasis.
- **Command icons**: undo, redo, new, open, save, save as, play, stop, step, zoom in/out, fit,
  address, moon, sun and search, in the same vector style as the rest of `panels/icons.rs`.

### Changed

- **The ribbon is one row of icon buttons** with tooltips (name, shortcut, description). Undo and
  redo are disabled when there is nothing to undo or redo. Run/Stop is a filled button in the colour
  of what it will do. The address and theme toggles show their state.
- **The instruction palette moved onto the document** it inserts into, as a compact toolbar: glyphs
  for contacts, coils and program control, mnemonics (`TON`, `CTU`, `FIFO`…) for blocks, and family
  and shortcut in the tooltip. It wraps instead of clipping, so every instruction can be reached at
  any window width. At 1440 px it fits in a single 36 px row.
- The property strip above the canvas now shows only while the inspector, which does the same job,
  is closed.
- Secondary text (`text_dim`) is darker in the light theme. A test now checks both themes against
  WCAG AA on every surface it sits on.

### Removed

- `design::caption`, which hard-coded a light-theme colour and had no callers.

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
