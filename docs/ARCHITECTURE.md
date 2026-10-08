# SoftLadder — Architecture

> Companion to [`PLAN.md`](PLAN.md) (Spanish, the working plan). This document is the technical
> reference for contributors: crate boundaries, invariants and data flow.

## 1. Crate map

| Crate | Responsibility | Depends on | `unsafe` |
| --- | --- | --- | --- |
| `softladder-core` | Domain model, variable namespace, expression evaluation, scan engine, function blocks, SFC, diagnostics | nothing (only `serde`, `thiserror`) | forbidden |
| `softladder-project` | Native `.slprj` format, migrations, ClassicLadder `.clp/.clprj/.clprjz` import/export, PLCopen XML (later) | `core` | forbidden |
| `softladder-runtime` | Scan scheduler, run-state machine, hot reload, scan statistics, flight recorder/replay, alarms | `core`, `project` | forbidden |
| `softladder-io` | `IoDriver` trait and drivers: `sim`, `modbus`, `gpio`, `hal` | `core` | forbidden (FFI lives in dedicated `-sys` crates) |
| `softladder-monitor` | Online monitor protocol + web dashboard server | `core` | forbidden |
| `softladder-ui` | egui/eframe editor (main binary) | `core`, `project`, `runtime` | forbidden |
| `softladder-cli` | Headless binary: `run`, `lint`, `test`, `import`, `export` | `core`, `project`, `runtime`, `io` | forbidden |

**Invariant:** dependencies only point right-to-left in the table above. `core` must never grow a
dependency on IO, UI, networking, or the system clock.

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
| `%TM` | IEC timer instance — `%TM3` is the done bit, `%TM3.V` its current value | `Bit`/`Word` | `%TM` |
| `%C` | Counter instance — `%C1` is the done bit, `%C1.V` its current value | `Bit`/`Word` | `%C` |
| `%R` | FIFO/LIFO register | `Word` | `%R` |
| `%X` | SFC step activity / step timer | `Bit`/`Word` | `%X` |
| `%S` | System variables (clock, PLC state, scan time) | both | `%S` |
| `%QLED` | User LED (physical and on-screen) | `Bit` | `%QLED` |

Indexing and bit access are first-class: `%MW[%MW0]` (index by variable) and `%MW20.3`
(bit extraction / insertion in a word) — two items from the original ClassicLadder TODO list.
`Display` always renders the canonical modern mnemonic, so parsing `%B0` and printing it yields
`%M0`.

## 5. Expression engine

`softladder-core::expr` is a hand-written tokenizer + Pratt parser. No `eval`, no `unsafe`, no
recursion without a depth limit. The AST is evaluated against a `VarSource` trait, which keeps it
decoupled from storage and trivially mockable in tests.

Supported (M1): integer/real literals, variable references, `+ - * / %`, `= <> < <= > >=`,
`AND OR XOR NOT`, parentheses, and explicit casts. Function library (`ABS MIN MAX LIMIT SEL MUX
SCALE NORM SIN COS SQRT`) lands with M9 extras; the parser is designed to accept them from the start.

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

Editing is command-based (`Command` trait with `apply`/`revert`) which gives undo/redo for free and
makes every edit testable without a UI. The UI holds no authoritative state: it renders `Project` +
`Runtime` and emits `Command`s.

## 10. Engineering conventions

- MSRV pinned in `Cargo.toml`; `rust-toolchain.toml` tracks stable.
- `cargo fmt`, `cargo clippy -- -D warnings`, `cargo nextest run` must be clean.
- Errors: `thiserror` in libraries, `anyhow` in binaries.
- Logging: `tracing`; the CLI supports `--log-format json`.
- Tests: unit tests next to the code, integration tests in `tests/`, snapshots with `insta`,
  properties with `proptest`, fuzzing with `cargo-fuzz`, benchmarks with `criterion`.
- Every format parser gets a fuzz target; every new behavior gets a test.
- Architectural decisions are recorded in `docs/adr/`.
