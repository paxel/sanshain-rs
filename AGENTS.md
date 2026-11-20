# sanshain-rs

The Rust client of the [Sanshain Service](https://github.com/paxel/sanshain-service): a Cargo
workspace of two published crates.

| Path             | What it is                                                                  |
|------------------|-----------------------------------------------------------------------------|
| `sanshain`       | The library: reads `sanshain.yaml` and talks to the service.                |
| `cargo-sanshain` | The `cargo sanshain` subcommand wrapping the library.                       |
| `fuzz`           | The cargo-fuzz targets, a crate of its own outside the workspace.           |

The user documentation lives on the Sanshain site, in the repository of the service.

## Agent skills

### Issue tracker

Issues live as GitHub issues in `paxel/sanshain-rs`, managed with the `gh` CLI. See
`docs/agents/issue-tracker.md`.

### Triage labels

Default canonical vocabulary: `needs-triage`, `needs-info`, `ready-for-agent`,
`ready-for-human`, `wontfix`. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: `CONTEXT.md` plus `docs/adr/` at the repo root. See `docs/agents/domain.md`.

### Ticket IDs

Prefix `sanrs`, as in `sanrs-1003`. See `docs/agents/ticket-ids.md`.

## Quality gates

The standard sanshain-rs is held to is [paxel/gates](https://github.com/paxel/gates);
`docs/adr/0001-move-to-gates.md` says where sanshain-rs adds to it and why. The workflows
call gates' shared workflows, pinned by commit; Dependabot moves the pin.

### Before a commit

`cargo fmt --all`, clippy and the tests, on the toolchain `rust-toolchain.toml` pins. Clippy
is two runs, as in the shared Rust gate: the shipped code under the whole lint set, and the
tests with the lints allowed that are how a test fails:

```sh
cargo clippy --locked --all-features -- -D warnings
cargo clippy --locked --tests --benches --examples --all-features -- -D warnings \
  -A clippy::unwrap_used -A clippy::expect_used -A clippy::panic -A clippy::indexing_slicing \
  -A clippy::string_slice -A clippy::arithmetic_side_effects -A clippy::unreachable
cargo test --locked --all-features
```

The crates promise Rust 1.89 (`rust-version` in `Cargo.toml`); CI tests it in a job of its
own with `cargo +1.89 test --workspace --locked`. Do not use what 1.89 does not have.

### Coverage ratchet

Line coverage may only go up. The Rust gate in `ci.yml` fails under the floor in
`coverage-floors.toml`. Every feature ships with tests that hold or raise coverage; a
feature that lowers it is not done. When coverage rises, raise the floor and the badge in
the same pull request, with gates checked out beside the repository:

```sh
cargo tarpaulin --engine llvm --locked --all-features --workspace \
  --skip-clean --out Lcov --output-dir coverage && mv coverage/lcov.info coverage/rust.lcov
python3 ../gates/scripts/coverage_ratchet.py --floors coverage-floors.toml
```

Never lower the floor.

### Fuzzing

`fuzz/` holds the targets `config`, `annotations` and `response`; `fuzz.yml` runs them
weekly. What `response` calls is the library's module `fuzzing`, which exists only under
`--cfg fuzzing` and is no part of the public API. `fuzz/Cargo.lock` is committed and has its
own `fuzz/osv-scanner.toml`, because osv-scanner reads the exceptions beside the lockfile.

### Before a release

fmt, both clippy runs, the tests, the tests on Rust 1.89, osv-scanner with gates'
`scripts/osv_gate.py`, the coverage ratchet and `cargo package --workspace --locked`. The
version stands in `Cargo.toml` (`[workspace.package]`), in the `sanshain` dependency of
`cargo-sanshain/Cargo.toml` and in both lockfiles. The release workflow runs the shared
release gate and the tests on the tagged commit before it packages, and publishes a crate
only when it is byte for byte the attested file.

### Security rules

The decisions behind these are in `docs/adr/0001-move-to-gates.md`.

- Shipped code may not panic: the root `Cargo.toml` denies unwrap, expect, panic, todo,
  unimplemented, unreachable, indexing, string slicing, overflowing arithmetic and `dbg!`
  under `[workspace.lints]`, and both crates take the set. Tests are exempt through the
  second clippy run.
- `unsafe` is forbidden. `#[allow(...)]` is denied: a lint that must be silenced gets
  `#[expect(<lint>, reason = "...")]`.
- Every action in `.github/workflows/` is pinned to a full commit SHA with its version in
  a trailing comment. Never reference an action by tag or branch alone.
- Rust is pinned in `rust-toolchain.toml`; the shared workflows read it from there. The
  release workflow's own jobs mirror it (`dtolnay/rust-toolchain@<sha> # <version> branch`).
  A toolchain bump is a commit of its own that changes the file and those references together.
- Every workflow starts with `permissions: contents: read`; a job that writes widens it
  itself.
- A dependency advisory is fixed by an update. When no fixed release exists, it is accepted
  in `osv-scanner.toml` with the reason sanshain-rs is not exposed and an `ignoreUntil` date.
  A licence outside the allowlist is accepted per package with `effectiveUntil`. Never
  ignore without reason and date.
- A code-scanning alert or a SonarQube Cloud finding that stays open is accepted in
  `code-scanning-ignores.toml` with its tool, rule, file, reason and an `ignoreUntil` date.
  Nothing is dismissed in GitHub or marked in SonarQube Cloud.
- The release fails on a known vulnerability, on every open code-scanning alert, secret
  alert and SonarQube Cloud finding that is not accepted, and on a missing `CHANGELOG.md`
  section for the version. The weekly Security run fails on the same, and on unmaintained
  dependencies and passed dates.
- `summer` takes pull requests only.
