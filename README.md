# sanshain-rs

Rust client for the [Sanshain Service](https://github.com/paxel/sanshain-service): the
`sanshain-rs` library and the `cargo-sanshain` subcommand. A **Producer** provides its full
API specification (OpenAPI, AsyncAPI, gRPC/Proto) under a producer-declared version; a
**Consumer** requires only the endpoint snippets it needs, pinned to an exact version.

**Compatibility: sanshain-rs 2.x speaks Sanshain Service 2.x.**

## Configuration

Configuration lives in `[package.metadata.sanshain]` in your `Cargo.toml`:

```toml
[package.metadata.sanshain]
serviceName = "gateway-service"
sanshainUrl = "http://localhost:3000"   # better: set SANSHAIN_URL in the environment

[[package.metadata.sanshain.provides]]
file = "specs/openapi.yaml"             # version is read from info.version

[[package.metadata.sanshain.provides]]
file = "specs/gateway.proto"            # version from the // sanshain-version: marker
apiType = "proto"

[[package.metadata.sanshain.requires]]
serviceName = "user-service"
version = "2.3.0"                       # exact MAJOR.MINOR.PATCH pin — no ranges, no latest
outputDirectory = "generated/user-service"
endpoints = [
    { method = "GET", path = "/api/v1/users/{id}" },
    { method = "POST", path = "/api/v1/users" },
]
```

- **The spec version lives in the spec file**, never in the configuration: `info.version`
  for OpenAPI/AsyncAPI, a mandatory `// sanshain-version: MAJOR.MINOR.PATCH` comment for
  proto. It must be strict three-part semver.
- **Every requires entry pins an exact version.** What your build downloads changes only
  when someone edits `version`. A pin that does not exist fails immediately.
- A single `provide` table is accepted in place of the `provides` list.
- Configuration is validated before any network call; removed 1.x-era fields (`branch`,
  `timeout`, `baseVersion`, `releaseBranches`, `stability`, `spec_id`, `api_key`,
  `service_url`) are rejected by name with a migration hint.

## Environment

| Variable         | Effect                                                        |
|------------------|---------------------------------------------------------------|
| `SANSHAIN_URL`   | Server base URL; overrides `sanshainUrl` from the manifest.   |
| `SANSHAIN_TOKEN` | API token, sent as `Authorization: Bearer` on every request.  |
| `SANSHAIN_GA`    | `true` (or `1`): provide as immutable GA instead of snapshot. |

## Providing

```console
$ cargo sanshain provide          # snapshot (the default)
$ cargo sanshain provide --ga     # immutable GA release
$ SANSHAIN_GA=true cargo sanshain provide   # the same switch, for CI pipelines
```

Stability is always `snapshot` unless explicitly switched to `ga` — there is no git or
branch detection. Set `SANSHAIN_GA=true` on your release pipeline; that is the whole
mechanism. Snapshots are overwritable and expire when unused; GA versions are immutable
and permanently claim their number.

## Requiring

```console
$ cargo sanshain require
```

Downloads each requires entry to its `outputDirectory`: one endpoint uses `GET /require`,
several are fetched as a single merged spec via `POST /require-bundle` (one copy of each
shared schema, so generated DTOs are shared).

From a `build.rs`, the library performs all configured requires:

```rust,no_run
fn main() -> sanshain_rs::Result<()> {
    let config = sanshain_rs::Config::from_manifest()?;
    sanshain_rs::sync_specs(&config)
}
```

## Failure modes

- **404 (unknown)** — the producer or the pinned version does not exist. A configuration
  error: fix the pin (list what exists: `GET /producers/{serviceName}/versions`).
- **410 (absent)** — the pinned version exists but deliberately does not include the
  requested endpoint(s). Pin a version that provides them.
- **409 (rejected provide)** — the version rules refused the provide; the message names
  the server's reason and the `proposed_version` to publish as instead. The fix is always
  a version bump in your own spec file — the client never modifies it.
- **Pre-2.0 server** — after a failure the client makes one lazy `GET /version`; a server
  older than 2.0 is reported as such: upgrade the server.
