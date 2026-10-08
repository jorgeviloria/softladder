# Contributing to SoftLadder

Thanks for helping build a better free PLC toolchain.

## Before you start

Please read:

- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) — crate boundaries and invariants.
- [`docs/adr/0004-clean-room-policy.md`](docs/adr/0004-clean-room-policy.md) — **required reading**.
  SoftLadder is an independent reimplementation of ClassicLadder. You may read the C sources to
  understand formats and behaviour, but you must not copy or mechanically translate their code.
- [`docs/PLAN.md`](docs/PLAN.md) — where the project is going and which milestone you are in.

## Development setup

```bash
rustup show                     # picks up rust-toolchain.toml (stable + fmt + clippy)
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
./scripts/fetch_corpus.sh       # ClassicLadder example projects for compatibility tests
```

Linux desktop builds of the editor need the usual windowing dependencies, e.g. on Debian/Ubuntu:

```bash
sudo apt-get install -y libgtk-3-dev libxkbcommon-dev libwayland-dev libudev-dev
```

## Ground rules

1. **Format before you push.** `cargo fmt` and `cargo clippy -D warnings` must be clean; CI enforces
   both.
2. **No panics on untrusted input.** Project files, expressions and monitor frames are hostile
   input: return `Err` or a `Diagnostic`, never `unwrap()`/`panic!`.
3. **Time is injected.** Nothing in `softladder-core` may read the clock, the filesystem or the
   network; those are parameters or live in an outer crate.
4. **Determinism.** Never let hash-map iteration order affect execution or serialization.
5. **Tests come with the change.** Bug fixes get a regression test; new formats get a round-trip
   test and a fuzz target.
6. **`unsafe` is forbidden** outside dedicated `-sys` crates, and must be justified in an ADR.
7. **Document decisions.** Non-obvious architectural choices go into `docs/adr/NNNN-title.md`.

## Workflow

- Branch from `main`, keep commits conventional (`feat:`, `fix:`, `docs:`, `refactor:`, `test:`,
  `chore:`), and reference the milestone in the PR description (e.g. "M3 / import").
- Update `CHANGELOG.md` under `[Unreleased]` for user-visible changes.
- Touch the docs when behaviour, formats or the element library change.

### Milestone checklist for a PR

- [ ] `cargo fmt --all --check` passes
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` passes
- [ ] `cargo test --workspace` passes
- [ ] New behavior is covered by tests (unit, snapshot, property or `.sltest` scenario)
- [ ] Public items documented; new element/FB documented in `docs/ELEMENTS.md`
- [ ] `CHANGELOG.md` updated
- [ ] ADR added if the change is architectural

## Writing a good bug report

Include: SoftLadder version/commit, OS and Rust version, the project file (or a minimal
reproduction) — **redact credentials** — the exact steps, expected vs observed behaviour, and any
`--log-format json` output. Runtime bugs are much easier to fix when the reproduction can be
scripted; consider attaching the smallest `.slprj` + `.sltest` pair that fails.

## Security

Please do not open a public issue for a vulnerability. Report it privately to the maintainers
(a `SECURITY.md` with the contact channel is added before the first release).
