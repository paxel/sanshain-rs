# sanshain

Client library for the [Sanshain Service](https://github.com/paxel/sanshain-service).
**Provide** a full API specification (OpenAPI, AsyncAPI, gRPC/Proto) under the version declared
in the spec file, **require** pinned endpoint snippets, and **retire** a protocol family you no
longer publish.

**The documentation lives on the Sanshain site: [Rust client](https://paxel.github.io/sanshain-service/developer/clients/rust.html),** with
[the library API](https://paxel.github.io/sanshain-service/developer/clients/rust.html#library) for builds that embed the client. The configuration file is
described in [`sanshain.yaml`](https://paxel.github.io/sanshain-service/developer/sanshain-yaml.html).

**Compatibility: `sanshain` 4.x speaks Sanshain Service 2.x.** Rust 1.89 or later. For the
command line, see [`cargo-sanshain`](https://crates.io/crates/cargo-sanshain).

## Install

```toml
[dependencies]
sanshain = "4"
```

## A minimal example

```rust
use sanshain::{CONFIG_FILE, Client, Config, Stability, Stream};

fn main() -> Result<(), sanshain::Error> {
    let config = Config::from_yaml_file(CONFIG_FILE)?;
    let Some(service_name) = config.service_name.as_deref() else {
        return Ok(()); // no serviceName: nothing to provide as
    };
    let client = Client::from_config(&config)?.with_stream(Stream::Trunk);
    for entry in &config.provides {
        let stored = client.provide(service_name, entry, Stability::Snapshot)?;
        println!("stored {} ({})", stored.version, stored.stability);
    }
    Ok(())
}
```

`Client::from_config` reads `SANSHAIN_URL` and `SANSHAIN_TOKEN` from the environment.

## Licence

Apache-2.0. See [LICENSE](https://github.com/paxel/sanshain-rs/blob/summer/LICENSE).
