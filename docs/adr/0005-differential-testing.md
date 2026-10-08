# ADR-0005 — Differential testing against the C engine

- **Status:** proposed
- **Date:** 2025
- **Deciders:** project owner

## Context

The M3 milestone promises behavioural parity for imported ClassicLadder projects. Unit tests over
hand-written expectations are necessary but not sufficient: ladder semantics have subtle ordering
rules (vertical propagation, stacked coils, edge detection across cycles, timer base quantisation)
that are easy to get almost-right.

## Proposal

Add an opt-in **differential harness** that:

1. Compiles a minimal C shim around the reference engine (`calc.c`, `vars_access.c`, plus the
   variable/timer/counter state) exposing `load_project`, `set_var`, `scan_once`, `get_var` over a
   flat C ABI.
2. Wraps it from Rust in `tests/oracle/` behind a `cargo xtask oracle` command (a C toolchain is
   required; never part of `cargo test` by default and never linked into a release binary).
3. For each corpus project and each of a set of scripted input sequences, compares variable states
   after every scan cycle between the C engine and SoftLadder.
4. Writes divergences to `testdata/known-divergences.md` with a diff, so parity is measurable and
   regression-tracked instead of assumed.

## Open questions

- Shipping risk: keep the shim strictly in `tests/`/`xtask/` so no LGPL object file can end up in a
  distributed artifact (ties into ADR-0004).
- Timer/monostable legacy elements are excluded from parity by design; the exclusion list must be
  explicit.
- The C engine's scan depends on real time (`TypeTime`, `DoPauseMilliSecs`); the shim needs a
  controllable clock, which may require small patches to the vendored copy. Decision: patch the
  *copy fetched by the harness*, never the user's checkout.

## Consequences if accepted

- Parity claims become verifiable rather than aspirational, and the corpus doubles as a regression
  suite for both engines.
- CI gains an optional, slower job (Linux only).
- Effort: roughly one milestone-week of work, absorbed inside M3.
