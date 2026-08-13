# cargo-sanshain

The `cargo sanshain` subcommand: publish this crate's API specifications to a
[Sanshain Service](https://github.com/paxel/sanshain-service), and download the pinned
endpoint snippets it depends on, as part of the build.

**Compatibility: `cargo-sanshain` 2.x speaks Sanshain Service 2.x.** The library behind it
is [`sanshain`](https://crates.io/crates/sanshain).

```console
$ cargo install cargo-sanshain
```

## Configure

`sanshain.yaml` in the crate root — see the [library README](https://crates.io/crates/sanshain)
for the full reference:

```yaml
serviceName: gateway-service
provides:
  - file: specs/openapi.yaml
requires:
  - serviceName: user-service
    version: 2.3.0
    outputDirectory: generated/user-service
    endpoints:
      - method: GET
        path: /api/v1/users/{id}
```

## Commands

```console
$ cargo sanshain provide          # publish as a snapshot (the default)
$ cargo sanshain provide --ga     # publish as an immutable GA release
$ cargo sanshain require          # download the pinned snippets
```

`require` writes one file per entry into its `outputDirectory`. A single endpoint is fetched
with `GET /require`; several arrive as one merged specification via `POST /require-bundle`,
so shared schemas are emitted once and generated types stay shared.

## Flags and environment

| Flag       | Environment        | Effect                                                          |
|------------|--------------------|-----------------------------------------------------------------|
| `--ga`     | `SANSHAIN_GA`      | Publish as immutable GA instead of a snapshot.                  |
| `--trunk`  | `SANSHAIN_TRUNK`   | This build is trunk's: it maintains the main graph.             |
| `--tag=<branch>` | `SANSHAIN_TAG` | This build belongs to a sanshain-branch (release or hotfix).  |
| —          | `SANSHAIN_URL`     | Server base URL; overrides `sanshainUrl` from `sanshain.yaml`.  |
| —          | `SANSHAIN_TOKEN`   | API token, sent as `Authorization: Bearer` on every request.    |

Stability and stream are declared by the pipeline, never committed to `sanshain.yaml` — the
same checkout is built by trunk CI and on a developer's laptop, and only the pipeline knows
which of them speaks for trunk. Declaring both `--trunk` and `--tag` fails before any
request is sent.

## In CI

```yaml
# trunk pipeline
- run: cargo sanshain require --trunk
- run: cargo sanshain provide --trunk --ga

# release-branch hotfix
- run: cargo sanshain provide --tag "Release-Maribou" --ga
```

Publishing GA requires the `releaser` role; without it the server answers `403` and the
error names the role. Re-providing byte-identical content is a no-op, so unconditional
`provide` on every build causes no churn.

## Retiring a protocol

Removing a `provides` entry tells Sanshain nothing — it cannot distinguish a dropped
protocol from a pipeline that stopped running. Keep the entry and mark it:

```yaml
provides:
  - file: specs/events.yaml
    apiType: asyncapi
    retired: true
```

The next `cargo sanshain provide` retires that family: the capability tag is cleared, it
leaves the current graph, and its AsyncAPI channel contracts are released. Version history
and existing Consumer pins are untouched. Retiring needs the `releaser` role or a maintainer
grant on the Producer.

## Licence

Apache-2.0.
