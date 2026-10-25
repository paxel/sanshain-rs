# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).


## [3.2.0]

### Changed
- Requires Rust 1.89 (was 1.85).
- `sanshain.yaml` is read with serde-saphyr, the YAML reader of Sanshain Service 2.12; merge keys (`<<: *anchor`) now work, so `requires` entries can share settings through an anchor.
- Dependencies updated, among them ureq 3; requests behave as before (no proxy from the environment, 30 second connect timeout, at most 5 redirects).

### Security
- rustls-webpki 0.103.16 and webpki-roots 1.0.11: the distrusted emSign intermediates are no longer accepted (RUSTSEC-2026-0381, RUSTSEC-2026-0382).
- The unmaintained `unsafe-libyaml` is gone (RUSTSEC-2024-0449).

---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
