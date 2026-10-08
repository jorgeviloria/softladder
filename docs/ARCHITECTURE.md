# SoftLadder — Architecture

> Companion to [`PLAN.md`](PLAN.md) (Spanish, the working plan). This document is the technical
> reference for contributors: crate boundaries, invariants and data flow.

## 1. Crate map

| Crate | Responsibility | Depends on | `unsafe` |
| --- | --- | --- | --- |
| `softladder-core` | Domain model, variable namespace, expression evaluation, scan engine, function blocks, SFC, diagnostics | nothing (only `serde`, `thiserror`) | forbidden |
| `softladder-project` | Native `.slprj` format, migrations, ClassicLadder `.clp/.clprj/.clprjz` import/export, PLCopen XML (later) | `core` | forbidden |
| `softladder-runtime` | Scan scheduler, run-state machine, hot reload, scan statistics, flight recorder/replay, alarms | `core`, `project` | forbidden |
| `softladder-edit` | Editing commands, undo/redo history, file handling and the simulation bench (the editor's logic, without a UI) | `core`, `project`, `runtime` | forbidden |
| `softladder-io` | `IoDriver` trait and drivers: `sim`, `modbus`, `gpio`, `hal` | `core` | forbidden (FFI lives in dedicated `-sys` crates) |
| `softladder-monitor` | Online monitor protocol + web dashboard server | `core` | forbidden |
| `softladder-ui` | egui/eframe editor (main binary) | `core`, `project`, `runtime`, `edit` | forbidden |
| `softladder-cli` | Headless binary: `run`, `lint`, `test`, `import`, `export` | `core`, `project`, `runtime`, `io` | forbidden |

**Invariant:** dependencies only point right-to-left in the table above. `core` must never grow a
dependency on IO, UI, networking, or the system clock.

`softladder-edit` exists so that everything an editor *does* — placing elements, undoing, saving,
driving the simulation bench — is testable in a plain `cargo test` with no window. The egui layer
only draws state and turns input into `Command`s.

## 2. Runtime data flow

```
                 ┌──────────────────────────── softladder-runtime ───────────────────────────┐
                 │                                                                           │
 IoDriver ──read──▶ IoImage (inputs) ──▶ ScanEngine ──▶ outputs ──write──▶ IoDriver          │
                 │        ▲                    │                    │                        │
                 │        │                    ▼                    ▼                        │
                 │   FlightRecorder      VarStore (bits/words)   IoImage (outputs)           │
                 │        │                    │                                             │
                 │        └──────────────┬─────┴──────────────┬──────────────────────────────│
                 └───────────────────────┼────────────────────┼──────────────────────────────┘
                                         ▼                    ▼
                                   monitor/WS            alarm/event journal
```

A **scan cycle** is strictly ordered:

1. `driver.read(&mut image)` — input image refresh (digital + analog, with timestamps/quality).
2. `engine.scan_once(now_ms)` — solve main sections in declared order, then subroutine calls, then
   the active SFC pages; produce `Vec<Diagnostic>` and a `VarStore` snapshot.
3. `driver.write(&image)` — output image flush.
4. Statistics (cycle time, jitter, missed deadlines) and alarm evaluation; flight recorder append.

The engine never reads the clock: `now_ms` is injected by the scheduler. This is what makes scans
reproducible in tests and replayable from a recording.

## 3. Domain invariants

- **Time is a parameter.** No `Instant::now()` inside `core`.
- **No panics on malformed input.** Untrusted input (project files, expressions, monitor frames)
  produces `Diagnostic`s or `Err`, never an abort. Division by zero, index out of range, numeric
  overflow (`checked_*`) and type mismatches are diagnosed per element.
- **One rung's failure is contained.** A faulty element marks its rung (severity `Error`) and the
  rest of the scan continues.
- **Determinism.** Section order, element evaluation order and FB update order are fixed and
  documented; no `HashMap` iteration order influences execution or serialization.
- **Serialization stability.** Project files serialize with a stable key/element order so that
  `git diff` output is meaningful and `insta` snapshots are stable.
- **One process, many PLCs.** `Project` and `Runtime` are ordinary values: tests may instantiate
  several independent instances (no global mutable state, unlike the C implementation's `InfosGene`).

## 4. Variable namespace

| Prefix | Meaning | Type | ClassicLadder equivalent |
| --- | --- | --- | --- |
| `%M` | Internal memory bit | `Bit` | `%B` |
| `%MW` | Internal memory word | `Word`/`DWord` | `%W` |
| `%I` / `%Q` | Physical digital input / output | `Bit` | `%I` / `%Q` |
| `%IW` / `%QW` | Physical analog input / output | `Word`/`Real` | `%IW` / `%QW` |
| `%TM` | IEC timer instance: `%TM3.Q` output, `.V` elapsed, `.P` preset | `Bit`/`Word` | `%TM` |
| `%C` | Counter instance: `%C1.D` done, `.V` value, `.P` preset, `.E`/`.F` wrap flags | `Bit`/`Word` | `%C` |
| `%R` | FIFO/LIFO register: `.E` empty, `.F` full, `.I` in, `.O` out, `.S` count | `Bit`/`Word` | `%R` |
| `%X` | SFC step: `.A` activity, `.V` elapsed | `Bit`/`Word` | `%X` |
| `%S` | System variables (clock, PLC state, scan time) | both | `%S` |
| `%QLED` | User LED (physical and on-screen) | `Bit` | `%QLED` |

Indexing, accessors and bit selection are first-class: `%MW[%MW0]` (index by variable),
`%MW20.3` (bit extraction / insertion inside a word) and the ClassicLadder sub-value spellings above
— the last two are items from the original ClassicLadder TODO list. `Display` always renders the
canonical form, so parsing `%B0` yields `%M0` and parsing `%TM0` yields `%TM0.Q`.

## 5. Expression engine

`softladder-core::expr` is a hand-written tokenizer + Pratt parser. No `eval`, no `unsafe`, no
recursion without a depth limit. The AST is evaluated against a `VarSource` trait, which keeps it
decoupled from storage and trivially mockable in tests.

Supported: integer/real literals, hexadecimal literals in ClassicLadder's `$8000` spelling (and
`0x8000`), variable references (including indexed and accessor forms), `+ - * / %`, `= <> < <= > >=`,
`AND OR XOR NOT` (with `&`/`|` accepted for `AND`/`OR`, which is how ClassicLadder writes them), and
parentheses.

Function library: `ABS`, `MIN`, `MAX`, `AVG`, `POW`, `SHL`, `SHR`, `ROL`, `ROR` — with the
ClassicLadder aliases `MINI`, `MAXI` and `MOY` accepted, and the shifts/rotates matching the
reference's 32-bit behaviour (a logical shift right shifts zeros in; `ROL`/`ROR` rotate). The
remaining M9 math (`SCALE`, `NORM`, `SIN`, `COS`, `SQRT`) plugs into the same `Function` enum.
Every function checks its arity at parse time and its argument domain at evaluation time, returning
`EvalError::BadArgument`/`Overflow` instead of panicking.

## 6. Ladder semantics

A rung is a 2-D grid of cells. Power flow enters from the left rail and propagates cell to cell:
contacts conduct when their variable matches the required state, coils are energized by the flow
reaching them, and `Connection` cells implement vertical links between rows (parallel branches).
The engine evaluates columns left-to-right and propagates vertically downwards, which reproduces
ClassicLadder's behaviour including the ordering caveat for stacked coils.

M1 implements the skeleton (series/parallel detection, edge elements, coils). The authoritative
evaluator with the full element set and the differential test harness against the C engine lands in
M3/M4 (see ADR-0004).

## 7. Project format

See [`FORMAT.md`](FORMAT.md) for the native schema and [`COMPAT.md`](COMPAT.md) for the
ClassicLadder container. Key rules:

- `schema_version` is bumped on every breaking change; migrations are chained and unit-tested.
- `.slprj` (pretty JSON) is the human/git-friendly form; `.slprjz` is gzip for embedded targets.
- Import never silently drops data: unmappable elements become `Warning` diagnostics and an
  `import report`.

## 8. IO layer

`IoDriver` is a pull/push pair over a flat `IoImage`. Drivers own their retry/supervision policy and
report quality; the runtime owns the fail-safe policy (configurable safe state per output channel).

- `sim` — in-process, scriptable from `.sltest` scenarios and from the UI HMI panel.
- `modbus` — master (TCP/RTU) with polling plan, retries and timeout supervision; slave with a
  **configurable** register map (offset, type, scale, word order), an improvement over the fixed
  `%B`/`%W` mirroring of the C implementation.
- `gpio` — libgpiod v2 on generic Linux, `rppal` on Raspberry Pi.
- `hal` — LinuxCNC HAL pins via a dedicated `-sys` crate behind the `linuxcnc` feature.

## 9. UI layer

egui (immediate mode) with `egui_dock` for panel layout. Rendering is recomputed per frame, so the
cost model matters: documents are culled to the visible viewport, element shapes are cached, and the
canvas keeps an element index for O(visible) hit-testing. Target: 60 fps with 5,000 elements.

Editing is command-based: `softladder-edit` exposes `Command`, `Editor` (project + bounded
undo/redo history + dirty flag + file path + problems) and `Bench` (a `Runtime` plus the simulation
panel's operator positions). `apply` snapshots only the part of the project a command touches, so
undo/redo is exact without cloning the whole project per edit.

The UI keeps no authoritative state: it renders `Editor` + `Bench` and emits `Command`s. Anything
that changes the program, the bench or the run state is a method on those two types, which is why
the editor's behaviour is covered by headless tests rather than by clicking through a window.

## 10. Engineering conventions

- MSRV pinned in `Cargo.toml`; `rust-toolchain.toml` tracks stable.
- `cargo fmt`, `cargo clippy -- -D warnings`, `cargo nextest run` must be clean.
- Errors: `thiserror` in libraries, `anyhow` in binaries.
- Logging: `tracing`; the CLI supports `--log-format json`.
- Tests: unit tests next to the code, integration tests in `tests/`, snapshots with `insta`,
  properties with `proptest`, fuzzing with `cargo-fuzz`, benchmarks with `criterion`.
- Every format parser gets a fuzz target; every new behavior gets a test.
- Architectural decisions are recorded in `docs/adr/`.
