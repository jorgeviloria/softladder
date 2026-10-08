# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
