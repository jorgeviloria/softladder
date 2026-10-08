# Element library

Reference for the ladder elements SoftLadder supports, their serialization (`ElementKind`,
`var` and `params`, see [`FORMAT.md`](FORMAT.md)) and their runtime behaviour (normative details in
[`SEMANTICS.md`](SEMANTICS.md)).

`params` is a `Vec<String>`; the tables below give the meaning of each position. Numeric parameters
accept either a literal (`"3000"`) or a variable reference (`"%MW5"`), resolved at scan time.

Every element lives on **one cell** (`col`, `row`). Function blocks additionally read the flow
arriving at the rows below them, which is how their inputs are wired in ladder form.

## Contacts (inputs)

| Element | `kind` | `var` | `params` | Milestone |
| --- | --- | --- | --- | --- |
| Normally open `-[ ]-` | `"ContactNo"` | bit, or `.n` of a word | — | M0 |
| Normally closed `-[/]-` | `"ContactNc"` | bit, or `.n` of a word | — | M0 |
| Rising edge `-[P]-` | `"ContactRising"` | bit, or `.n` of a word | — | M0 |
| Falling edge `-[N]-` | `"ContactFalling"` | bit, or `.n` of a word | — | M0 |
| Compare | `"Compare"` | — | `["<expr>"]` or `["<lhs>", "<op>", "<rhs>"]` | M0 |
| Wire / vertical link | `"Connection"` | — | — | M0 |

`<op>` is one of `=`, `<>`, `<`, `<=`, `>`, `>=`. A `Connection` is transparent horizontally; its
purpose is `connected_with_top`, which merges the row with the one above it (parallel branches).

## Coils (outputs)

| Element | `kind` | `params` | Milestone |
| --- | --- | --- | --- |
| Output `-( )-` | `"CoilOut"` | — | M0 |
| Negated output `-(/)-` | `"CoilOutNeg"` | — | M0 |
| Set (latch) `-(S)-` | `"CoilSet"` | — | M0 |
| Reset (unlatch) `-(R)-` | `"CoilReset"` | — | M0 |
| Jump `-(J)-` | `"CoilJump"` | `["<rung label>"]` or `["<rung index>"]` | M1 |
| Subroutine call `-(C)-` | `"CoilCall"` | `["<subroutine number>"]` | M1 |
| Operate (assignment) | `"Operate"` | `["<target var>", "<expr tokens…>"]`, e.g. `["%MW0", "=", "%MW0 + 1"]` | M0 |

Coils drive their `var` from the rung flow: `CoilOut`/`CoilOutNeg` on every scan, `CoilSet` and
`CoilReset` only while the flow is present (level-triggered, not edge-triggered). The operate block
joins everything after the target with spaces and strips a leading `=`, so both
`["%MW0", "=", "%MW0 + 1"]` and `["%MW0", "%MW0 + 1"]` are valid; it writes only when the rung is
live.

A jump resolves to a rung index when its parameter parses as an integer (ClassicLadder semantics) and
otherwise to a rung `label` in the same section. A jump aborts the rest of its rung immediately.

## Function blocks

The block's `var` selects the instance and must match the block type (`%TM…`, `%C…`, `%R…`); a
mismatch is reported as `SL-E003` and the block passes its input flow through unchanged. Input rows
are counted from the block's own `row`.

| Element | `kind` | `params` | Input rows | Accessors |
| --- | --- | --- | --- | --- |
| IEC timer (TON/TOF/TP) | `{"Timer": {"mode": "On" \| "Off" \| "Pulse"}}` | `["<preset ms>"]` | `row` = enable | `%TM<n>.Q` output (bare `%TM<n>`), `.V` elapsed, `.P` preset |
| Counter (CTU/CTD/CTUD) | `{"Counter": {"kind": "Up" \| "Down" \| "UpDown"}}` | `["<preset>"]` | `row` = reset, `row+1` = preset, `row+2` = count up, `row+3` = count down | `%C<n>.D` done (bare), `.V` value, `.P` preset, `.E` wrapped down, `.F` wrapped up |
| Register (FIFO/LIFO) | `{"Register": {"mode": "Fifo" \| "Lifo"}}` | `["<capacity>"]` | `row` = reset, `row+1` = in, `row+2` = out | `%R<n>.E` empty (bare), `.F` full, `.I` input value, `.O` output value, `.S` stored count |
| Bistable RS / SR | `"Bistable"` | `{"kind": "Rs" \| "Sr"}` | — | M2 |
| Pulse / PWM | `"Pwm"` | `["<period ms>"]` (duty is an input) | M9 |
| PID | `"Pid"` | `["kp", "ki", "kd", "<period ms>"]` | M9 |
| Scale / normalize | `"Scale"` | `["<in min>", "<in max>", "<out min>", "<out max>"]` | M9 |
| Selection | `"Select"` | `{"kind": "Sel" \| "Mux" \| "Limit"}` | M9 |
| RTC | `"Rtc"` | — | M9 |

Presets are live: a timer or counter stores its preset at `%TM<n>.P` / `%C<n>.P`, so an HMI can
change it while the program runs, and a literal `params[0]` is written there.

A timer's `params[0]` is a duration in **milliseconds** and may carry a suffix that also picks the
time base (`"300"` → 100 ms base, `"3s"` → 1 s base, `"5m"` → 60 min base), while `%TM<n>.P` and
`%TM<n>.V` are counted in time-base units. A counter's preset and value are plain counts. See
[`SEMANTICS.md`](SEMANTICS.md) §3.5.

The block's **wire output** (what the next column sees) is its primary flag: `Q` for a timer, `D`
for a counter, `E` (empty) for a register.

## Sequential (SFC)

Steps, transitions, divergences/convergences (AND/OR) and IEC action qualifiers (`N, S, R, L, D, P,
SD, DS, SL`). The model exists in `softladder-core::sfc`; the engine and editor land in M4, and until
then the scan engine reports `SL-W002` and skips SFC sections. See
[`ARCHITECTURE.md`](ARCHITECTURE.md) §6.

## Diagnostics vocabulary

Emitted by the scan engine and `lint`, surfaced in the editor's Problems panel and in
`softladder lint` (exit code 3 when any `Error` is present):

| Code | Severity | Meaning |
| --- | --- | --- |
| `SL-E001` | Error | Referenced variable is not defined in the store |
| `SL-E002` | Error | Expression could not be parsed or evaluated (includes divide-by-zero) |
| `SL-E003` | Error | Element/variable mismatch or index out of range |
| `SL-E004` | Error | Element is missing its required variable |
| `SL-E005` | Error | Jump target (rung index or label) does not exist |
| `SL-E006` | Error | Mad-loop jump guard tripped; the runtime is asked to stop |
| `SL-E007` | Error | Call to an undefined section, or to one that is not a subroutine |
| `SL-E008` | Error | Subroutine call stack overflow |
| `SL-E009` | Error | Two elements placed on the same cell |
| `SL-W001` | Warning | A live row cannot reach the left rail (unreachable branch) |
| `SL-W002` | Warning | SFC section skipped (engine lands in M4) |
| `SL-E010` | Error | A rung or section id is used twice |
| `SL-W010` | Warning | The project has no rungs |
| `SL-W011` | Warning | A rung is empty |

`SL-E001`–`SL-E009` and `SL-W001`/`SL-W002` come from `softladder_core::lint` and the scan engine;
`SL-E010`, `SL-W010` and `SL-W011` are project-level checks reported by `softladder lint`. A rung
with an `Error` diagnostic is marked in the editor; the rest of the scan continues.

## Legacy elements on import

The deprecated ClassicLadder `ELE_TIMER` / `ELE_MONOSTABLE` elements are imported as `Timer` blocks
with a `Warning` diagnostic, preserving preset, time base and current value where possible. They are
not offered in the editor palette.

## Not planned

- Anything requiring a proprietary runtime license or closed protocol reverse engineering.
- Safety-rated blocks (SIL/PL) — out of scope by design.
