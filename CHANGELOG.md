# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
