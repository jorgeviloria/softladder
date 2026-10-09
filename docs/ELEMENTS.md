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

## Function block pins

A block reads **one row per input pin**, and `ElementKind::input_pins` is the single source of truth
for how many that is and what each row means — the engine's block span and the editor's drawing both
come from it, so the canvas can never advertise a wire the engine does not read. A parameter is not a
pin: a preset is data (`%TM0.P`), drawn beside the instance name (`%TM0 · PT 3000`) and edited in the
inspector.

| Block | Input pins (row 0 first) | What each row does | Readouts |
| --- | --- | --- | --- |
| Timer (TON/TOF/TP) | `IN` | the enable; the preset is the `PT` parameter | `Q` (flow out), `ET` |
| Counter (CTU/CTD/CTUD) | `R`, `LD`, `CU`, `CD` | reset, load the `PV` preset, count up, count down | `Q` (flow out), `CV` |
| Register (FIFO/LIFO) | `R`, `IN`, `OUT` | reset, push, pop | `E`, `F` |
| Compare, Operate | — (no pin is drawn) | reads the row it sits on | the result, as the block text |

## Sequential (SFC)

A chart lives in a `SectionLanguage::Sfc` section as a [`SequentialPage`]: a comment, **steps**
(number, initial flag, position) and **transitions** (an optional condition expression, the steps
that must all be active for it to fire — an AND convergence — and the steps it activates together —
an AND divergence). Step activity and elapsed time are published as `%X<n>.A` and `%X<n>.V`.

The engine runs a chart once per scan with the evolution rule of [`SEMANTICS.md`](SEMANTICS.md) §4,
and `import`/`export` map ClassicLadder's `sequential.csv` (see [`COMPAT.md`](COMPAT.md) §8.4).
Diagnostics: `SL-W002` (a chart with no page, or a transition naming a step outside its page),
`SL-W011` (a transition with no condition) and `SL-E011` (a transition whose step does not exist).
The SFC **editor** is implemented — the sequential document, its palette, its tree rows, its
inspector and its diagnostics — and is described in [`UX.md`](UX.md) §12.

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
| `SL-E011` | Error | A section references a rung id that does not exist; a chart transition names a step that does not exist |
| `SL-W011` | Warning | A rung is empty; a chart transition has no condition |
| `SL-W001` | Warning | A live row cannot reach power; a non-initial chart step is never activated |
| `SL-W001` | Warning | A live row cannot reach the left rail (unreachable branch) |
| `SL-W002` | Warning | SFC section skipped (engine lands in M4) |
| `SL-E010` | Error | A rung or section id is used twice |
| `SL-W010` | Warning | The project has no rungs |
| `SL-W020` | Warning | A simulation-bench widget addresses the wrong variable kind, or has an inverted range |
| `SL-E030` | Error | A ClassicLadder document is malformed (bad container, unusable numbers, unparsable part) |
| `SL-W030` | Warning | A ClassicLadder element or structure has no SoftLadder equivalent and was skipped or approximated |
| `SL-W031` | Warning | A ClassicLadder variable family is not modelled (`%SW<n>`, deprecated timers/monostables) or an expression could not be translated |
| `SL-W032` | Warning | A project part is passed through because SoftLadder does not model it |
| `SL-W033` | Warning | A SoftLadder feature has no ClassicLadder equivalent and was not exported |

`SL-E001`–`SL-E009` and `SL-W001`/`SL-W002` come from `softladder_core::lint` and the scan engine;
`SL-E010`, `SL-W010` and `SL-W011` are project-level checks reported by `softladder lint`; and
`SL-W020` is produced by `softladder-edit` from `SimulationPanel::validate`. A rung
with an `Error` diagnostic is marked in the editor; the rest of the scan continues.

## Legacy elements on import

The deprecated ClassicLadder `ELE_TIMER` / `ELE_MONOSTABLE` elements are imported as `Timer` blocks
with a `Warning` diagnostic, preserving preset, time base and current value where possible. They are
not offered in the editor palette.

## Not planned

- Anything requiring a proprietary runtime license or closed protocol reverse engineering.
- Safety-rated blocks (SIL/PL) — out of scope by design.
