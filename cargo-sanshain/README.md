# cargo-sanshain

The `cargo sanshain` subcommand: publish this crate's API specifications to a
[Sanshain Service](https://github.com/paxel/sanshain-service), and download the pinned
endpoint snippets it depends on, as part of the build.

**The documentation lives on the Sanshain site: [Rust client](https://paxel.github.io/sanshain-service/developer/clients/rust.html).** It lists every flag,
variable and `sanshain.yaml` field with the version it arrived in, the precedence, strict mode
and best effort, the failure modes and CI recipes. The file itself is described in
[`sanshain.yaml`](https://paxel.github.io/sanshain-service/developer/sanshain-yaml.html).

**Compatibility: `cargo-sanshain` 4.x speaks Sanshain Service 2.x.** Rust 1.89 or later. The
library behind it is [`sanshain`](https://crates.io/crates/sanshain).

## Install

```console
$ cargo install cargo-sanshain
```

## A minimal example

`sanshain.yaml` in the crate root:

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

Apache-2.0. See [LICENSE](https://github.com/paxel/sanshain-rs/blob/summer/LICENSE).
