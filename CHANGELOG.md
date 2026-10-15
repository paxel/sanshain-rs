# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).


## [3.0.0] - Unreleased

### Added
- `cargo sanshain catalog` pushes the Backstage `catalog-info.yaml` named by the new `catalogInfo` field in `sanshain.yaml` and prints the resources Sanshain recorded; a `--trunk` build updates the main graph as well as the dev graph. A file Sanshain cannot read records nothing and is printed as a warning, exiting 0 (failing under strict mode). The library offers the same as `Client::catalog`.
- `catalog` requires Sanshain Service 2.4.0; all other commands work with any 2.x service.
- Strict mode (`strict: true` in `sanshain.yaml` or `SANSHAIN_STRICT=true`): missing configuration fails the command instead of warning and skipping.
- Best effort (`bestEffort: true`, `SANSHAIN_BEST_EFFORT=true` or `--best-effort`): a failed call is printed as a warning and the command exits 0.
- `format: openapi` on an `apiType: asyncapi` requires entry downloads the pinned channels as an OpenAPI document under the same file name; needs Sanshain Service 2.4.0.
- A field in `sanshain.yaml` that no Sanshain client knows (a typo such as `catalogInf`) is reported once per run with the puzzled cat and ignored; under strict mode it fails the command.

### Changed
- Relative paths in `sanshain.yaml` (`file`, `outputDirectory`, `catalogInfo`) resolve against the directory containing `sanshain.yaml`, not the working directory.
- A missing `serviceName`, or no `provide`/`provides` or `requires` entries for the command at hand, now warns and exits 0 instead of failing; strict mode restores the failure.
- Declaring both trunk and a tag (`--trunk`/`SANSHAIN_TRUNK` and `--tag`/`SANSHAIN_TAG`) no longer fails: it prints `trunk and tag are mutually exclusive - this build is recorded in neither` and sends neither; strict mode fails instead. In the library, `Stream::resolve` and `Stream::resolve_with` return the stream together with that warning instead of an error.
- Every warning Sanshain sends with a response (the `Sanshain-Warning` header, e.g. for an unknown sanshain-branch) is printed as `warning: <text>`, one line per warning; under strict mode it fails the command with `strict: Sanshain warned: <text>`, even with best effort. The library collects them for `Client::take_warnings`.
- `SANSHAIN_GA` and `SANSHAIN_TRUNK` ignore surrounding whitespace, so ` true ` switches them on like `true`, `TRUE` or `1`.

### Security
- Updated `rustls` to 0.23.46 (RUSTSEC-2026-0285, RUSTSEC-2026-0346, RUSTSEC-2026-0348) and `anyhow` to 1.0.104 (RUSTSEC-2026-0190).

### Breaking Changes
- Library: `Config::service_name` is now an `Option<String>`, and `Config` has new public fields (`strict`, `best_effort`, `catalog_info`, `unknown_fields`), so code that reads or builds a `Config` must be adapted.
- Library: `Stream::resolve` and `Stream::resolve_with` return `(Stream, Option<&str>)` instead of a `Result`.

---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
