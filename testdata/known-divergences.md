# Known divergences from ClassicLadder

Behavioural differences between SoftLadder and the reference implementation, for projects that come
from `projects_examples/` (the compatibility corpus). The *format* round-trips exactly — see
[`docs/COMPAT.md`](../docs/COMPAT.md) §8.7 — so these are cases where the same file means something
slightly different when it runs.

Every entry states what the reference does, what SoftLadder does, and why we chose that. Entries are
removed when a milestone closes the gap.

## 1. The flag that leaves a block's output cell

**Reference:** a counter writes its `empty` flag to the wire leaving its own cell (row `y`), `done`
one row below and `full` two rows below; a register writes `empty` on its own row and `full` below.

**SoftLadder:** a block occupies one cell whose wire output is its primary flag — `Q` for a timer,
`done` for a counter, `empty` for a register — and the other flags are read as variables
(`%C0.E`, `%C0.F`, `%R0.F`, …).

**Why:** the variable states, which is what the parity tests compare, are identical; only a rung that
chains off one of the *secondary* rows of a block differs. Modelling four output rows per block would
put the reference's body-cell geometry back into the model for no gain, since the editor shows every
flag as a variable.

**Affects:** imported rungs that chain an element after a counter's row `y+1` or `y+2`.

## 2. Rotate and shift side effects

**Reference:** `SHL`, `SHR`, `ROL` and `ROR` write the bit that fell off the end into the system bit
`%S8` as a side effect of evaluating the expression.

**SoftLadder:** expressions are pure; the bit is not recorded.

**Why:** a pure evaluator cannot be surprised by the order in which rungs are scanned, and `%SW`-style
system words are not modelled at all. A project that reads `%S8` will see it stay `false`; the
importer reports `SL-W031` for `%S` references it cannot represent, and this case is documented here
because `%S8` itself *is* representable (it parses fine, it simply never changes).

## 3. Word width in expressions

**Reference:** words are stored as C `int` and the shifts are 32-bit (with the arithmetic shift right
masked to clear the sign bit).

**SoftLadder:** matches: `Word` is `i32`, `SHR` masks the sign bit, `SHL` drops the bits that leave
the top, and `ROL`/`ROR` rotate over 32 bits.

No divergence — recorded because the format's `$8000` literals look 16-bit and are not.

## 4. Rung geometry limits

**Reference:** a fixed 12 × 8 matrix per rung and 5 sequential pages of 32 × 32.

**SoftLadder:** unbounded; the exporter reports `SL-W033` for anything outside the reference matrix
(columns ≥ 12, rows ≥ 8) and then writes it anyway, so nothing is lost on the SoftLadder side.

**Why:** the whole point of the new editor is that the grid stops being a cage. Imported projects are
inside the limits by construction.

## 5. Expression syntax

**Reference:** `@<type>/<num>@` variable placeholders, `:=` for assignment, `&`/`|` for `AND`/`OR`,
`$`-prefixed hexadecimal literals, and the function names `ABS`, `MINI`, `MAXI`, `MOY`, `POW`, `SHL`,
`SHR`, `ROL`, `ROR`.

**SoftLadder:** `%MW0`, `=` for comparison, `AND`/`OR` (with `&`/`|` accepted), `$8000` and `0x8000`,
and the same functions with `MIN`/`MAX`/`AVG` as the canonical names and the ClassicLadder aliases
accepted. The importer translates in both directions.

**Why:** the `%`-notation is the one ClassicLadder's own variable table shows to users, and
`MIN`/`MAX`/`AVG` are the IEC spellings. Untranslatable text is kept verbatim with `SL-W031` so it is
visible in the Problems panel rather than silently dropped.

## 6. Features SoftLadder has and the format cannot express

A simulation-bench panel, SFC sections, bit accessors (`%MW0.3`), indexed variables in a symbol or an
expression position the reference cannot address, and columns or rows outside the reference matrix
are all reported as `SL-W033` on export and simply omitted from the ClassicLadder file. The SoftLadder
project keeps them; the exported copy is lossy by definition and says so.

## 7. Variable families that are not modelled

`%SW<n>` (system words), `%T<n>.R` (the deprecated timer's running bit) and `%M<n>.R` (the deprecated
monostable's running bit) have no SoftLadder equivalent. An element that references one is skipped
with `SL-W030` and the variable is reported with `SL-W031`; the deprecated timer and monostable
*blocks* themselves are imported as IEC timers (`On` and `Pulse` respectively) with their preset and
base preserved.

## Closing these

Items 1 and 2 are cheap to close if a real project needs them: the counter's output rows would become
three cells instead of one, and `%S8` would become a system bit written by the shift/rotate
functions. Neither is worth doing before the SFC work (M4), which is where imported sequential
projects will be measured the same way this corpus was.
