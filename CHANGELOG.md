# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).


## [4.0.0]

### Added
- `autoGa: true` in `sanshain.yaml` makes a provide GA whenever the build declares trunk or a tag; `SANSHAIN_GA=false` keeps a build a snapshot. The library resolves it with `Stability::resolve` and exposes it as `Config::auto_ga`.
- `annotations` in `sanshain.yaml` travel with provide, require, bundle and catalog push, their `{{NAME}}` placeholders filled from `--set NAME=value` or `SANSHAIN_SET_NAME` (needs Sanshain Service 2.13.0 or later); the library offers `fill_annotations` and `Client::with_annotations`.

### Breaking Changes
- Library: `Config` has new public fields (`auto_ga`, `annotations`), so code that builds a `Config` must be adapted.

### Fixed
- `apiType: grpc` is read as `proto` on provides and requires, as the other clients read it: a require goes to `/require/grpc` and is saved as `.proto`; the spelling was refused as an invalid `sanshain.yaml`.
- Best effort softens failed calls only, the same in provide, require and catalog: an unreadable or invalid `sanshain.yaml` now fails also with `--best-effort` or `SANSHAIN_BEST_EFFORT`, a missing URL fails under strict also with best effort, and a failed provide no longer stops the next `provides` entry.
- A `403` on provide or retire names the releaser role only when the service refused for want of it; any other `403`, such as a read-only token's, shows the service's message alone.
- An empty `SANSHAIN_URL` counts as unset, so `sanshainUrl` from `sanshain.yaml` is used; it hid the configured URL and failed with "missing 'sanshainUrl'".
- The library's crate docs say 4.x and name `sanshain.yaml` as the configuration source; they said 2.x and `[package.metadata.sanshain]`.

---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
