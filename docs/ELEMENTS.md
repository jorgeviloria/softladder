# Element library

Reference for the ladder elements SoftLadder supports, their serialization (`ElementKind` and
`params`, see [`FORMAT.md`](FORMAT.md)) and the milestone that delivers them.

`params` is a `Vec<String>`; the tables below give the meaning of each position. Numeric parameters
accept either a literal (`"3000"`) or a variable reference (`"%MW5"`), resolved at scan time.

## Contacts (inputs)

| Element | `kind` | `params` | Milestone |
| --- | --- | --- | --- |
| Normally open `-[ ]-` | `"ContactNo"` | — | M0 |
| Normally closed `-[/]-` | `"ContactNc"` | — | M0 |
| Rising edge `-[P]-` | `"ContactRising"` | — | M0 |
| Falling edge `-[N]-` | `"ContactFalling"` | — | M0 |
| Compare | `"Compare"` | `["<expr>"]` or `["<lhs>", "<op>", "<rhs>"]` | M0 |
| Wire / vertical link | `"Connection"` | — | skeleton (full power flow in M1) |

`<op>` is one of `=`, `<>`, `<`, `<=`, `>`, `>=`.

## Coils (outputs)

| Element | `kind` | `params` | Milestone |
| --- | --- | --- | --- |
| Output `-( )-` | `"CoilOut"` | — | M0 |
| Negated output `-(/)-` | `"CoilOutNeg"` | — | M0 |
| Set (latch) `-(S)-` | `"CoilSet"` | — | M0 |
| Reset (unlatch) `-(R)-` | `"CoilReset"` | — | M0 |
| Jump `-(J)-` | `"CoilJump"` | `["<rung label>"]` | M1 (TODO) |
| Subroutine call `-(C)-` | `"CoilCall"` | `["<subroutine number>"]` | M1 (TODO) |
| Operate (assignment) | `"Operate"` | `["<target var>", "<expr tokens…>"]`, e.g. `["%MW0", "=", "%MW0 + 1"]` | M0 |

The operate block joins everything after the target with spaces and strips a leading `=`, so both
`["%MW0", "=", "%MW0 + 1"]` and `["%MW0", "%MW0 + 1"]` are valid.

## Function blocks

The block's variable must match the block type (`%TM…` for timers, `%C…` for counters, `%R…` for
registers); a mismatch is reported as `SL-E003` and the block passes its input flow through
unchanged.

| Element | `kind` | `params` | Milestone |
| --- | --- | --- | --- |
| IEC timer (TON/TOF/TP) | `{"Timer": {"mode": "On" \| "Off" \| "Pulse"}}` | `["<preset ms>"]` | M0 |
| Counter (CTU/CTD/CTUD) | `{"Counter": {"kind": "Up" \| "Down" \| "UpDown"}}` | `["<preset>"]` | M0 (reset/load inputs in M1) |
| Register (FIFO/LIFO) | `{"Register": {"mode": "Fifo" \| "Lifo"}}` | `["<max values>"]` | M1 (TODO) |
| Bistable RS / SR | `"Bistable"` | `{"kind": "Rs" \| "Sr"}` | M2 |
| Pulse / PWM | `"Pwm"` | `["<period ms>"]` (duty is an input) | M9 |
| PID | `"Pid"` | `["kp", "ki", "kd", "<period ms>"]` | M9 |
| Scale / normalize | `"Scale"` | `["<in min>", "<in max>", "<out min>", "<out max>"]` | M9 |
| Selection | `"Select"` | `{"kind": "Sel" \| "Mux" \| "Limit"}` | M9 |
| RTC | `"Rtc"` | — | M9 |

Timer and counter blocks read their preset on every scan, so a preset that points at a variable is
live-editable from the HMI panel.

## Sequential (SFC)

Steps, transitions, divergences/convergences (AND/OR) and IEC action qualifiers (`N, S, R, L, D, P,
SD, DS, SL`). The model exists in `softladder-core::sfc`; the engine and editor land in M4, and until
then the scan engine reports `SL-W002` and skips SFC sections. See
[`ARCHITECTURE.md`](ARCHITECTURE.md) §6.

## Diagnostics vocabulary

Emitted by the scan engine and surfaced in the editor's Problems panel:

| Code | Severity | Meaning |
| --- | --- | --- |
| `SL-E001` | Error | Referenced variable is not defined in the store |
| `SL-E002` | Error | Expression could not be parsed or evaluated (includes divide-by-zero) |
| `SL-E003` | Error | Element/variable mismatch or index out of range |
| `SL-E004` | Error | Element is missing its required variable |
| `SL-W002` | Warning | SFC section skipped (engine lands in M4) |

A rung with an `Error` diagnostic is marked in the editor and reported by `softladder lint`
(exit code 3); the rest of the scan continues.

## Legacy elements on import

The deprecated ClassicLadder `ELE_TIMER` / `ELE_MONOSTABLE` elements are imported as `Timer` blocks
with a `Warning` diagnostic, preserving preset, base and current value where possible. They are not
offered in the editor palette.

## Not planned

- Anything requiring a proprietary runtime license or closed protocol reverse engineering.
- Safety-rated blocks (SIL/PL) — out of scope by design.
