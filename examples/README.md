# SoftLadder examples

## `traffic_light.slprj`

A four-rung project that exercises the pieces of the M1 scan engine:

| rung | what it does |
|------|--------------|
| 1 | `%I0` (start) latches `%Q0` (green lamp) through its own contact, `%I2` (stop) breaks the seal. The two rows are **parallel branches** merged at column 1 by a `Connection` with `connected_with_top` — see [`docs/SEMANTICS.md`](../docs/SEMANTICS.md) §2. |
| 2 | A TON timer `%TM0` with a 3000 ms preset follows `%Q0` and drives `%Q1` (amber lamp). |
| 3 | A CTU counter `%C0` with a preset of 5 (`params[0]`, exposed as `%C0.P`) counts rising edges of `%I1` and raises its done bit `%C0.D`. |
| 4 | On the rising edge of `%C0.D` a `CoilSet` latches `%M0`; while `%M0` is set an `Operate` block copies the current count `%C0.V` into `%MW0`. |

Rung 1 in grid form — column 1 is the merge column, and the vertical link is the
`connected_with_top` flag of the `Connection` at `(col 1, row 1)`:

```
        col 0        col 1        col 2
row 0  -[ %I0 ]--+--[ /%I2 ]-------------( %Q0 )-
                 |
row 1  -[ %Q0 ]--+
```

The scan configuration is 10 ms per scan. The project is a normal `softladder-project`
file, so the tests load it, re-serialize it and run it through the scan engine
(`crates/softladder-project/tests/traffic_light.rs`).

### Run it with the CLI

`run` uses **simulated** time by default, so the same command always produces the
same output (add `--real-time` to pace with the wall clock instead):

```console
$ cargo run -p softladder-cli -- run examples/traffic_light.slprj --cycles 500
cycle      1  now=     0ms  state=Run  diagnostics=0
cycle      2  now=    10ms  state=Run  diagnostics=0
…
summary: cycles=500 period=10ms simulated=true max_scan_ms=0.012 missed=0 diagnostics=0
```

The 3000 ms timer above therefore fires after 300 simulated scans, without waiting
three seconds of wall-clock time. Ask for a machine-readable summary with `--json`;
because the run is simulated, the JSON of two identical invocations is byte-identical.

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

## Writing your own

A project is plain, diff-friendly JSON; see [`docs/FORMAT.md`](../docs/FORMAT.md)
for the schema and [`docs/ELEMENTS.md`](../docs/ELEMENTS.md) for every element and
its `params`.
