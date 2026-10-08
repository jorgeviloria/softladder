# SoftLadder examples

## `traffic_light.slprj`

A minimal three-rung project that exercises the pieces of the M0 scan engine:

| rung | what it does |
|------|--------------|
| 1 | `%I0` (start button) latches `%Q0` (green lamp) through its own contact — a classic self-holding circuit with a parallel branch. |
| 2 | A TON timer `%TM0` with a 3000 ms preset follows `%Q0` and drives `%Q1` (amber lamp). |
| 3 | A CTU counter `%C0` with a preset of 5 counts `%I1`, mirrors its done bit on `%M0`, and an `Operate` block bumps `%MW0` on every completed batch. |

The scan configuration is 10 ms per scan. The project is a normal
`softladder-project` file, so the tests load it, re-serialize it and run it
through the scan engine (`crates/softladder-project/tests/traffic_light.rs`).

### Run it with the CLI

```console
$ cargo run -p softladder-cli -- run examples/traffic_light.slprj --cycles 500
cycle      1  now=     0ms  state=Run  diagnostics=0
cycle      2  now=    10ms  state=Run  diagnostics=0
…
summary: cycles=500 max_scan_ms=0.012 missed=0 diagnostics=0
```

`run` uses the real elapsed time of the run as the simulated clock and paces the
loop with `--period-ms` (default 10 ms), so the 3000 ms timer above needs about
three seconds of wall-clock time to fire. Ask for a machine readable summary
with `--json`:

```console
$ cargo run -p softladder-cli -- run examples/traffic_light.slprj --cycles 50 --json
```

### Lint it

```console
$ cargo run -p softladder-cli -- lint examples/traffic_light.slprj
examples/traffic_light.slprj: 0 diagnostic(s), 0 error(s)
```

### Open it in the editor

```console
$ cargo run -p softladder-ui --bin softladder-editor
```

The editor loads `examples/traffic_light.slprj` relative to the current working
directory, so run it from the repository root. If the file is missing it starts
with an empty project instead.

### Importing or exporting ClassicLadder projects

`import` and `export` parse and write the ClassicLadder text container today,
but the element-level mapping is a stub that reports `M3`:

```console
$ cargo run -p softladder-cli -- import my_project.clprj -o my_project.slprj
error: importing ClassicLadder projects is not implemented yet (planned for M3)…
```

`scripts/fetch_corpus.sh` downloads the ClassicLadder sources and example
projects that the M3 compatibility tests will use.
