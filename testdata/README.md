# Test data

This directory holds third-party material used by the test suite. None of it is
part of the SoftLadder source distribution and none of it is committed: the
corpus is downloaded on demand into `classicladder-corpus/`, which
`.gitignore` excludes.

## `classicladder-corpus/`

A shallow clone of <https://github.com/MaVaTi56/classicladder> — the reference C
implementation of ClassicLadder (LGPL-2.1-or-later, © Marc Le Douarain and
contributors), including its `projects_examples/` directory.

It is used only as a **compatibility corpus**:

* M3 tests will import every `.clprj`/`.clprjz` in `projects_examples/` into a
  SoftLadder `Project` and check the result against the behaviour of the
  reference implementation.
* The corpus is read-only input. SoftLadder never links against or copies
  ClassicLadder code; see `NOTICE` for the clean-room statement.

Fetch or refresh it with:

```console
$ scripts/fetch_corpus.sh
```

The script is idempotent: it exits early when the corpus is already present, so
it is safe to call from CI. Delete the directory to force a fresh clone.
