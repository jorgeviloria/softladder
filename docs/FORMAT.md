# Native project format — `.slprj`

SoftLadder's own format. Design goals, in priority order:

1. **Diffable.** A PLC program lives in version control. Two consecutive commits must produce a
   readable `git diff`, which means stable key order, one element per line where possible, and no
   incidental churn from renumbering.
2. **Versioned.** Every breaking change bumps `schema_version` and ships a migration.
3. **Complete.** Everything the editor can express is saved; nothing is reconstructed by guessing.
4. **Portable.** No absolute paths, no machine-specific data, no credentials.

## Encoding

- `.slprj` — UTF-8 JSON via `serde_json::to_string_pretty`, 2-space indent, `\n` line endings,
  exactly one trailing newline. Keys are emitted in declaration order (struct field order), never
  from a hash map.
- `.slprjz` — the same bytes compressed with gzip (for embedded targets), detected on load by the
  magic bytes `1f 8b`.

## Top-level schema (v1)

The field order below is the order `serde` emits, and therefore the order that appears in the file:

```jsonc
{
  "schema_version": 1,
  "name": "Traffic light",
  "author": "",
  "comment": "",
  "sections": [
    {
      "id": 0,
      "name": "Main",
      "language": "Ladder",          // "Ladder" | "Sfc"
      "subroutine": null,            // number when it is a subroutine
      "rungs": [0, 1]
    }
  ],
  "rungs": [
    {
      "id": 0,
      "label": "LAMP",               // jump target, "" when unused
      "comment": "Self-holding lamp",
      "elements": [
        { "kind": "ContactNo",
          "var": { "kind": "PhysIn", "index": 0, "index_expr": null, "bit": null },
          "col": 0, "row": 0, "params": [] },
        { "kind": {"Timer": {"mode": "On"}},
          "var": { "kind": "TimerIec", "index": 0, "index_expr": null, "bit": null },
          "col": 1, "row": 0, "params": ["3000"] },
        { "kind": "CoilOut",
          "var": { "kind": "PhysOut", "index": 0, "index_expr": null, "bit": null },
          "col": 2, "row": 0, "params": [] }
      ]
    }
  ],
  "symbols": [
    { "name": "start_button", "comment": "Start pushbutton", "unit": null }
  ],
  "scan": { "period_ms": 10, "input_period_ms": 10 }
}
```

### Notes

- `elements` are stored as a flat list with `(col, row)` coordinates rather than a dense
  12 × 8 matrix. Empty cells are not serialized, so adding a contact to a 40-column rung costs one
  object, and reordering rendering never rewrites the file. Rungs are still grid-aligned, which is
  what ladder semantics require.
- `kind` uses serde's externally tagged representation: a plain string for unit variants
  (`"ContactNo"`, `"CoilSet"`, `"Connection"`, `"Compare"`, `"Operate"`) and a single-key object for
  data-carrying variants (`{"Timer": {"mode": "On"}}`, `{"Counter": {"kind": "Up"}}`,
  `{"Register": {"mode": "Fifo"}}`). The full list is in [`ELEMENTS.md`](ELEMENTS.md).
- `var.kind` is a `VarKind` variant name (`MemBit`, `MemWord`, `PhysIn`, `PhysOut`, `PhysInWord`,
  `PhysOutWord`, `TimerIec`, `TimerIecValue`, `Counter`, `CounterValue`, `Register`, `Step`,
  `System`, `Led`); `index_expr` holds a nested `VarRef` for indexed variables (`%MW[%MW0]`); `bit`
  holds the `.n` word-bit selector (`%MW0.3`).
- Element parameters live in `params`, whose meaning depends on `kind`. This keeps the schema stable
  while the element library grows; each element documents its parameters in
  [`ELEMENTS.md`](ELEMENTS.md).
- `Symbol` is currently just a named mnemonic (`name`, `comment`, `unit`). **Known gap (M1):** a
  symbol cannot yet be bound to a `VarRef`, so the editor cannot resolve `start_button` to `%I0`.
  Binding it adds an optional `var` field, which is backwards compatible (`#[serde(default)]`) and
  therefore does not bump `schema_version`.

## Migrations

`softladder-project::native` keeps a `MIGRATIONS: &[fn(Value) -> Result<Value, ProjectError>]`
chain. Loading a document with `schema_version = N` applies migrations `N..CURRENT` in order and
then deserializes. Every migration ships with a fixture pair (`vN_before.slprj`,
`vN_after.slprj`) asserted with `insta`.

## Compatibility guarantees

| Change | Required action |
| --- | --- |
| Additive optional field | keep `schema_version`, add `#[serde(default)]` |
| Rename/retype/remove field | bump `schema_version`, add a migration |
| Semantic change of an existing element | bump `schema_version`, add a migration + ADR |

The importer for ClassicLadder files is deliberately a separate concern: it produces the same
in-memory `Project` and never writes `.clprj` structures into `.slprj`. See
[`COMPAT.md`](COMPAT.md).
