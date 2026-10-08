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

## Top-level schema (v2)

The field order below is the order `serde` emits, and therefore the order that appears in the file:

```jsonc
{
  "schema_version": 2,
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
      "wire_mode": "Implicit",       // "Implicit" (gaps conduct) | "Explicit" (gaps break)
      "elements": [
        { "kind": "ContactNo",
          "var": { "kind": "PhysIn", "index": 0, "index_expr": null, "accessor": null },
          "col": 0, "row": 0, "connected_with_top": false, "params": [] },
        { "kind": {"Timer": {"mode": "On"}},
          "var": { "kind": "TimerIec", "index": 0, "index_expr": null, "accessor": null },
          "col": 1, "row": 0, "connected_with_top": false, "params": ["3000"] },
        { "kind": "CoilOut",
          "var": { "kind": "PhysOut", "index": 0, "index_expr": null, "accessor": null },
          "col": 2, "row": 0, "connected_with_top": false, "params": [] }
      ]
    }
  ],
  "symbols": [
    { "name": "start_button",
      "var": { "kind": "PhysIn", "index": 0, "index_expr": null, "accessor": null },
      "comment": "Start pushbutton", "unit": null }
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
- `connected_with_top` marks the cell as wired to the cell above it in the same column; it is what
  builds parallel branches and merges. See [`SEMANTICS.md`](SEMANTICS.md) §2.
- `wire_mode` decides what an empty cell does inside a live row: `Implicit` (the default) makes it
  conduct, which is what the editor produces when elements are placed apart; `Explicit` makes a gap
  break the circuit, which is ClassicLadder's behaviour and what the importer sets on every rung it
  reads, so imported programs keep behaving exactly as they did. See
  [ADR-0007](adr/0007-wire-modes.md).
- `var.kind` is a `VarKind` variant name (`MemBit`, `MemWord`, `PhysIn`, `PhysOut`, `PhysInWord`,
  `PhysOutWord`, `TimerIec`, `Counter`, `Register`, `Step`, `System`, `Led`); `index_expr` holds a
  nested `VarRef` for indexed variables (`%MW[%MW0]`); `accessor` selects a sub-value of a structured
  variable (see below).
- Element parameters live in `params`, whose meaning depends on `kind`. This keeps the schema stable
  while the element library grows; each element documents its parameters in
  [`ELEMENTS.md`](ELEMENTS.md).
- `Symbol` binds a mnemonic to a variable: `var` is optional (`#[serde(default)]`), so a symbol
  without a binding is still valid.

### Accessors (schema v2)

ClassicLadder exposes function-block sub-values as suffixed variables, and SoftLadder follows the
same spelling. `accessor` is one of:

| `accessor` | Suffix | Meaning | Example |
| --- | --- | --- | --- |
| `null` | — | the variable itself (for `%TM`/`%C` this is the done bit) | `%Q3`, `%MW7`, `%C2` |
| `"Value"` | `.V` | current value (timer/counter, SFC step time) | `%TM0.V`, `%C1.V` |
| `"Preset"` | `.P` | preset | `%TM0.P`, `%C1.P` |
| `"Done"` | `.Q` / `.D` | done/output bit | `%TM0.Q`, `%C1.D` |
| `"Empty"` | `.E` | register/counter empty | `%R0.E`, `%C1.E` |
| `"Full"` | `.F` | register/counter full | `%R0.F`, `%C1.F` |
| `"In"` | `.I` | register input value | `%R0.I` |
| `"Out"` | `.O` | register output value | `%R0.O` |
| `"Count"` | `.S` | number of values stored in a register | `%R0.S` |
| `"Activity"` | `.A` | SFC step activity | `%X2.A` |
| `{"Bit": 3}` | `.3` | bit 3 of a word | `%MW0.3` |

Parsing is permissive in the direction of the reference: `%TM0` and `%TM0.Q` are the same variable,
`%C0` and `%C0.D` are the same variable, and the aliases `%B`→`%M`, `%W`→`%MW` are still accepted.
`Display` always renders the canonical form (`%TM0.Q`, `%MW0.3`), so a round trip normalizes input.

## Migrations

`softladder-project::native` keeps a `MIGRATIONS: &[fn(Value) -> Result<Value, ProjectError>]` chain.
Loading a document with `schema_version = N` applies migrations `N..CURRENT` in order and only then
deserializes, so a v1 document never reaches the v2 types. Every migration ships with a fixture pair
(`vN_before.slprj`, `vN_after.slprj`) asserted with `insta`.

### v1 → v2

| v1 | v2 |
| --- | --- |
| `var.bit: 3` | `var.accessor: {"Bit": 3}` |
| `var.kind: "TimerIecValue"` | `var.kind: "TimerIec"`, `var.accessor: "Value"` |
| `var.kind: "CounterValue"` | `var.kind: "Counter"`, `var.accessor: "Value"` |

Everything else is unchanged; missing `accessor` and `connected_with_top` fields default to `null`
and `false`. The migration also rewrites `schema_version`, and re-serializing a migrated document is
byte-identical to loading and saving it as v2.

## Compatibility guarantees

| Change | Required action |
| --- | --- |
| Additive optional field | keep `schema_version`, add `#[serde(default)]` |
| Rename/retype/remove field | bump `schema_version`, add a migration |
| Semantic change of an existing element | bump `schema_version`, add a migration + ADR |

Current version: **2**.

The importer for ClassicLadder files is deliberately a separate concern: it produces the same
in-memory `Project` and never writes `.clprj` structures into `.slprj`. See
[`COMPAT.md`](COMPAT.md).
