# Older Changes

This file contains historical changelog entries for sanshain-rs.
For recent changes, see [CHANGELOG.md](CHANGELOG.md).

## [3.1.0] - 2026-10-21

### Added
- `cargo sanshain provide` sends the origin of the build: `--build-url` and `--change-url` default to the runner's own variables on GitHub Actions, GitLab CI and Jenkins (overridden by `SANSHAIN_BUILD_URL` / `SANSHAIN_CHANGE_URL`), plus `--tickets` / `SANSHAIN_TICKETS` and `--note` / `SANSHAIN_NOTE`; the library exposes it as `Origin` and `Client::with_origin`. Sanshain Service 2.9.0 and later stores it with the version and shows it as "why".

## [3.0.0] - 2026-10-16

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

## [2.0.0] - 2026-08-14

### Added
- The `sanshain` library and the `cargo sanshain` subcommand, speaking Sanshain Service 2.x, configured by the shared `sanshain.yaml` (`serviceName`, `sanshainUrl`, a `provide` mapping or `provides` list, and `requires`).
- `cargo sanshain provide` publishes each configured OpenAPI, AsyncAPI or proto spec under the version declared in the file itself (`info.version` or the `// sanshain-version:` marker), with line endings normalized to LF.
- Every provide is a snapshot; `--ga` or `SANSHAIN_GA=true` publishes an immutable GA version instead.
- `cargo sanshain require` downloads the endpoints of each `requires` entry, pinned to an exact `version`, into its `outputDirectory`; several endpoints arrive as one bundled specification.
- `--trunk` / `SANSHAIN_TRUNK` and `--tag` / `SANSHAIN_TAG` declare which dependency graph a build's provides and requires belong to; declaring both fails before any request.
- A `provides` entry marked `retired: true` retires that API family on the next `provide` instead of publishing it.
- AsyncAPI provides print the harvested subscriptions, with drift or a missing publisher shown as warnings.
- The server URL comes from `SANSHAIN_URL` or `sanshainUrl`, and `SANSHAIN_TOKEN` is sent as a bearer token.
- Configuration is validated before any request: removed 1.x fields (`branch`, `timeout`, `baseVersion`, `releaseBranches`, `stability`, `spec_id`, `api_key`, `service_url`), a missing pin, a pin that is not a version, and an entry without endpoints fail with a hint.
- Failures name their fix: a `409` prints the server's proposed version, a `403` names the `releaser` role, unknown (`404`) and absent (`410`) pins are told apart, and a pre-2.0 server is reported as such.
