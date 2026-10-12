# Known divergences from ClassicLadder

Behavioural differences between SoftLadder and the reference implementation, for projects that come
from `projects_examples/` (the compatibility corpus). The *format* round-trips exactly — see
[`docs/COMPAT.md`](../docs/COMPAT.md) §8.7 — so these are cases where the same file means something
slightly different when it runs.

Every entry states what the reference does, what SoftLadder does, and why we chose that. Entries are
removed when a milestone closes the gap.

## 1. Block outputs per row — closed, see below

This entry is kept so old links land somewhere: a rung chaining off a counter's `y+1`/`y+2` row
now reads `done`/`full`, and off a register's `y+1` row `full`, exactly like the reference.

## 2. Word width in expressions

**Reference:** words are stored as C `int` and the shifts are 32-bit (with the arithmetic shift right
masked to clear the sign bit).

**SoftLadder:** matches: `Word` is `i32`, `SHR` masks the sign bit, `SHL` drops the bits that leave
the top, and `ROL`/`ROR` rotate over 32 bits.

No divergence — recorded because the format's `$8000` literals look 16-bit and are not.

## 3. Rung geometry limits

**Reference:** a fixed 12 × 8 matrix per rung and 5 sequential pages of 32 × 32.

**SoftLadder:** unbounded; the exporter reports `SL-W033` for anything outside the reference matrix
(columns ≥ 12, rows ≥ 8) and then writes it anyway, so nothing is lost on the SoftLadder side.

**Why:** the whole point of the new editor is that the grid stops being a cage. Imported projects are
inside the limits by construction.

## 4. Expression syntax

**Reference:** `@<type>/<num>@` variable placeholders, `:=` for assignment (though its own
projects, e.g. `3pumps.clprj`, spell operate assignments with a bare `=`, as in
`@200/0@=@200/0@+1`), `&`/`|` for `AND`/`OR`, `$`-prefixed hexadecimal literals, and the function
names `ABS`, `MINI`, `MAXI`, `MOY`, `POW`, `SHL`, `SHR`, `ROL`, `ROR`.

**SoftLadder:** `%MW0`, `=` for comparison, `AND`/`OR` (with `&`/`|` accepted), `$8000` and `0x8000`,
and the same functions with `MIN`/`MAX`/`AVG` as the canonical names and the ClassicLadder aliases
accepted. The importer translates in both directions, reading a bare `=` as assignment inside an
operate cell (and exporting `:=`, which re-imports to the same parameters).

**Why:** the `%`-notation is the one ClassicLadder's own variable table shows to users, and
`MIN`/`MAX`/`AVG` are the IEC spellings. Untranslatable text is kept verbatim with `SL-W031` so it is
visible in the Problems panel rather than silently dropped.

## 5. Features SoftLadder has and the format cannot express

A simulation-bench panel, bit accessors (`%MW0.3`), indexed variables in a symbol or an expression
position the reference cannot address, and columns or rows outside the reference matrix are all
reported as `SL-W033` on export and simply omitted from the ClassicLadder file. (SFC sections used to
be in this list; since M4 they round-trip through `sequential.csv` — see
[`../docs/COMPAT.md`](../docs/COMPAT.md) §8.4.) The SoftLadder
project keeps them; the exported copy is lossy by definition and says so.

## 6. Variable families that are not modelled

`%SW<n>` (system words), `%T<n>.R` (the deprecated timer's running bit) and `%M<n>.R` (the deprecated
monostable's running bit) have no SoftLadder equivalent. An element that references one is skipped
with `SL-W030` and the variable is reported with `SL-W031`; the deprecated timer and monostable
*blocks* themselves are imported as IEC timers (`On` and `Pulse` respectively) with their preset and
base preserved.

## Closed

* **The flag that leaves a block's output cell (was item 1).** Counters drive empty/done/full
  on rows `y`/`y+1`/`y+2` and registers empty/full on `y`/`y+1`, verified against `calc.c`
  (`CalcTypeCounter`, register block): same input rows, same output rows, same apply order
  (count, preset, reset). Rows with no flag read false, like the reference's unwritten body
  cells. Deliberately not copied: the IEC timer's second-row `Running` output (our timer stays
  one row) and the `%C0.E`/`.F` latch-vs-arrival nuance — both recorded here instead of
  silently differing.
* **The rotate and shift side effect (was item 2).** `SHL`, `SHR`, `ROL` and `ROR` now publish the
  bit that left the operand to `%S8`, exactly as `arithm_eval.c` does it: the operand's most
  significant bit for a left shift or rotate, its least significant bit for a right one, whatever the
  count, with the last such operation in an expression winning. A scan that runs no shift leaves the
  bit alone. See [`../docs/SEMANTICS.md`](../docs/SEMANTICS.md) §4, "The shift and rotate carry".

* **Residual, found while closing it.** ClassicLadder's own demonstration,
  `WordsShiftsLeftRightExample.clprj`, does not become live through our import: with every
  `%I1`…`%I9` contact closed its coils still read false, so its arithmetic cells never execute. The
  carry is therefore validated in `softladder-core` (per function, last-wins, and a rung that reads
  `%S8` in the same scan) and the file is covered by an import-and-scan smoke test. Whether those
  rows should be live is an import question for the next milestone review, not a carry question.

## Closing these

Items 2 to 6 are the shape of the two implementations rather than gaps to
close.

The SFC milestone (M4) has landed, and the sequential projects were measured the way this corpus was:
eight charts, 73 steps and 100 transitions import, run and round-trip exactly (see
[`../docs/COMPAT.md`](../docs/COMPAT.md) §8.4).
