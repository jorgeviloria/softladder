# ADR-0001 — Licensing and provenance

- **Status:** proposed (needs owner confirmation)
- **Date:** 2025
- **Deciders:** project owner

## Context

SoftLadder is a functional clone of ClassicLadder (Marc Le Douarain, LGPL-2.1-or-later / LGPL-3,
with an additional LGPL-2 permission for EMC/LinuxCNC usage). The reference C sources sit next to
this repository and will be read constantly: to reproduce the `.clprj` format, to match scan
semantics, and to compare behaviour in tests. ClassicLadder is also the runtime historically
embedded in LinuxCNC (GPL-2.0), and SoftLadder wants to be usable in that ecosystem.

Options considered:

1. **MIT OR Apache-2.0** — maximum reuse, compatible with being linked from GPL-2.0 projects.
2. **LGPL-3.0-or-later** — mirrors the original; safe if we ever consider the Rust code a derived
   work; adds friction for static linking in embedded/commercial products.
3. **GPL-3.0-or-later** — maximal copyleft, closest to LinuxCNC but prevents proprietary drivers
   from linking the core.

## Decision

Adopt **MIT OR Apache-2.0** (dual, Rust-ecosystem convention) for all SoftLadder-authored code,
combined with a strict clean-room policy (ADR-0004) and explicit attribution to ClassicLadder.

## Consequences

- Downstream users (including GPL-2.0 projects such as LinuxCNC) can link the core freely.
- We must **not** translate C code line by line into Rust, or the LGPL would attach to the result.
  Format knowledge and observable behaviour are not copyrightable; expression and structure are.
- `NOTICE` credits Marc Le Douarain and the ClassicLadder project and states that SoftLadder is an
  independent reimplementation.
- Any future code that is intentionally derived from the C sources must be isolated in its own file
  with an explicit LGPL header and listed in `docs/PROVENANCE.md`.
- Test fixtures taken from the reference repository (`projects_examples/`) are fetched, not vendored,
  and are used only as test data.

## Alternatives rejected

- **LGPL-3.0-or-later**: safe but restricts the driver/embedded use cases that motivate this project.
- **GPL-3.0-or-later**: would block proprietary IO drivers and reduce adoption.
