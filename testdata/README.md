# Test data

This directory holds third-party material used by the test suite, plus the record of
where SoftLadder knowingly differs from the reference. The corpus itself is not
committed: it is copied or cloned on demand into `classicladder-corpus/`, which
`.gitignore` excludes. `known-divergences.md` **is** committed — it is our own text.

## `classicladder-corpus/`

The `projects_examples/` directory of <https://github.com/MaVaTi56/classicladder> — the
reference C implementation of ClassicLadder (LGPL-2.1-or-later, © Marc Le Douarain and
contributors). 41 projects: 33 `.clprj` and 8 `.clp`.

It is used only as a **compatibility corpus**, and it is read-only input. SoftLadder never
links against or copies ClassicLadder code; see `NOTICE` and
[`docs/adr/0004-clean-room-policy.md`](../docs/adr/0004-clean-room-policy.md).

Fetch or refresh it with:

```console
$ scripts/fetch_corpus.sh
```

The script copies the corpus from a ClassicLadder checkout next to this repository when
there is one (so it works offline) and clones upstream otherwise. It is idempotent, and CI
calls it before running the tests, so the compatibility suite really runs there. Delete
`classicladder-corpus/` to force a refresh.

## How the corpus is used

`crates/softladder-project/tests/m3_classicladder.rs` imports every file and asserts, per
[`docs/COMPAT.md`](../docs/COMPAT.md) §8.7:

* every project imports without panicking and reports located diagnostics;
* `import → export → import` is a fixed point, and a second export is byte-identical;
* every part SoftLadder does not model survives byte for byte;
* a hand-transcribed ground truth for `example.clprj` rung 0 (cells, label, symbols,
  sections) and a *behavioural* test that the imported timer is enabled by the power the
  reference taps for it, with its preset and time base intact.

The suite prints a note and skips itself when the corpus is absent, so a checkout without
network still passes `cargo test --workspace`.

## `known-divergences.md`

Every behavioural difference between SoftLadder and the reference that the corpus exposed,
with what each side does and why we chose ours. Read it before claiming a project "runs
the same": the format round-trips exactly, but a handful of circuits legitimately differ.
