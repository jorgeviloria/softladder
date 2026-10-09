# ClassicLadder compatibility (`.clp` / `.clprj` / `.clprjz`)

This document records the ClassicLadder project format as observed in the reference implementation
(`classicladder/src/files.c`, `files_project.c`, `files_sequential.c`) and how SoftLadder imports
and exports it. It is written from the format and observable behaviour only — no C code is
translated (see [ADR-0004](adr/0004-clean-room-policy.md)).

## 1. Container

`JoinFiles()` / `SplitFiles()` implement a flat text container. The file is opened through zlib's
`gzopen()`, which transparently reads **both** gzip-compressed and plain text, so `.clprj` and
`.clprjz` share one format — only the compression differs.

```
_FILES_CLASSICLADDER
_FILE-general.txt
<contents of general.txt>
_/FILE-general.txt
_FILE-rungs.txt
<contents of rungs.txt>
_/FILE-rungs.txt
...
_/FILES_CLASSICLADDER
```

- First line: `_FILES_CLASSICLADDER`.
- Part start: `_FILE-<name>`; part end: `_/FILE-<name>`.
- Last line: `_/FILES_CLASSICLADDER`.
- Lines are capped at 300 bytes in the C reader (`fgets(..., 300, ...)`), so long lines could be
  split in principle; treat line boundaries as soft.
- `.clp` is the legacy extension for the same container; SoftLadder accepts `.clp`, `.clprj` and
  `.clprjz` (detecting gzip by magic bytes `1f 8b`).

Part names seen in the wild: `general.txt`, `rungs.txt`, `sections.txt`, `symbols.txt`, `vars.txt`,
`logs.txt`, `alarms.txt`, `sequential_<n>.txt`.

## 2. Part contents

Every part starts with a version header comment of the form `#VER=<major>.<minor>`; the C reader
branches on it to migrate old files. Observed versions: rungs/symbols/sections/etc. at `2.0`–`3.0`,
several legacy parts at `1.0`.

- `general.txt` — sizes of every array (`NBR_RUNGS`, `NBR_BITS`, `NBR_WORDS`, `NBR_TIMERS_IEC`,
  `NBR_COUNTERS`, `NBR_REGISTERS`, `NBR_SECTIONS`, `NBR_SYMBOLS`, physical I/O counts, …), scan
  periods, project properties (`PARAM_VERSION=`, author, company, dates, comment), remote-alarm
  settings, and the IO mapping table (device type, port/sub-device, first channel, channel count,
  inversion flag, config data).
- `rungs.txt` — one rung per record: used flag, previous/next links, label (10 chars), comment
  (100 chars), and the `RUNG_WIDTH × RUNG_HEIGHT` (12 × 8) cell matrix with, per cell, element type,
  `ConnectedWithTop`, variable type/number and indexed-variable type/number.
- `sections.txt` — used flag, name (20 chars), language (`0` ladder, `1` sequential), subroutine
  number (`-1` for main), first/last rung or sequential page.
- `symbols.txt` — variable name (10 chars), symbol (10 chars), comment (30 chars).
- `sequential_<n>.txt` — steps (init flag, number, page, x, y) and transitions (condition variable,
  up to 10 target steps for AND divergence, reverse-condition targets, page/coordinates), plus
  sequential comments.
- `vars.txt` / `logs.txt` / `alarms.txt` — saved variable lists, event-log configuration and the
  8 remote-alarm slots (SMS/email, including SMTP credentials in clear text — SoftLadder never
  imports those credentials).

## 3. Variable mapping

ClassicLadder declares its variables in a name table (`vars_names_list.c`); the spellings below are
authoritative, and SoftLadder's namespace is a superset of them. The `accessor` column refers to
[`FORMAT.md`](FORMAT.md) §"Accessors (schema v2)".

| ClassicLadder | SoftLadder (`kind`) | `accessor` | Notes |
| --- | --- | --- | --- |
| `%B<n>` | `MemBit` | — | bit memory; `%M<n>` is the canonical spelling |
| `%W<n>` | `MemWord` | — | word memory; `%MW<n>` canonical |
| `%I<n>` / `%Q<n>` | `PhysIn` / `PhysOut` | — | physical digital |
| `%IW<n>` / `%QW<n>` | `PhysInWord` / `PhysOutWord` | — | physical analog |
| `%QLED<n>` | `Led` | — | user LED |
| `%S<n>` | `System` | — | system bit |
| `%SW<n>` | *(pending)* | — | system word: **not yet modelled**, import reports a warning |
| `%TM<n>.Q` | `TimerIec` | `Done` (`null` accepted) | timer output; bare `%TM<n>` means the same |
| `%TM<n>.P` | `TimerIec` | `Preset` | |
| `%TM<n>.V` | `TimerIec` | `Value` | elapsed, in time-base units |
| `%C<n>.D` | `Counter` | `Done` (`null` accepted) | done bit |
| `%C<n>.E` / `%C<n>.F` | `Counter` | `Empty` / `Full` | wrap-around indicators, see [`SEMANTICS.md`](SEMANTICS.md) §3.6 |
| `%C<n>.P` / `%C<n>.V` | `Counter` | `Preset` / `Value` | |
| `%R<n>.E` / `%R<n>.F` | `Register` | `Empty` / `Full` | |
| `%R<n>.I` / `%R<n>.O` | `Register` | `In` / `Out` | value pushed / popped |
| `%R<n>.S` | `Register` | `Count` | number of stored values |
| `%X<n>.A` / `%X<n>.V` | `Step` | `Activity` / `Value` | SFC step activity and time |
| legacy `%T<n>.D/.R/.P/.V` | `TimerIec` | `Done` / — / `Preset` / `Value` | old timer family; imported as an IEC timer with a warning (`%T<n>.R` has no equivalent) |
| legacy `%M<n>.R/.P/.V` | `TimerIec` | — / `Preset` / `Value` | old monostable family; imported as a pulse timer with a warning |

Indexed variables (ClassicLadder's `IndexedVarType` / `IndexedVarNum`) map to
`VarRef::index_expr`. Bit extraction (`%W20.3`, which ClassicLadder has only on its TODO list) is a
SoftLadder extension and therefore cannot be exported; it is reported as a warning.

## 4. Element mapping (ladder)

| ClassicLadder constant | Element | SoftLadder |
| --- | --- | --- |
| `ELE_INPUT` | NO contact | `ElementKind::Contact { negated: false, edge: None }` |
| `ELE_INPUT_NOT` | NC contact | `negated: true` |
| `ELE_RISING_INPUT` / `ELE_FALLING_INPUT` | edge contacts | `edge: Rising` / `Falling` |
| `ELE_CONNECTION` | wire / vertical link | `Connection` (`ConnectedWithTop`) |
| `ELE_TIMER_IEC` | IEC timer | `Timer { mode: On/OFF/Pulse }` |
| `ELE_TIMER`, `ELE_MONOSTABLE` | legacy timer/monostable | `Timer { mode: ... }` + warning |
| `ELE_COUNTER` | counter | `Counter { kind: Up/Down/UpDown }` |
| `ELE_REGISTER` | FIFO/LIFO register | `Register { mode }` |
| `ELE_COMPAR` | comparison | `Compare` with an `Expr` |
| `ELE_OUTPUT` / `_NOT` | coil / inverted coil | `Coil { kind: Out }` |
| `ELE_OUTPUT_SET` / `_RESET` | set / reset coil | `Coil { kind: Set/Reset }` |
| `ELE_OUTPUT_JUMP` | jump to label | `Coil { kind: Jump }` |
| `ELE_OUTPUT_CALL` | subroutine call | `Coil { kind: Call }` |
| `ELE_OUTPUT_OPERATE` | assignment | `Operate` with an `Expr` |
| `ELE_UNUSABLE` | part of a multi-cell element | skipped on import (reconstructed on export) |

`StrRung.NbrLinesUsed` is parsed and **not** modelled: `Rung` derives its height from the rows its
elements use, and the exporter writes `max(8, rows)`, which reproduces the reference's eight-row
extent in practice. A rung whose content really needs more than eight rows keeps them, but the
reference's own reader caps at `RUNG_HEIGHT`, so such rows are reported as `SL-W033` on export.

A cell is `Type-ConnectedWithTop-VarType/VarNum[IndexedVarType/IndexedVarNum]`; note the field order:
`0-1-0/0` is a **free** cell that carries a vertical link (type 0, link flag 1), while `0-0-0/1` is a
free cell with `VarNum = 1` and no link. Both are dropped as elements, but the first becomes a linked
`Connection` so the merging it expresses survives.

### Block geometry

ClassicLadder draws function blocks over several cells: timers and counters are two columns wide with
the "alive" cell on the right and their input pins reading the flow arriving at the *previous*
column, rows `y`, `y+1`, … Register blocks are two columns by three rows, and compare/operate blocks
span three columns with their power tapped two columns to the left.

SoftLadder normalizes all of them to **one cell per block** whose inputs are the flow arriving at
the rows starting at its own `(col, row)` — see [`SEMANTICS.md`](SEMANTICS.md) §3.4–3.7 for the exact
per-element row layout and the list of deliberate divergences. Because empty cells in a live row are
wires, the normalized form evaluates identically as long as the imported rung's intervening cells
were wires, which is the case in every project in the corpus; the import report flags any case where
it is not. On export, the multi-cell form is re-materialized (wires plus `ELE_UNUSABLE` body cells).

## 5. Import / export rules

**Import** is lossless-by-default and never silently drops data:

1. Parse the container, then each part by its `#VER=` header.
2. Map variables and elements using the tables above.
3. Anything unmappable or descending from a deprecated feature becomes a `Warning` diagnostic with
   the source part, line and original token.
4. Emit an **import report** (text or JSON) that the CLI prints and the UI shows in the Problems
   panel.

**Export** targets ClassicLadder 3.0 (rungs) / 2.0 (sections) so an existing installation can load
the result:

- Only elements with a ClassicLadder equivalent are exported; the rest are listed in the warnings.
- `%M`/`%MW` are written back as `%B`/`%W`.
- Timer/counter/register instances are renumbered densely and consistently.
- Sizes in `general.txt` are computed from the project (rounded up to ClassicLadder's minimums).

## 6. Acceptance criteria (M3)

The golden corpus is the 39 projects in the reference repository's `projects_examples/`
(`scripts/fetch_corpus.sh` clones it into `testdata/classicladder-corpus`, git-ignored).

1. **No panics** importing any corpus file (also enforced by the `clprj` fuzz target).
2. **Round-trip stability**: `import → export → import` yields an identical `Project` (excluding
   diagnostics) for every corpus file.
3. **Behavioural parity**: for corpus projects that use only M1/M4 elements, SoftLadder's scan
   produces the same variable states as the reference C engine for a scripted input sequence.
   Where parity is impossible by design (deprecated timers, `%QLED`), the difference is documented
   and the case is listed in `testdata/known-divergences.md`.
4. **Report quality**: each corpus file imports with an explicit report; no `Warning` without a
   location.

## 7. Divergences accepted by design

| Area | ClassicLadder | SoftLadder |
| --- | --- | --- |
| Array sizes | fixed maxima per project inside `general.txt` | dynamic; `general.txt` limits are informational |
| Registers | `NBR_REGISTERS` × `REGISTER_LIST_SIZE` | dynamic vectors |
| SFC pages | 5 pages, 128 steps, 256 transitions, 32 × 32 | unbounded, free-form pages |
| Expression length | 50 bytes | unlimited (compiled AST) |
| Alarm credentials | SMTP user/password stored in the project | never imported; configured outside the project |
| Monitor protocol | proprietary binary over UDP/serial/modem | JSON/CBOR over TCP/WS; legacy protocol support tracked in [ADR-0006](adr/0006-monitor-protocol.md) |
| Chart evolution | `RefreshSequentialPage` repeats the page up to 50 times until it settles, so a chain of transitions can fire through in one scan | one snapshot per scan: a chain advances by one transition per scan ([`SEMANTICS.md`](SEMANTICS.md) §4) |
| Step time | `%X<n>.V` counts whole seconds | milliseconds |
| Step storage | one global step array shared by every page | pages are self-contained; `%X<n>` is the step's own number |
| Chart comments | `N` records are part of the page drawing | dropped on import with `SL-W030`; the page and step comments are kept |

## 8. Format reference

This section is normative for `softladder-project::classicladder`: the exact shape of every part the
importer reads and the exporter writes. It was verified against the reference implementation's
loaders/savers (`files.c`, `files_project.c`, `files_sequential.c`) and against the 41 projects in
`projects_examples/`.

### 8.1 Part inventory

| Part | Modelled | Notes |
| --- | --- | --- |
| `project_infos.txt` | yes | `KEY=VALUE` lines: `PROJECT_NAME`, `PROJECT_SITE`, `PARAM_VERSION`, `PARAM_AUTHOR`, `PARAM_COMPANY`, `CREA_DATE`, `MODIF_DATE`, `PARAM_COMMENT` (with literal `\n` escapes) |
| `general.txt` | partly | `KEY=VALUE`: `PERIODIC_REFRESH`, `PERIODIC_INPUTS_REFRESH`, `REAL_INPUTS_OUTPUTS_ONLY_ON_TARGET`, `SIZE_*` array sizes, and the Modbus/serial settings (passed through, see §8.5) |
| `sections.csv` | yes | see §8.3 |
| `rung_<n>.csv` | yes | one part per rung, `<n>` is the flat rung index, see §8.2 |
| `symbols.csv` | yes | `VARNAME,SYMBOL,COMMENT` |
| `timers_iec.csv` | yes | `TM<n>,<base>,<preset>,<mode>`; base `0` = 1 minute (`TIME_BASE_MINS` is 60 000 ms), `1` = 1 s, `2` = 100 ms; mode `0` = on-delay, `1` = off-delay, `2` = pulse |
| `counters.csv` | yes | `C<n>,<preset>` |
| `registers.csv` | yes | `R<n>,<mode>`; `1` = FIFO, `2` = LIFO, `0` = undefined |
| `arithmetic_expressions.csv` | yes | `%04d,<expression>` (index zero-padded to four digits) |
| `timers.csv`, `monostables.csv` | no | deprecated element families; their presets/bases are read so that imported blocks keep their timing, and the parts are passed through |
| `sequential.csv` | yes | the SFC charts: pages, steps, transitions and their conditions (§8.4) |
| `ioconf.csv`, `modbusioconf.csv`, `com_params.txt`, `modem_config.txt`, `remote_alarms.txt`, `config_events.csv`, `spy_vars.csv` | no | M5/M6/M8 territory; passed through untouched (§8.5) |

### 8.2 Rung file

```
#VER=3.0
#LABEL=<label>
#COMMENT=<comment>
#PREVRUNG=<flat index of the previous rung>
#NEXTRUNG=<flat index of the next rung>
#NBRLINES=<rows used by this rung>
<cell>, <cell>, ... (one line per row, `RUNG_WIDTH` = 12 cells)
```

A cell is `Type-ConnectedWithTop-VarType/VarNum`, optionally followed by `[IndexedVarType/IndexedVarNum]`
for an indexed variable — e.g. `1-0-50/1` (`ELE_INPUT` on `%I1`), `1-1-50/6` (same, wired to the cell
above), `50-0-0/1` (`ELE_OUTPUT` on `%B1`), `13-0-0/0` (`ELE_TIMER_IEC` instance 0). The separator
between cells is a comma; the reference reader also tolerates spaces around it.

Element types:

| # | Constant | Meaning |
| --- | --- | --- |
| 0 | `ELE_FREE` | empty cell |
| 1 / 2 | `ELE_INPUT` / `_NOT` | normally open / closed contact |
| 3 / 4 | `ELE_RISING_INPUT` / `_FALLING_INPUT` | edge contacts |
| 9 | `ELE_CONNECTION` | wire |
| 10 / 11 | `ELE_TIMER` / `ELE_MONOSTABLE` | deprecated timer families |
| 12 | `ELE_COUNTER` | counter block (2 columns × 4 rows) |
| 13 | `ELE_TIMER_IEC` | IEC timer (2 columns × 2 rows) |
| 14 | `ELE_REGISTER` | register (2 columns × 3 rows) |
| 20 | `ELE_COMPAR` | compare block (3 columns) |
| 50 / 51 | `ELE_OUTPUT` / `_NOT` | output coil / negated output |
| 52 / 53 | `ELE_OUTPUT_SET` / `_RESET` | latch / unlatch |
| 54 / 55 | `ELE_OUTPUT_JUMP` / `_CALL` | jump to a rung / call a subroutine |
| 60 | `ELE_OUTPUT_OPERATE` | assignment (3 columns) |
| 99 | `ELE_UNUSABLE` | body cell of a multi-cell block |

Variable types (the numeric space is split at `VAR_ARE_WORD = 199`: below it the variable is a bit,
above it a word):

| # | Constant | SoftLadder |
| --- | --- | --- |
| 0 | `VAR_MEM_BIT` | `%M<n>` (written `%B<n>` upstream) |
| 10 / 11 | `VAR_TIMER_DONE` / `_RUNNING` | deprecated timer outputs |
| 15 | `VAR_TIMER_IEC_DONE` | `%TM<n>.Q` |
| 20 | `VAR_MONOSTABLE_RUNNING` | deprecated |
| 25 / 26 / 27 | `VAR_COUNTER_DONE` / `_EMPTY` / `_FULL` | `%C<n>.D` / `.E` / `.F` |
| 30 | `VAR_STEP_ACTIVITY` | `%X<n>.A` |
| 50 / 60 | `VAR_PHYS_INPUT` / `_OUTPUT` | `%I<n>` / `%Q<n>` |
| 65 | `VAR_USER_LED` | `%QLED<n>` |
| 70 | `VAR_SYSTEM` | `%S<n>` |
| 80 / 81 | `VAR_REGISTER_EMPTY` / `_FULL` | `%R<n>.E` / `.F` |
| 200 | `VAR_MEM_WORD` | `%MW<n>` (upstream `%W<n>`) |
| 220 | `VAR_STEP_TIME` | `%X<n>.V` |
| 230 / 231 | `VAR_TIMER_PRESET` / `_VALUE` | deprecated |
| 240 / 241 | `VAR_MONOSTABLE_PRESET` / `_VALUE` | deprecated |
| 250 / 251 | `VAR_COUNTER_PRESET` / `_VALUE` | `%C<n>.P` / `.V` |
| 260 / 261 | `VAR_TIMER_IEC_PRESET` / `_VALUE` | `%TM<n>.P` / `.V` |
| 270 / 280 | `VAR_PHYS_WORD_INPUT` / `_OUTPUT` | `%IW<n>` / `%QW<n>` |
| 290 | `VAR_WORD_SYSTEM` | `%SW<n>` — **not modelled** (see §8.4) |
| 300 / 301 / 302 | `VAR_REGISTER_IN_VALUE` / `_OUT_VALUE` / `_NBR_VALUES` | `%R<n>.I` / `.O` / `.S` |

### 8.3 Sections

```
#VER=1.0
#NAME000=<name of section 0>
000,<language>,<subroutine>,<firstRung>,<lastRung>,<sequentialPage>
```

The leading number is the section index zero-padded to three digits; it also marks the section as
used. `language` is `0` for ladder and `1` for sequential; `subroutine` is `-1` for a main section;
for a ladder section `firstRung`/`lastRung` are flat rung indices and `sequentialPage` is `0`
(the reference chains rungs through `#PREVRUNG`/`#NEXTRUNG`, so the section's rung list is the
linked-list walk, not necessarily a contiguous range).

### 8.4 Mapping rules

* **Multi-cell blocks are placed on their reference *body* column.** ClassicLadder reads a block's
  inputs with `StateOnLeft(alive_column - 1, row + i)` — the power arriving at the column of the
  block's *body* — while SoftLadder reads a cell's input as `state_on_left(column, row + i)`. Keeping
  the alive column would shift every imported block's enable one or two columns to the right and land
  on a cell the reference leaves unused, so the block would never be enabled. The importer therefore
  puts the block on the body column (`alive - (width - 1)`) and materialises a wire in each column it
  vacated, up to and including the reference's alive cell, so the flow still reaches the next column.
  Vertical links recorded on the block's body cells are materialised as linked `Connection` cells,
  because the reference's input tap walks the links of the body column and the corpus uses them (58
  body cells carry a link). The exporter is the exact mirror: the block is written on the alive
  column, the rest of the rectangle becomes `ELE_UNUSABLE`, and each body cell keeps the vertical link
  our model recorded for it.
* **Multi-cell blocks** are read from their top-right ("alive") cell and become one SoftLadder cell:
  a counter at `(x, y)` is 2 columns × 4 rows with its input rows read from column `x-1` at
  `y..y+3`; a timer is 2 × 2; a register is 2 × 3; compare and operate are 3 columns wide with their
  power tapped two columns to the left. Body cells (`ELE_UNUSABLE`) are dropped; the mapping is the
  inverse of the geometry documented in [`SEMANTICS.md`](SEMANTICS.md) §3.4–3.7.
* `ConnectedWithTop` becomes `PlacedElement::connected_with_top`.
* Indexed variables become `VarRef::index_expr`; `%W<n>`/`%B<n>` become `%MW<n>`/`%M<n>`.
* `ELE_OUTPUT_OPERATE` and `ELE_COMPAR` carry the index of an `arithmetic_expressions.csv` entry;
  the expression is copied into `params`.
* **`sequential.csv`** records, one per line, after a `#VER=` header:
  * `P<page>,<comment>` — a page comment (written only when it is not empty).
  * `S<slot>,<init>,<stepNumber>,<page>,<x>,<y>` — a step. `slot` is the array index the transition
    records refer to; **`stepNumber` is the user's number and is what `%X<n>` addresses**.
  * `T<slot>,<activate ×10>,<deactivate ×10>,<linkStart ×10>,<linkEnd ×10>,<page>,<x>,<y>` — a
    transition. The activate/deactivate sets hold step **slots** (`-1` unused): all deactivate steps
    must be active for it to fire (AND convergence) and the activate steps are set together (AND
    divergence). The twenty link fields are the editor's OR-bracket drawing and are dropped with
    `SL-W030`.
  * `C<slot>,0,<VarType>/<VarNum>` — the transition's condition variable, as the same
    `(VarType, VarNum)` pair the rung cells use. A transition without a `C` record defaults to
    `%M0`, matching the reference.
  * `N<index>,<page>,<x>,<y>,<comment>` — a free comment on the page; not modelled, dropped with
    `SL-W030`.
  The importer translates every slot reference into step numbers and numbers transitions densely in
  slot order, so skipped records still leave `import → export → import` a fixed point; a page no
  `sections.csv` entry references gets a synthesized SFC section. The reference's capacities (5
  pages, 128 steps, 256 transitions, 10 targets per direction) are reported as `SL-W033` on export.
* Timer and counter presets come from `timers_iec.csv` / `counters.csv`. A timer's `params[0]` is a
  duration in **milliseconds** (see `docs/SEMANTICS.md` §3.5), so the importer preserves both the
  value and the recorded base by writing base 2 (100 ms) as `"<preset * 100>"`, base 1 (1 s) as
  `"<preset>s"` and base 0 (1 min) as `"<preset>m"`. A bare number would silently mean milliseconds
  and lose the base. Counter presets are plain counts. `timers.csv`/`monostables.csv` only supply a
  preset for the deprecated families, converted with the base they record (`preset * base_ms`).
* `%SW<n>` (system words) and the deprecated `%T<n>`/`%M<n>` families are **not modelled**: the
  element is imported with a warning (`SL-W031`) and, when a variable reference cannot be
  represented at all, the element is skipped with `SL-W030`.
* A register's capacity is clamped **up** to the reference minimum (`SIZE_REGISTER_LIST`, 500) so
  that export cannot drift: the reference's `SIZE_*` values are hints, and a project that records a
  smaller list would otherwise be rewritten with a larger one on the next export.
* A `sections.csv` rung list is resolved by walking `#PREVRUNG`/`#NEXTRUNG`; when that chain is broken
  (11 of the 41 corpus files) the section falls back to its `firstRung..lastRung` range and reports
  `SL-W030`, so nothing is dropped silently.

### 8.5 Export and passthrough

Export rebuilds a document the reference implementation can load:

* Every rung is written as a dense 12 × `max(8, rows)` matrix: elements are placed at their cell,
  multi-cell blocks are expanded to their reference geometry (block on the alive column, body cells
  as `ELE_UNUSABLE`, vertical links copied from our model), and gaps are filled with `ELE_CONNECTION`
  (`Type 9`) **only for `WireMode::Implicit` rungs**. An imported rung is `Explicit`, so its gaps are
  real breaks and must stay: filling them would add elements that the next import turns into
  `Connection` elements, breaking the round-trip fixed point. The authored → reference direction is
  where the filling belongs.
* `sections.csv`, `symbols.csv`, `timers_iec.csv`, `counters.csv`, `registers.csv`,
  `arithmetic_expressions.csv` are regenerated from the project, and `general.txt` /
  `project_infos.txt` are **merged**: the keys SoftLadder models (`PERIODIC_REFRESH`,
  `PERIODIC_INPUTS_REFRESH` and the project properties) are rewritten in place, every other line
  (the Modbus/serial settings, the `SIZE_*` hints) is preserved verbatim, and canonical keys that the
  original lacked are appended in reference order. Array sizes in `general.txt` are computed from the
  project and rounded up to the reference minimums, so an imported project keeps working if it is
  exported again.
* Parts that SoftLadder does not model are **passed through unchanged**. The importer therefore
  returns them alongside the project (`Document`), and the exporter takes them back as a template:
  importing and immediately exporting a project preserves every unmodelled part byte for byte.
* Features with no ClassicLadder equivalent (bit access `%MW0.3`, a bench panel, more than 12
  columns, a chart bigger than the reference's capacities) are reported as `SL-W033` and omitted
  from the exported file.

### 8.6 Diagnostics

| Code | Severity | Raised when |
| --- | --- | --- |
| `SL-E030` | Error | the document is malformed (bad container, unusable numbers, a part that cannot be parsed) |
| `SL-W030` | Warning | a ClassicLadder element has no SoftLadder equivalent and was skipped |
| `SL-W031` | Warning | a variable family is not modelled (`%SW<n>`, deprecated timers/monostables) |
| `SL-W032` | Warning | a part is passed through because SoftLadder does not model it |
| `SL-W033` | Warning | a SoftLadder feature has no ClassicLadder equivalent and was not exported |

### 8.7 Round-trip contract

The acceptance criterion for M3 is stability, not byte equality with the original file (the
reference rewrites whitespace, sizes and ordering). For every project in the corpus:

1. **Import never panics** and always yields either a `Project` or an `SL-E030` error.
2. **`import → export → import` is a fixed point**: the two `Project`s compare equal.
3. **Unmodelled parts survive**: exporting an imported project reproduces every passthrough part
   byte for byte.
4. **Every loss is reported**: no element or variable is dropped without a `SL-W0xx` diagnostic
   that names the part and the line.
