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
summary: mode=simulated cycles=500 period_ms=10 missed=0 diagnostics=0
```

The 3000 ms timer above therefore fires after 300 simulated scans, without waiting
three seconds of wall-clock time. Ask for a machine-readable summary with `--json`;
because the run is simulated, the JSON of two identical invocations is byte-identical.

### Lint it

```console
$ cargo run -p softladder-cli -- lint examples/traffic_light.slprj
examples/traffic_light.slprj: 0 diagnostic(s), 0 error(s)
```

### Test it

`test` runs a `.sltest` scenario — an input timeline plus output assertions keyed
by scan cycle — and reports one line per failure plus a summary. The exit code is
what CI branches on (`0` green, `3` failures, `1` load errors):

```console
$ cargo run -p softladder-cli -- test examples/traffic_light.slprj examples/traffic_light.sltest
test: scenario="traffic light start/stop" project="examples/traffic_light.slprj" cycles=6 asserted=5 failures=0
```

Scenarios are hand-written JSON: variables as text (`"%I0"`), values as scalars
(`true`, `17`, `1.5`). Steps apply before their cycle's scan, assertions check
after it. See `traffic_light.sltest` (start/stop seal-in) for the shape.

### Serve it

`serve` scans on a wall-clock period while exposing the runtime twice: the monitor
TCP protocol and the dashboard HTTP page, both against the same program:

```console
$ cargo run -p softladder-cli -- serve examples/traffic_light.slprj --cycles 50
serve: monitor=127.0.0.1:4840 dashboard=127.0.0.1:8080 cycles=50 period_ms=10
serve: done cycles=50 monitor_connections=0 http_requests=0
```

Every connection speaks one JSON request per line after a `{"token":"…"}` line
(`viewer` reads, `operator` writes, `engineer` forces); bearers pass as repeated
`--token id=role`, defaulting to a local engineer bearer with a warning. The
dashboard polls `/api/status`, `/api/vars` and `/api/alarms` and posts to
`/api/set`, `/api/force`, `/api/release` and `/api/ack`:

```console
$ curl -s "http://127.0.0.1:8080/api/vars?token=local&var=%25Q0"
{"values":[{"value":{"Bit":false},"var":"%Q0"}]}
```

### Open it in the editor

```console
$ cargo run -p softladder-ui --bin softladder-editor
```

The editor loads `examples/traffic_light.slprj` relative to the current working
directory, so run it from the repository root. If the file is missing it starts
with an empty project instead.

### Importing or exporting ClassicLadder projects

`import` and `export` convert the ClassicLadder text container (parts, variables,
rungs, sections and sequential charts), validated against all 41 upstream example
projects — see [`docs/COMPAT.md`](../docs/COMPAT.md):

```console
$ cargo run -p softladder-cli -- import my_project.clprj -o my_project.slprj
$ cargo run -p softladder-cli -- export my_project.slprj -o my_project_back.clprj
```

`scripts/fetch_corpus.sh` downloads the ClassicLadder sources and example
projects that the M3 compatibility tests run against (they skip when the corpus
is missing). Behavioural differences that remain are listed in
[`testdata/known-divergences.md`](../testdata/known-divergences.md).

## `turret.slprj`

An 8-tool CNC lathe turret (tools T0–T7) with Gray-code position feedback, an
unclamp/rotate/clamp cycle, shortest-path direction, a settle timer and a
rotation watchdog:

| rung | what it does |
|------|--------------|
| 1 | Decodes Gray bits `%I2 %I1 %I0` into the binary position `%MW1`. |
| 2 | Forward distance from position to request `%IW0`, modulo 8, into `%MW2` (0 means home). |
| 3 | Off-target unclamps: `%Q2` while `%MW2` is not zero. |
| 4 | Runs the motor `%Q0` only unclamped (`%I3` open), off-target and fault-free. |
| 5 | Picks the direction `%Q1`: forward up to 4 steps away, reverse beyond that. |
| 6 | TON `%TM0` waits 300 ms on target before the change counts. |
| 7 | Settled and clamped: the tool-ready lamp `%Q3`. |
| 8–9 | Watchdog: `%TM1` trips `%Q4` when a rotation takes longer than 10 s, which stops the motor. |

Play it on the bench: set the *requested tool* slider, open the *clamped*
switch, and flip the three Gray switches to "turn" the turret — the motor,
direction and done lamps follow. `turret.sltest` scripts the whole loop
(to T3 forward, back to T1 in reverse) with 15 assertions:

```console
$ cargo run -p softladder-cli -- test examples/turret.slprj examples/turret.sltest
test: scenario="turret tool change there and back" project="examples/turret.slprj" cycles=130 asserted=15 failures=0
```

Toward LinuxCNC: every variable already uses IEC naming (`%I` sensors, `%Q`
actuators), so the M7 HAL bridge will map them 1:1 onto HAL pins; until then,
commission the cycle here in simulation.

## `turret_hydraulic.slprj`

A hydraulic 8-station lathe turret (tools T1–T8) turning forward only, with one
sensor per job: index at station 1 (`%I0`), a passing pulse per station (`%I1`),
arrival at the requested tool (`%I2`) and unclamped feedback (`%I3`):

| rung | what it does |
|------|--------------|
| 1 | CTU `%C0` counts passing pulses; `%I0` holds reset while parked at station 1, and the unused `LD`/`CD` rows are tied off with `spare_off`. |
| 2 | Position `%MW1` is the count modulo 8, plus one (T1–T8). |
| 3 | Forward distance to `%IW0`, modulo 8, into `%MW2`. |
| 4 | Off-target unclamps: `%Q0`. |
| 5 | Runs the hydraulics `%Q1` only unclamped, before arrival and fault-free. |
| 6–7 | Settled, arrived and clamped after 300 ms: the tool-ready lamp `%Q2`. |
| 8–9 | Watchdog: `%TM1` trips `%Q3` when a rotation takes longer than 10 s. |

Stations are T1–T8 everywhere it matters (`%MW1`): the counter itself counts pulses
since index, so reset law holds it at 0 while parked and `%MW1` still reads 1 —
there is no position 0.

Play it on the bench: request a tool on the slider, open *unclamped*, drop the
index, pulse *passing* once per station and raise *at_target* on arrival —
then close *unclamped* and watch *ready*. A full turn wraps station 8 back to 1 on its
own: the 8th pulse reads done with position 1, then the index clears the count.
`turret_hydraulic.sltest` scripts a change to T4 plus the full turn home with 23 assertions:

```console
$ cargo run -p softladder-cli -- test examples/turret_hydraulic.slprj examples/turret_hydraulic.sltest
test: scenario="hydraulic turret changes with full-turn wrap" project="examples/turret_hydraulic.slprj" cycles=135 asserted=23 failures=0
```

## Writing your own

A project is plain, diff-friendly JSON; see [`docs/FORMAT.md`](../docs/FORMAT.md)
for the schema and [`docs/ELEMENTS.md`](../docs/ELEMENTS.md) for every element and
its `params`.
