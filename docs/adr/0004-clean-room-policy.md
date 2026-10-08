# ADR-0004 — Clean-room reimplementation policy

- **Status:** accepted
- **Date:** 2025
- **Deciders:** project owner

## Context

The ClassicLadder C sources (LGPL) sit next to this repository and are the only complete reference
for the `.clprj` format and for the exact scan semantics. Reading them is unavoidable and useful.
Copying them — including "translating line by line into Rust" — would make SoftLadder a derivative
work, dragging LGPL obligations into a project that chose MIT OR Apache-2.0 (ADR-0001).

## Decision

SoftLadder is a **clean-room-style reimplementation based on documented formats and observable
behaviour**. Concretely, contributors:

- **May** read the C sources to understand data formats, file layouts, field meanings, timing
  behaviour, and edge cases; and may read its documentation and issue tracker.
- **May** reimplement algorithms from published knowledge (ladder power flow, IEC 61131-3 function
  blocks, Modbus is a public specification).
- **Must not** copy C code, translate C statements one-to-one into Rust, or reproduce the original's
  structure, identifier names or comments as a direct mapping.
- **Must** express domain concepts in Rust idioms (enums, traits, ownership) rather than mirroring C
  structs and global state.
- **Must** record in `docs/PROVENANCE.md` any file whose content is intentionally derived from
  third-party sources, with the license and origin.

Interoperability is validated by **behavioural testing**, not by shared code: imported projects are
compared against expected results from the golden corpus and, where parity matters (ADR-0005), a
separate differential harness compiles the C engine as an oracle and compares outputs.

## Consequences

- `NOTICE` and `README` credit ClassicLadder and state the independence of this project.
- Test data (`projects_examples/`) is fetched by `scripts/fetch_corpus.sh` into a git-ignored
  directory and used only as fixtures; it is not vendored or relicensed.
- The differential harness (M3+) is opt-in, lives in `xtask`/`tests/oracle/`, requires a C toolchain,
  and is not part of the default build. It never ships in a released binary.
- Rule of thumb for reviews: if a patch would be recognizable as "the same code in another
  language", reject it and reimplement from the specification.
