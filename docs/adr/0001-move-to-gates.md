# sanshain-rs moves to the gates standard

Decided on 2026-11-18.

sanshain-rs had its own gates: a CI job with format, clippy and tests, `cargo audit`, a
SonarQube Cloud workflow fed by `cargo llvm-cov`, CodeQL, Scorecard, and a release that ran
the same checks and published to crates.io. Every paxel repository had done so, each
differently. [paxel/gates](https://github.com/paxel/gates) is the one standard that replaces
those, as shared workflows a repository calls, and its audit found 19 checks of it missing
here ([issue 6](https://github.com/paxel/sanshain-rs/issues/6)).

sanshain-rs meets the standard of gates v1.2.0
([sanrs-1000](https://github.com/paxel/sanshain-rs/issues/9) to
[sanrs-1007](https://github.com/paxel/sanshain-rs/issues/16)). Where the standard decides, it
is followed without a note here: the shared OSV gate, Rust gate, SonarQube Cloud job, CodeQL,
Scorecard, fuzz, release gate and provenance workflows, one coverage ratchet, the lint set in
`Cargo.toml`, the files and the badge row. What follows is where sanshain-rs adds to it, or
had a choice.

**The oldest supported Rust.** The crates promise Rust 1.89 in `rust-version`, and a
published library keeps such a promise only when it is tested. The shared Rust gate runs on
the release `rust-toolchain.toml` pins, 1.100.0. So CI keeps a job of the repository's own
that runs the tests on 1.89, naming the toolchain, because the pin file would otherwise
choose there too.

**Linux only.** The shared gate can also build on Windows and macOS. sanshain-rs ships
source, as crates, and no binary for any system, so it does not ask for that.

**Lints.** The set was already denied by attributes at the top of both crates, except
`dbg!` and `#[allow]`; it moved to `[workspace.lints]` and nothing had to be silenced.
`unsafe` stays forbidden, which is stricter than the standard's denial. The attributes
exempted the unit tests inside `src/` from the whole set; they are now linted like the
integration tests, in the second clippy run.

**Coverage.** One part, `rust`, over the whole workspace, measured by the gate with
cargo-tarpaulin. The first floor is 93.78, the local measurement on Rust 1.100.0 minus 0.1.
SonarQube Cloud used to get its coverage from `cargo llvm-cov`; it now reads the file the
gate measured.

**Fuzz targets.** Three, with the names every Sanshain client uses: `config` reads
`sanshain.yaml`, `annotations` fills the placeholders of annotation values, `response` reads
the body and the warning headers of a failed call. There is no `state` target, because
sanshain-rs keeps no state file. The readers `response` needs are private. They are shown
through a module `fuzzing` that exists only when the library is compiled with
`--cfg fuzzing`, as cargo-fuzz does, so the public API of a normal build is unchanged. The
bodies of successful answers are read by serde alone and have no target.

**The fuzz crate's lockfile** is committed: the OSV gate wants a scanned lockfile beside
every manifest. osv-scanner reads its exceptions from the file beside the lockfile, so
`fuzz/osv-scanner.toml` exists next to the one at the root, and Dependabot updates both
lockfiles.

**Accepted licences.** `webpki-roots` (CDLA-Permissive-2.0) is Mozilla's list of root
certificates, which ureq's default TLS setup brings in. `libfuzzer-sys` (NCSA) carries
libFuzzer and is linked only into the fuzz targets. Both are accepted per package until
2027-02-18; no advisory is open.

**Release.** The shared release gate checks alerts, findings, vulnerabilities, licences and
the changelog section, but runs no tests. A tag can sit on any commit, and a version on
crates.io cannot be replaced, so the release keeps a job of its own that checks the tag
against the workspace version and runs format, clippy and every test on the tagged commit.

**What the release attaches.** Both crates as `.crate` files, the files crates.io holds,
with the SBOM of the source, a build provenance attestation and its Sigstore bundle. cargo
cannot upload a file it did not package itself. Packaging is reproducible, so the publishing
job packages each crate again and compares it byte for byte with the attested file before
it uploads; a difference stops the upload. The library is published first: the lockfile
inside the subcommand's crate names the library's checksum, which is the same only once
that very file is on crates.io.

## Consequences

- A branch gets its CI run through its pull request; pushes run CI only on `summer`.
- SonarQube Cloud analyses pull requests as well as `summer`, and no longer imports a clippy
  report: clippy fails CI on any warning, so the report was always empty.
- `cargo audit` is gone; osv-scanner reads the same advisories and the licences as well.
- Fuzzing, CodeQL, Scorecard and the weekly Security run come from gates; the weekly run
  also fails on SonarQube Cloud findings and secret alerts.
- The release and the weekly Security run need the secret `GATES_ALERTS_TOKEN`, a
  fine-grained token that can read the repository's alerts
  ([sanrs-1008](https://github.com/paxel/sanshain-rs/issues/17)).
- The release's SBOM and Sigstore bundle are named `sanshain-rs-<tag>-source.cdx.json` and
  `sanshain-rs-<tag>-provenance.sigstore.json`.
