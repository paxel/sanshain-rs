# sanshain-rs

Rust client for the [Sanshain Service](https://github.com/paxel/sanshain-service): the
[`sanshain`](sanshain) library and the [`cargo-sanshain`](cargo-sanshain) subcommand. A
**Producer** provides its full API specification (OpenAPI, AsyncAPI, gRPC/Proto) under a
producer-declared version; a **Consumer** requires only the endpoint snippets it needs,
pinned to an exact version.

**Compatibility: 2.x speaks Sanshain Service 2.x.**

| Crate                                | What it is                                              |
|--------------------------------------|---------------------------------------------------------|
| [`sanshain`](sanshain)               | The client library — configuration, and a `Client` that provides, requires, retires and pushes the catalog. |
| [`cargo-sanshain`](cargo-sanshain)   | The `cargo sanshain` subcommand wrapping it for builds. |

## Install

```console
$ cargo install cargo-sanshain
```

## Configure

`sanshain.yaml` in the project root — the same file every other Sanshain client reads:

```yaml
serviceName: gateway-service
sanshainUrl: http://localhost:3000   # better: set SANSHAIN_URL in the environment
catalogInfo: catalog-info.yaml       # optional: Backstage file for `cargo sanshain catalog`

provides:
  - file: specs/openapi.yaml         # version read from info.version
  - file: specs/gateway.proto        # version from the // sanshain-version: marker
    apiType: proto

requires:
  - serviceName: user-service
    version: 2.3.0                   # an exact pin — no ranges, no latest
    outputDirectory: generated/user-service
    endpoints:
      - method: GET
        path: /api/v1/users/{id}
      - method: POST
        path: /api/v1/users
```

## Use

```console
$ cargo sanshain provide          # snapshot (the default)
$ cargo sanshain provide --ga     # immutable GA release
$ cargo sanshain require          # download the pinned snippets
$ cargo sanshain catalog          # push catalog-info.yaml (needs Sanshain Service 2.4.0+)
```

Relative paths in `sanshain.yaml` resolve against the directory containing it. `catalog`
needs Sanshain Service 2.4.0 or later; every other command works with any 2.x service.

See the [library README](sanshain/README.md) for the configuration reference and the API,
and the [subcommand README](cargo-sanshain/README.md) for the flags and CI recipes.

## Licence

Apache-2.0. See [LICENSE](LICENSE).
