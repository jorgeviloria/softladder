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

- **Editor** — a ladder document on paper, the way industrial tools draw one: numbered network
  headers with title, comment and state badge; tag names over addresses; real IEC symbols including
  timer and counter boxes with named pins; live power flow in green with inline values; a grouped
  instruction palette; a project tree; a context-sensitive inspector; and unlimited command-based
  undo/redo. See [`docs/UX.md`](docs/UX.md).
- **Simulation bench** — a panel of switches, push-buttons, lamps, sliders and gauges that is part of
  the project, so you can commission the logic before touching hardware. The program's `%I`/`%Q`
  variables are wired to the bench automatically.
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

**M3 complete — your ClassicLadder projects work.** What you can do today:

- **import and export ClassicLadder projects** (`.clp`, `.clprj`, `.clprjz`), validated against all
  41 upstream example projects: every one imports, `import → export → import` is a fixed point, and
  the parts SoftLadder does not model (Modbus and serial configuration, IO mapping, alarm slots, …)
  survive byte for byte. What cannot be represented is reported with a located diagnostic, never
  dropped silently ([`docs/COMPAT.md`](docs/COMPAT.md), [`testdata/known-divergences.md`](testdata/known-divergences.md));

- open, edit and save ladder programs in the desktop editor, with unlimited undo/redo, an element
  palette, variable validation, problems list and live power-flow indication
  ([`docs/EDITOR.md`](docs/EDITOR.md));
- run them on a simulation bench of switches, push-buttons, lamps, sliders and gauges that is saved
  with the project, so logic can be commissioned before any hardware exists;
- execute the same program headless and reproducibly (`softladder run` is deterministic by default);
- rely on a deterministic scan engine with the full element set, jumps/subroutines and structured
  diagnostics ([`docs/SEMANTICS.md`](docs/SEMANTICS.md));
- keep programs in `.slprj` schema v2 — the ClassicLadder sub-value spellings (`%TM0.Q`, `%R0.I`,
  `%MW0.3`) included — with a migration chain from v1.

| Milestone | Scope |
| --- | --- |
| M0 ✅ | Repo, workspace, CI, plan, ADRs |
| M1 ✅ | Core: variable accessors, expressions, function blocks, real power flow, diagnostics |
| M2 ✅ | egui editor: element palette, undo/redo, simulation bench, problems, live power flow |
| M3 ✅ | ClassicLadder import/export + golden-corpus round trip |
| M4 | SFC/Grafcet model, engine and editor |
| M5 | IO: simulator scripting, Modbus TCP master/slave, then RTU |
| M6 | Online monitor, web dashboard, alarms and journal |
| M7 | Physical IO (GPIO) + LinuxCNC HAL bridge |
| M8 | `.sltest` harness, flight recorder, replay, fuzzing |
| M9 | Extras: ST/IL/FBD, PID/PWM, PLCopen XML, WASM, packaging |

See [`docs/PLAN.md`](docs/PLAN.md) (Spanish) for the full plan, milestones and acceptance criteria.

## How the interface is built

The editor is designed against the tools people actually program PLCs with — TIA Portal, Studio 5000,
CODESYS, TwinCAT, GX Works, Sysmac Studio, Control Expert — and [`docs/UX.md`](docs/UX.md) records
what that means concretely: the shell (ribbon, project tree, document tabs, inspector, status bar),
the design tokens (light "paper" theme, spacing scale, type scale, icon set), the ladder conventions
(networks, symbolic tags, live state) and the panels.

There is no display server in CI, so the interface is reviewed through **headless screenshots**:
`crates/softladder-ui/tests/ui_shots.rs` runs the real `EditorApp::draw` on an `egui::Context` and
rasterises the tessellated frame and the font atlas into PNGs under `target/ui-shots/` — no GPU and
no window, and nothing added to the shipped binary.

```bash
cargo test -p softladder-ui --test ui_shots    # writes target/ui-shots/*.png
```

## Quick start

```bash
# prerequisites: Rust stable (see rust-toolchain.toml)

cargo test --workspace            # unit tests (330+ tests)
cargo run -p softladder-ui        # desktop editor (run it from the repo root)
cargo run -p softladder-cli -- run examples/traffic_light.slprj --cycles 500   # deterministic
cargo run -p softladder-cli -- run examples/traffic_light.slprj --real-time    # wall-clock pacing
cargo run -p softladder-cli -- lint examples/traffic_light.slprj
cargo run -p softladder-cli -- import my_project.clprj -o my_project.slprj
cargo run -p softladder-cli -- export my_project.slprj -o my_project.clprj
cargo run -p softladder-cli -- --help
```

Runs are simulated by default: `now_ms` advances by exactly the scan period, so the same command
produces byte-identical `--json` output. `--real-time` paces with the wall clock instead.

Fetch the ClassicLadder example projects used by the compatibility tests (M3):

```bash
./scripts/fetch_corpus.sh
```

## Repository layout

```
crates/softladder-core       domain model, expressions, scan engine, function blocks, SFC
crates/softladder-project    .slprj format + ClassicLadder import/export
crates/softladder-runtime    scheduler, run states, statistics, flight recorder
crates/softladder-edit       editing commands, undo/redo history and the simulation bench
crates/softladder-io         IoDriver trait + sim / Modbus / GPIO / HAL drivers
crates/softladder-monitor    online protocol + web dashboard server
crates/softladder-ui         egui/eframe editor (main binary)
crates/softladder-cli        headless run / lint / import / export
docs/                        plan, semantics, architecture, format, compatibility, ADRs
examples/                    sample projects and test scenarios
testdata/                    golden corpus (fetched, not vendored)
```

## Documentation

| Document | Contents |
| --- | --- |
| [`docs/PLAN.md`](docs/PLAN.md) | Vision, ClassicLadder inventory, differentiators, roadmap, risks (Spanish) |
| [`docs/SEMANTICS.md`](docs/SEMANTICS.md) | Normative specification of ladder execution: power flow, every element, jumps/calls, diagnostics |
| [`docs/UX.md`](docs/UX.md) | Interface design: vendor conventions, shell, design tokens, ladder editor, panels, accessibility |
| [`docs/EDITOR.md`](docs/EDITOR.md) | Editor behaviour contract: shortcuts, placement, simulation bench, live indication |
| [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) | Crate boundaries, data flow, invariants, engineering conventions |
| [`docs/FORMAT.md`](docs/FORMAT.md) | Native `.slprj` schema and migration rules |
| [`docs/COMPAT.md`](docs/COMPAT.md) | ClassicLadder container/parts format, variable and element mapping |
| [`docs/ELEMENTS.md`](docs/ELEMENTS.md) | Element library, its serialization parameters and every diagnostic code |
| [`testdata/known-divergences.md`](testdata/known-divergences.md) | Where an imported project behaves differently from ClassicLadder, and why |
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
