# Older Changes

This file contains historical changelog entries for sanshain-rs.
For recent changes, see [CHANGELOG.md](CHANGELOG.md).

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
