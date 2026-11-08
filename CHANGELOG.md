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

---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
