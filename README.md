<div align="center">

# SoftLadder

**A modern ladder-logic / SFC PLC editor and runtime, written in Rust.**

[![CI](https://github.com/jorgeviloria/softladder/actions/workflows/ci.yml/badge.svg)](https://github.com/jorgeviloria/softladder/actions/workflows/ci.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

An independent reimplementation of [ClassicLadder](https://github.com/MaVaTi56/classicladder) —
same domain, better UX, better engineering, and your existing `.clprj` projects still work.

`Rust` · `egui` · `Linux / macOS / Windows` · `Modbus` · `SFC (Grafcet)` · `CI-testable logic`

</div>

---

## Why

ClassicLadder has been the free ladder-language implementation of choice since 2001 and is embedded
in LinuxCNC. It also shows its age: no undo/redo, no refactoring, no automated tests for the logic
you write, a rigid 12 × 8 grid per rung, an unauthenticated monitor, and variables spread across
GTK windows.

SoftLadder keeps what makes it valuable — the ladder/SFC semantics and the project format — and
rebuilds everything around it:

- **Editor** — canvas with pan/zoom, multi-selection, drag & drop, unlimited undo/redo, command
  palette, inline variable editing with autocompletion, cross-references and rename-in-project.
- **Simulation** — a built-in HMI panel (switches, buttons, LEDs, sliders) plus a time-series scope,
  so you can commission the logic before touching hardware.
- **Testing** — `.sltest` scenarios assert outputs against a scripted input timeline and run in CI.
  Your PLC program finally gets unit tests.
- **Runtime** — deterministic fixed-step scan, scan statistics and jitter measurement, flight
  recorder with exact replay of an incident, fail-safe output policy, optional real-time priority.
- **IO** — simulator, Modbus TCP/RTU master and slave (with a configurable register map), Linux
  GPIO, and a LinuxCNC HAL bridge.
- **Observability** — JSON/CBOR monitor protocol over TCP/WebSocket with tokens, TLS and roles, plus
  a small web dashboard; alarms with severities, acknowledgement and a SQLite journal.
- **Interoperability** — import and export ClassicLadder `.clp` / `.clprj` / `.clprjz` files.

## Status

**M0 — repository skeleton.** The workspace compiles, CI runs, and the domain model, scan engine,
project format and UI shell exist as a working skeleton. Feature milestones:

| Milestone | Scope |
| --- | --- |
| M0 ✅ | Repo, workspace, CI, plan, ADRs |
| M1 | Core: variables, expressions, function blocks, scan engine, diagnostics |
| M2 | egui editor MVP: edit/save rungs, undo/redo, simulation panel |
| M3 | ClassicLadder import/export + golden corpus parity |
| M4 | SFC/Grafcet model, engine and editor |
| M5 | IO: simulator scripting, Modbus TCP master/slave, then RTU |
| M6 | Online monitor, web dashboard, alarms and journal |
| M7 | Physical IO (GPIO) + LinuxCNC HAL bridge |
| M8 | `.sltest` harness, flight recorder, replay, fuzzing |
| M9 | Extras: ST/IL/FBD, PID/PWM, PLCopen XML, WASM, packaging |

See [`docs/PLAN.md`](docs/PLAN.md) (Spanish) for the full plan, milestones and acceptance criteria.

## Quick start

```bash
# prerequisites: Rust stable (see rust-toolchain.toml)

cargo test --workspace            # unit tests
cargo run -p softladder-ui        # desktop editor (early skeleton)
cargo run -p softladder-cli -- run examples/traffic_light.slprj --cycles 20
cargo run -p softladder-cli -- --help
```

Fetch the ClassicLadder example projects used by the compatibility tests (M3):

```bash
./scripts/fetch_corpus.sh
```

## Repository layout

```
crates/softladder-core       domain model, expressions, scan engine, function blocks, SFC
crates/softladder-project    .slprj format + ClassicLadder import/export
crates/softladder-runtime    scheduler, run states, statistics, flight recorder
crates/softladder-io         IoDriver trait + sim / Modbus / GPIO / HAL drivers
crates/softladder-monitor    online protocol + web dashboard server
crates/softladder-ui         egui/eframe editor (main binary)
crates/softladder-cli        headless run / lint / import / export
docs/                        plan, architecture, format, compatibility, ADRs
examples/                    sample projects and test scenarios
testdata/                    golden corpus (fetched, not vendored)
```

## Documentation

| Document | Contents |
| --- | --- |
| [`docs/PLAN.md`](docs/PLAN.md) | Vision, ClassicLadder inventory, differentiators, roadmap, risks (Spanish) |
| [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) | Crate boundaries, data flow, invariants, engineering conventions |
| [`docs/FORMAT.md`](docs/FORMAT.md) | Native `.slprj` schema and migration rules |
| [`docs/COMPAT.md`](docs/COMPAT.md) | ClassicLadder container/parts format, variable and element mapping |
| [`docs/ELEMENTS.md`](docs/ELEMENTS.md) | Element library and its serialization parameters |
| [`docs/adr/`](docs/adr) | Architecture decision records (licensing, UI stack, format, clean-room, oracle tests, monitor) |

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.

SoftLadder is an independent reimplementation. It is not derived from the ClassicLadder source
code; see [`NOTICE`](NOTICE) for attribution and [`docs/adr/0004-clean-room-policy.md`](docs/adr/0004-clean-room-policy.md)
for the policy that governs this.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in
this work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any
additional terms or conditions.
