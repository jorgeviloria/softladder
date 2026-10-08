# ADR-0007 — Explicit and implicit wiring modes per rung

- **Status:** accepted
- **Date:** 2025
- **Deciders:** project owner

## Context

SoftLadder stores a rung as the elements that are actually placed, and derives the wires: in a row
that holds at least one element, an empty cell conducts (`Implicit`). That is what makes the editor
pleasant — you drop a contact and a coil two cells apart and the circuit works.

ClassicLadder stores a dense 12 × 8 matrix in which every wire is an explicit `ELE_CONNECTION` cell,
and its engine gives a cell that is not a connection (or a contact, or a coil) no output at all: a
gap **breaks** the circuit. In a real project that is visible, e.g.
`projects_examples/example.clprj` rung 0 leaves free cells between the self-holding contact and the
cell that carries its vertical link.

While implementing the M3 importer we found that importing such a rung and evaluating it with
`Implicit` wiring makes the seal conduct where ClassicLadder's does not. Compatibility is the whole
point of M3, so the semantics had to be representable rather than "close enough".

## Decision

Add `Rung.wire_mode: WireMode { Implicit, Explicit }`, serialized with `#[serde(default)]` so it is
an additive field and the schema stays at version 2 per
[`FORMAT.md`](../FORMAT.md) §migrations.

- `Implicit` is the default and what the editor produces.
- `Explicit` makes a gap in a live row break the circuit. **The ClassicLadder importer sets it on
  every rung it reads**, together with materializing the reference's connection cells as
  `ElementKind::Connection` elements.

## Consequences

- Imported projects evaluate with ClassicLadder's behaviour, including circuits that depend on a
  gap, while programs authored in SoftLadder keep the forgiving behaviour.
- The distinction is visible in the file and in the editor, so "why does this rung behave
  differently?" is answerable without reading the engine.
- The engine change is three lines in the wire computation plus one field in the precomputed rung
  index, and it is covered by tests in both modes (gap conducts / gap breaks / explicit seal).
- Export re-materializes `ELE_CONNECTION` cells for every gap, so an `Implicit` rung written back to
  ClassicLadder keeps its meaning.
- A future editor toggle ("strict wiring") has a natural home; it is not needed for M3.

## Alternatives rejected

- **Always explicit** (ClassicLadder's rule): forces the editor to insert wire cells for the user,
  which contradicts the M2 UX goal and pollutes authored projects.
- **Always implicit**: changes the behaviour of imported projects that rely on a gap, which would
  make M3's parity claim false.
- **A per-project or per-section flag**: the distinction is naturally per rung (an imported section
  can be edited and mixed), and ClassicLadder's own granularity is the rung.
