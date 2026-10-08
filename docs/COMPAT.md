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

| ClassicLadder | SoftLadder | Notes |
| --- | --- | --- |
| `%B<n>` | `%M<n>` | memory bit |
| `%W<n>` | `%MW<n>` | memory word |
| `%I<n>` / `%Q<n>` | `%I<n>` / `%Q<n>` | physical digital |
| `%IW<n>` / `%QW<n>` | `%IW<n>` / `%QW<n>` | physical analog |
| `%TM<n>` | `%TM<n>` | IEC timer instance |
| `%C<n>` | `%C<n>` | counter instance |
| `%R<n>` | `%R<n>` | register (FIFO/LIFO) |
| `%X<n>` | `%X<n>` | SFC step |
| `%S<n>` | `%S<n>` | system variable |
| `%QLED<n>` | `%QLED<n>` | user LED |
| legacy `%T<n>` / `%M<n>` timers-monostables | import as a `TON` FB | deprecated in ClassicLadder; reported as a warning |

Indexed variables (ClassicLadder's `IndexedVarType` / `IndexedVarNum`) map to
`VarRef::index_expr`. Bit extraction (`%W20:X4`, on the original TODO) has no ClassicLadder
equivalent and is therefore export-blocked with a clear diagnostic.

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

`StrRung.NbrLinesUsed` (the rung's used height) is preserved so the editor can keep the original
vertical extent instead of always drawing 8 rows.

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
