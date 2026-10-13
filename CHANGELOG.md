# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).


## [2.1.0] - Unreleased

### Added
- `cargo sanshain catalog` pushes the Backstage `catalog-info.yaml` named by the new `catalogInfo` field in `sanshain.yaml` and prints the resources Sanshain recorded; a `--trunk` build updates the main graph as well as the dev graph. The library offers the same as `Client::catalog`.
- `catalog` requires Sanshain Service 2.4.0; all other commands work with any 2.x service.
- Strict mode (`strict: true` in `sanshain.yaml` or `SANSHAIN_STRICT=true`): missing configuration fails the command instead of warning and skipping.
- Best effort (`bestEffort: true`, `SANSHAIN_BEST_EFFORT=true` or `--best-effort`): a failed call is printed as a warning and the command exits 0.
- A field in `sanshain.yaml` that no Sanshain client knows (a typo such as `catalogInf`) is reported once per run with the puzzled cat and ignored; under strict mode it fails the command.

### Changed
- Relative paths in `sanshain.yaml` (`file`, `outputDirectory`, `catalogInfo`) resolve against the directory containing `sanshain.yaml`, not the working directory.
- A missing `serviceName`, or no `provide`/`provides` or `requires` entries for the command at hand, now warns and exits 0 instead of failing; strict mode restores the failure. In the library, `Config::service_name` is now an `Option<String>`.
- `SANSHAIN_GA` and `SANSHAIN_TRUNK` ignore surrounding whitespace, so ` true ` switches them on like `true`, `TRUE` or `1`.

---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
