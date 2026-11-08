# sanshain-rs

[![CI](https://github.com/paxel/sanshain-rs/actions/workflows/ci.yml/badge.svg?branch=summer)](https://github.com/paxel/sanshain-rs/actions/workflows/ci.yml)
[![Security](https://github.com/paxel/sanshain-rs/actions/workflows/security.yml/badge.svg?branch=summer)](https://github.com/paxel/sanshain-rs/actions/workflows/security.yml)
[![crates.io sanshain](https://img.shields.io/crates/v/sanshain?label=sanshain)](https://crates.io/crates/sanshain)
[![crates.io cargo-sanshain](https://img.shields.io/crates/v/cargo-sanshain?label=cargo-sanshain)](https://crates.io/crates/cargo-sanshain)
[![MSRV 1.89](https://img.shields.io/badge/MSRV-1.89-blue)](https://github.com/paxel/sanshain-rs/actions/workflows/ci.yml)
[![docs.rs](https://docs.rs/sanshain/badge.svg)](https://docs.rs/sanshain)
[![dependency status](https://deps.rs/crate/sanshain/latest/status.svg)](https://deps.rs/crate/sanshain)
[![unsafe forbidden](https://img.shields.io/badge/unsafe-forbidden-success)](https://github.com/paxel/sanshain-rs/blob/summer/sanshain/src/lib.rs)
[![panic-free](https://img.shields.io/badge/panic--free-linted-success)](https://github.com/paxel/sanshain-rs/blob/summer/sanshain/src/lib.rs)
[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue)](https://github.com/paxel/sanshain-rs/blob/summer/LICENSE)

Rust client for the [Sanshain Service](https://github.com/paxel/sanshain-service): the
[`sanshain`](sanshain) library and the [`cargo-sanshain`](cargo-sanshain) subcommand. A
**Producer** provides its full API specification (OpenAPI, AsyncAPI, gRPC/Proto) under the
version declared in the spec file; a **Consumer** requires only the endpoint snippets it needs,
pinned to an exact version.

**The documentation lives on the Sanshain site: [Rust client](https://paxel.github.io/sanshain-service/developer/clients/rust.html).** It lists every flag,
variable and `sanshain.yaml` field with the version it arrived in, the precedence, strict mode
and best effort, the failure modes, CI recipes and the library API. The file itself is described
in [`sanshain.yaml`](https://paxel.github.io/sanshain-service/developer/sanshain-yaml.html).

**Compatibility: 4.x speaks Sanshain Service 2.x.** Rust 1.89 or later.

| Crate                                | What it is                                              |
|--------------------------------------|---------------------------------------------------------|
| [`sanshain`](sanshain)               | The client library — configuration, and a `Client` that provides, requires, retires and pushes the catalog. |
| [`cargo-sanshain`](cargo-sanshain)   | The `cargo sanshain` subcommand wrapping it for builds. |

## Install

```console
$ cargo install cargo-sanshain
```

## A minimal example

`sanshain.yaml` in the project root:

```yaml
serviceName: gateway-service
provides:
  - file: specs/openapi.yaml         # the version is read from info.version
requires:
  - serviceName: user-service
    version: 2.3.0                   # an exact pin
    outputDirectory: generated/user-service
    endpoints:
      - method: GET
        path: /api/v1/users/{id}
```

With `SANSHAIN_URL` and `SANSHAIN_TOKEN` in the environment:

```console
$ cargo sanshain provide          # snapshot (the default)
$ cargo sanshain provide --ga     # immutable GA, from the release pipeline
$ cargo sanshain require          # writes generated/user-service/user-service.yaml
```

## Licence

Apache-2.0. See [LICENSE](LICENSE).
