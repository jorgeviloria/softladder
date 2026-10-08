# ADR-0003 — Native project format: pretty JSON `.slprj`

- **Status:** accepted
- **Date:** 2025
- **Deciders:** project owner

## Context

ClassicLadder stores a project as a text container of parts (`general.txt`, `rungs.txt`, …) with
`#VER=` headers and dense 12 × 8 rung matrices. It is readable, but it is not designed for version
control: rung matrices are positional, sizes are baked into `general.txt`, and the container is
either one flat file or gzip — never a diff-friendly layout.

SoftLadder needs a format that (a) round-trips through ClassicLadder for interoperability, and
(b) is pleasant to review in a pull request.

Candidates:

1. **Pretty JSON** — trivial tooling, diffable, `serde` native.
2. **TOML** — nicer comments, but nested arrays of tables get awkward for rungs/elements.
3. **YAML** — diffable, but a large surface for parsers and easy to get subtly wrong.
4. **Binary (CBOR/MessagePack/bincode)** — compact and fast, but opaque to review and to `git diff`.
5. **A directory tree of files, one per section** — excellent diffs (like KiCad/`.gitignore`-aware
   tools) but complicates transfer to embedded targets and "single file" workflows.

## Decision

Use **pretty JSON as the single-file canonical form (`.slprj`)**, plus the **gzip variant
`.slprjz`** for constrained targets. Element storage is a flat `(col, row)` list, not a dense
matrix, so diffs are localized. Key order is deterministic (struct declaration order) and
serialization never depends on hash-map iteration.

## Consequences

- `git diff` on a project shows exactly which element changed in which rung.
- Schema evolution is explicit: `schema_version` + chained migrations + fixture pairs
  (see [`../FORMAT.md`](../FORMAT.md)).
- Files are larger than a binary encoding; compression (`zstd` later, gzip now) covers the embedded
  case, and a headless CI runtime does not care about a few hundred extra kilobytes.
- A directory-per-section layout remains possible as a *view* on the same model (M8+) without
  changing the schema, since the model is the source of truth and the format is just a serializer.
- The ClassicLadder importer/exporter is a separate module that never leaks container quirks into
  the native schema.

## Alternatives rejected

- **binary encoding as canonical**: kills reviewability, which is the main point.
- **TOML/YAML**: worse nested-array ergonomics or a heavier parser surface for no real gain.
- **directory layout now**: better diffs for huge projects, but worse for "send me the project"
  and for embedded deployment; can be added later as an export target.
