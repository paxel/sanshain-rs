# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).


## [3.1.0]

### Added
- `cargo sanshain provide` sends the origin of the build: `--build-url` and `--change-url` default to the runner's own variables on GitHub Actions, GitLab CI and Jenkins (overridden by `SANSHAIN_BUILD_URL` / `SANSHAIN_CHANGE_URL`), plus `--tickets` / `SANSHAIN_TICKETS` and `--note` / `SANSHAIN_NOTE`; the library exposes it as `Origin` and `Client::with_origin`. Sanshain Service 2.9.0 and later stores it with the version and shows it as "why".

---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
