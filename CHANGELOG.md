# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).


## [4.0.1]

### Added
- The release on GitHub carries both crates as `.crate` files, the files crates.io holds, with a CycloneDX SBOM of the source, a build provenance attestation and its Sigstore bundle; `gh attestation verify <file> -R paxel/sanshain-rs` checks a download ([sanrs-1005](https://github.com/paxel/sanshain-rs/issues/14)). The library and the subcommand behave as in 4.0.0.

---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
