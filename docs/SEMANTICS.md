# Ladder semantics (normative)

This document defines **what a scan does**. It is the reference for the implementation in
`softladder-core` and for the ClassicLadder compatibility work in M3.

It is derived from the *observable behaviour* of the reference implementation
(`classicladder/src/calc.c`, v0.9.113+). Per [ADR-0004](adr/0004-clean-room-policy.md) no C code is
translated: what follows is a behavioural specification, written in SoftLadder's own terms.

## 1. The rung grid

A rung is a grid of cells addressed by `(col, row)`, both zero-based, `col` growing to the right and
`row` growing downwards. There is no fixed size: the grid extends to the highest column/row used.

Each cell holds at most one [`PlacedElement`]. Two connectivity rules matter:

- **Horizontal flow is implicit.** A row that contains at least one element is *live*: an empty cell
  in a live row conducts exactly what a `Connection` cell would carry, i.e. `state_on_left` of that
  cell — including the vertical merge described below. A row with no elements at all is *inert*: it
  never conducts, so it cannot inject power into a vertical link.
  An empty cell in **column 0 is not the power rail**: only a cell that exists in column 0 touches
  the rail. That is what makes a branch that taps into the middle of a rung possible — the branch
  starts empty in column 0 and receives its power through the vertical link instead of from the
  rail. A live row whose column 0 is empty and that has no link to draw power from is unreachable
  and reported as `SL-W001`.
- **Vertical flow is explicit and per column.** An element with `connected_with_top: true` links the
  *left side* of its cell to the left side of the cell above it **in the same column** (the
  `Connection` element is the usual carrier, but the flag is a property of the cell and may sit on
  any element). Chains of such links form a vertical block. A link drawn in one column never affects
  its neighbours: place it in the column where the branches merge, so that a series element in that
  same column (a stop contact, for instance) is fed by the merged power and can still break the
  circuit.

### Divergence from ClassicLadder

ClassicLadder stores a dense 12 × 8 matrix in which every wire is an explicit `ELE_CONNECTION` cell
and a missing cell *breaks* the flow. SoftLadder derives wires from the elements that are present:

| Aspect | ClassicLadder | SoftLadder |
| --- | --- | --- |
| Grid | fixed `12 × 8` per rung | unbounded, derived from content |
| Horizontal wire | explicit `ELE_CONNECTION` cell | implicit in live rows |
| Empty cell mid-row | breaks the flow | conducts |
| Row with no elements | evaluated (breaks the flow) | inert |
| Vertical link | `ConnectedWithTop` on the cell | same flag, same column rule |

Consequences: a program that relies on a *deliberate gap* to break a row cannot be expressed
(use separate rows instead), and a ClassicLadder project whose rows are fully wired with connection
cells imports to the same behaviour. Both directions are covered by the M3 round-trip tests.

## 2. Power flow

The rung is evaluated column by column, left to right; within a column, rows top to bottom. Every
evaluated cell computes:

- `input(cell)` — the power arriving from the left (see `state_on_left`),
- `state(cell)` — the cell's own condition (e.g. the variable of a contact),
- `output(cell)` — the power leaving the cell to the right.

`state_on_left(col, row)` is the OR of the outputs of the cells immediately to the left, over the
whole vertical block that contains `(col, row)`:

```
state_on_left(col, row):
    if col == 0: return true                       # the left power rail
    result = output(col - 1, row)
    # walk upwards while each cell declares a link with the one above
    y = row
    while y > 0 and connected_with_top(col, y):
        y -= 1
        result |= output(col - 1, y)
    # walk downwards: (col, y+1) links to (col, y) when its own flag is set
    y = row + 1
    while y < row_count and connected_with_top(col, y):
        result |= output(col - 1, y)
        y += 1
    return result
```

`output(col - 1, y)` is `false` for coordinates with no cell.

Two properties follow, and both are intentional:

- A vertical link shares power **in both directions** (up and down) inside the same column, exactly
  like a soldered wire. Branches therefore merge wherever the link is drawn.
- Because evaluation is column-major, a cell never observes a write performed in the *same* column.
  Rows within a column are independent of each other, except through the previous column.

## 3. Element semantics

`output` is what the next column sees; `input` is `state_on_left(col, row)` unless stated otherwise.

### 3.1 Contacts

| Element | `state` | `output` |
| --- | --- | --- |
| `ContactNo` | `var` | `state && input` |
| `ContactNc` | `!var` | `state && input` |
| `ContactRising` | `var && !var_prev` | `state && input` |
| `ContactFalling` | `!var && var_prev` | `state && input` |

A contact whose `col == 0` is attached to the power rail: `output = state`.

`var_prev` is the value of the variable at the end of the previous scan, per element. Edge detection
is therefore *scan-based*, not time-based: a pulse shorter than one scan is not seen, and an edge is
consumed by exactly one cell.

### 3.2 Connection

| Element | `state` | `output` |
| --- | --- | --- |
| `Connection` | `input` | `input` |

Its purpose is the vertical link (`connected_with_top`); as a horizontal cell it is transparent.

### 3.3 Coils

A coil writes its variable from `input` (`%Q0`, `%M3`, …):

| Element | Write | `output` |
| --- | --- | --- |
| `CoilOut` | `var := input` | `input` |
| `CoilOutNeg` | `var := !input` | `input` |
| `CoilSet` | `if input { var := true }` | `input` |
| `CoilReset` | `if input { var := false }` | `input` |

Set/reset coils are level-triggered, not edge-triggered: they rewrite the variable on every scan
while the flow is present, and leave it untouched otherwise.

**Divergence from ClassicLadder:** the reference leaves an output cell's "output" at its previous
value, so chaining elements to the right of a coil produces stale results. SoftLadder defines
`output := input`, which makes serial coils (`-( )- -( )-`) behave as expected.

### 3.4 Compare and Operate

Both are single-cell elements in SoftLadder (ClassicLadder spreads them over three columns and taps
the flow two columns to the left; the normalization is equivalent because the intervening cells are
wires).

| Element | Behaviour |
| --- | --- |
| `Compare` | `state = eval(expr)`, `output = state && input`. `params[0]` is a full expression, or three params `<lhs> <op> <rhs>`. |
| `Operate` | `if input { write(target, eval(expr)) }`, `output = input`. `params[0]` is the target variable, the remaining params are the expression (a leading `=` is ignored). |

Evaluation never panics: a parse error, an unknown variable, a type mismatch or a division by zero
produces an `Error` diagnostic (`SL-E002`) and the element yields `false` / writes nothing.

### 3.5 IEC timer (`Timer { mode }`)

The block is a single cell at `(col, row)`; its enable is `input(col, row)`. State lives in the
timer instance `%TM<n>`: `%TM<n>.Q` (output), `%TM<n>.V` (elapsed, in time-base units),
`%TM<n>.P` (preset, in units).

`params[0]` is the preset expressed in **milliseconds**. It may be a literal, or a variable read on
every scan (also in milliseconds). A literal may carry a duration suffix, which also selects the
timer's time base:

| `params[0]` | Time base | `%TM<n>.P` for that value |
| --- | --- | --- |
| `"3000"`, `"300"`, `"100"` | 100 ms (default) | `30`, `3`, `1` |
| `"3s"`, `"30s"` | 1 s | `3`, `30` |
| `"5m"` | 60 min | `5` |

`%TM<n>.P` (preset) and `%TM<n>.V` (elapsed) are both counted in **time-base units**, matching
ClassicLadder, while `params[0]` and `%TM<n>`-as-a-duration are milliseconds. Counting accumulates
the real elapsed time and takes as many whole units as are available, carrying the remainder:

```
if counting:
    acc += delta_ms                     # milliseconds since the previous scan
    units = acc / base
    acc -= units * base
    value += units
```

| Mode | Behaviour |
| --- | --- |
| `On` (TON) | `!input` → `Q = false`, `V = 0`. `input && V < P` → count. `input && V >= P` → `Q = true`. |
| `Off` (TOF) | `input` → `Q = true`, `V = 0`, stop. On the falling edge of `input`, start counting; `V >= P` → `Q = false`, `V = 0`, stop. |
| `Pulse` (TP, not retriggerable) | Rising edge while stopped → `Q = true`, `V = 0`, start. Count while started; `V >= P` → `Q = false`, `V = 0`, stop. |

`output(cell) = Q`.

**Divergences (deliberate):**

- The reference quantizes with the *configured* logic period and adds at most one base unit per scan,
  so with a 1 s or 60 min base a timer can never expire when the scan period is 10 ms. SoftLadder
  uses the real elapsed time between scans and converts it into whole units, which keeps `%TM<n>.V`
  proportional to wall-clock time at any base. With a regular scan period the two agree.
- A huge `delta_ms` (a paused debugger, a suspended laptop) is clamped to 1000 ms so that a timer can
  never jump to its preset from a single stall.

### 3.6 Counter (`Counter { kind }`)

A counter block at `(col, row)` occupies four input rows and reads the flow arriving at each of them:

| Input | Row | Behaviour |
| --- | --- | --- |
| Reset | `row` | `value := 0` (level, not edge) |
| Preset | `row + 1` | `value := %C<n>.P` (level, not edge) |
| Count up | `row + 2` | increment on the rising edge |
| Count down | `row + 3` | decrement on the rising edge |

Order of application follows the reference: count up, count down, preset, reset (so a simultaneous
preset and reset ends at 0). The counter wraps in `0..=9999` and tracks the previous value so that
`%C<n>.E` ("empty") means *wrapped down from 0* and `%C<n>.F` ("full") means *wrapped up from 9999*.

`kind` selects which edges are honoured: `Up` (count up only), `Down` (count down only), `UpDown`
(both). `%C<n>.D` (`value == %C<n>.P`) is the done bit, and `%C<n>.V` the current value.
`params[0]` is the preset, literal or variable, like the timer.

`output(cell) = %C<n>.D`.

### 3.7 Register FIFO/LIFO (`Register { mode }`)

A register block at `(col, row)` occupies three input rows:

| Input | Row | Behaviour |
| --- | --- | --- |
| Reset | `row` | drop all stored values, `%R<n>.O := 0` |
| In | `row + 1` | on the rising edge: push `%R<n>.I` if the buffer is not full |
| Out | `row + 2` | on the rising edge: pop into `%R<n>.O` if the buffer is not empty |

`mode` selects the pop order: `Fifo` removes the oldest value, `Lifo` the newest. The buffer is a
ring of `params[0]` slots (literal or variable, default 500). Status: `%R<n>.E` (empty),
`%R<n>.F` (full), `%R<n>.S` (number of values stored). Pushing when full and popping when empty are
no-ops (not errors).

`output(cell) = %R<n>.E`.

**Divergence:** ClassicLadder exposes the block's boolean outputs as wires leaving the block's *body
cells* (one row per output). SoftLadder has a single-celled block whose only wire output is the
primary flag; the other flags are read as variables. Variable states — what the parity tests
compare — are unaffected.

### 3.8 Jump (`CoilJump`)

`if input { jump to params[0] }`. The target is resolved inside the current section:

- a parameter that parses as an integer is a **rung index** (ClassicLadder semantics),
- otherwise it is matched against the `label` of the section's rungs (SoftLadder convenience).

A jump aborts the current rung immediately: the remaining columns and rows are not evaluated. An
unknown target produces `SL-E005` and the jump is ignored. A jump executed more than 100 000 times in
one scan is treated as a mad loop (`SL-E006`), the section aborts and the runtime is asked to stop.

### 3.9 Call (`CoilCall`)

`if input { execute subroutine params[0] }`, at the position of the coil; afterwards the rung
continues with the call's output = `input`. `params[0]` is the subroutine number
(`Section::subroutine`). A missing subroutine produces `SL-E007` and is ignored. The call stack is
limited to 25 frames; exceeding it produces `SL-E008`, aborts the call and abandons the rung.

## 4. Scan structure

```
scan_once(now_ms):
    delta = now_ms - last_scan_ms
    for section in project.sections where section.language == Ladder and section.subroutine == None:
        run_section(section)
    last_scan_ms = now_ms
```

- Main sections run in the order they appear in `Project::sections`.
- Within a section, rungs run in the order of `Section::rungs`.
- Subroutine sections never run on their own; they only run from a `CoilCall`.
- Sections in `SectionLanguage::Sfc` are skipped with `SL-W002` until M4.
- An `Error` diagnostic does **not** abort the scan: it is recorded and evaluation continues. Only a
  failed jump or call aborts the current rung, and only the mad-loop guard aborts the section.

A scan is *deterministic*: it depends only on the project, the variable state and `now_ms`. Nothing
reads the clock, the filesystem or the network.

## 5. Diagnostics

| Code | Severity | Raised when |
| --- | --- | --- |
| `SL-E001` | Error | referenced variable is not present in the store |
| `SL-E002` | Error | expression parse/evaluation failure (includes division by zero) |
| `SL-E003` | Error | element/variable kind mismatch, or index out of range |
| `SL-E004` | Error | element is missing its required variable |
| `SL-E005` | Error | jump target (rung index or label) does not exist |
| `SL-E006` | Error | mad-loop jump guard tripped |
| `SL-E007` | Error | call to an undefined or non-subroutine section |
| `SL-E008` | Error | subroutine call stack overflow |
| `SL-E009` | Error | two elements placed on the same cell |
| `SL-W001` | Warning | a live row has no path to power (empty column 0 and no vertical link feeding it) |
| `SL-W002` | Warning | SFC section skipped (engine lands in M4) |

Diagnostics carry the section id and rung id when available. The runtime surfaces them to the CLI
(`softladder lint`, exit code 3 when any `Error` is present), the editor Problems panel and the
monitor protocol.

## 6. Variable store

Values live in `VarStore`, addressed by `VarRef { kind, index, index_expr, accessor }`
(see [`FORMAT.md`](FORMAT.md) §schema v2). Indexing is resolved at access time
(`%MW[%MW0]`) and the resolved index must be in range, otherwise `SL-E003`.

Function-block state (timers, counters, registers, edge history) lives in the store, indexed by the
element's variable index, so it persists across scans and is part of a recorded/replayed scan.

Capacities are dynamic: the store grows on demand and `DEFAULT_*` sizes only pre-allocate.
