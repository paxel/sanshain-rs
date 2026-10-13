# sanshain

Client library for the [Sanshain Service](https://github.com/paxel/sanshain-service).
**Provide** a full API specification (OpenAPI, AsyncAPI, gRPC/Proto) under a
producer-declared version, **require** pinned endpoint snippets, and **retire** a protocol
family you no longer publish.

**Compatibility: `sanshain` 2.x speaks Sanshain Service 2.x.** For the command line, see
[`cargo-sanshain`](https://crates.io/crates/cargo-sanshain).

```rust,no_run
use sanshain::{CONFIG_FILE, Client, Config, Stability, Stream};

let config = Config::from_yaml_file(CONFIG_FILE)?;
let Some(service_name) = config.service_name.as_deref() else {
    return Ok(()); // no serviceName: nothing to provide as
};
let client = Client::from_config(&config)?.with_stream(Stream::Trunk);

for entry in &config.provides {
    let stored = client.provide(service_name, entry, Stability::Snapshot)?;
    println!("stored {} ({})", stored.version, stored.stability);
}
# Ok::<(), sanshain::Error>(())
```

## Configuration

`sanshain.yaml`, the file every Sanshain client shares:

```yaml
serviceName: gateway-service
sanshainUrl: http://localhost:3000

provides:
  - file: specs/openapi.yaml
  - file: specs/gateway.proto
    apiType: proto
  - file: specs/events.yaml
    apiType: asyncapi
    retired: true          # no longer provided — see "Retiring" below

requires:
  - serviceName: user-service
    version: 2.3.0
    outputDirectory: generated/user-service
    endpoints:
      - method: GET
        path: /api/v1/users/{id}
```

- **The spec version lives in the spec file**, never in the configuration: `info.version`
  for OpenAPI/AsyncAPI, a mandatory `// sanshain-version:` comment for proto. Accepted
  spellings are `MAJOR[.MINOR[.PATCH]]` with an optional leading `v` — omitted parts are
  zero (`v2` becomes `2.0.0`) and the stored form is always the full three-part version.
- **Every requires entry pins an exact version.** What your build downloads changes only
  when someone edits `version`. A pin that does not exist fails immediately with `404`.
- A single `provide` mapping is accepted in place of the `provides` list.
- Relative paths (`file`, `outputDirectory`) resolve against the directory containing
  `sanshain.yaml` when it is loaded with [`Config::from_yaml_file`], never against the working
  directory. Absolute paths stay absolute.
- `strict: true` and `bestEffort: true` are read into [`Config::strict`] and
  [`Config::best_effort`] for the command to act on; `SANSHAIN_STRICT` /
  `SANSHAIN_BEST_EFFORT`, when set, override them ([`resolve_switch_with`]). A missing
  `serviceName` parses as `None` — the command decides whether that skips or fails.
- Configuration is validated before any network call; removed 1.x-era fields (`branch`,
  `timeout`, `baseVersion`, `releaseBranches`, `stability`, `spec_id`, `api_key`,
  `service_url`) are rejected by name with a migration hint.

- A field no Sanshain client knows (a typo like `catalogInf` or `requires[0].verison`) is
  collected in [`Config::unknown_fields`], in file order, after the rejections above. Fields
  other clients know but this one does not use (`compression`, `insecure`, …) are accepted
  silently. [`puzzled_cat`] formats the warning a command prints for them.

Parsing is separate from loading: [`Config::from_value`] validates a `serde_json::Value`,
so a caller holding configuration in another form can bypass the file entirely.

## Stability

Every provide declares one, and the default is always `snapshot`. GA is an explicit switch
— `Stability::Ga`, or `SANSHAIN_GA=true` via [`resolve_stability`]. There is no git or
branch detection. Snapshots are overwritable and expire when unused; GA versions are
immutable and permanently claim their number.

Publishing GA requires the `releaser` role. Without it the server answers `403`, and the
error names the role and the snapshot fallback.

## Streams

A build declares which dependency graph its calls belong to — trunk, a named
sanshain-branch, or neither — via [`Stream`], set once on the [`Client`]:

```rust,no_run
# use sanshain::{Client, Stream};
# let client = Client::new("http://localhost:3000", None);
let client = client.with_stream(Stream::resolve(false, Some("Release-Maribou"))?);
# Ok::<(), sanshain::Error>(())
```

The stream is a property of the invocation, not of the repository, so it comes from flags
or `SANSHAIN_TRUNK` / `SANSHAIN_TAG` and never from `sanshain.yaml`. Trunk and a tag are
mutually exclusive, and [`Stream`] cannot represent both.

## Retiring

Deleting a `provides` entry says nothing — Sanshain cannot distinguish a dropped protocol
from a pipeline that stopped running. Keep the entry, mark it `retired: true`, and call
[`Client::retire`]: the capability tag is cleared, the family leaves the current graph and
its AsyncAPI channel contracts are released. Version history and existing Consumer pins are
untouched.

Retiring needs the `releaser` role — which a release pipeline already holds — or a
maintainer grant on that Producer.

## AsyncAPI 2.x perspective

Sanshain reads 2.x `publish`/`subscribe` from the **application's** perspective: `publish`
means *this service publishes*, `subscribe` means *this service consumes*. The AsyncAPI 2.x
specification defines them from the **client's** perspective — exactly inverted. Sanshain
uses the application reading deliberately, matching the unambiguous 3.x `send`/`receive`
mapping. A document authored the spec-literal way registers its contracts, and has its
subscriptions harvested, backwards.

Harvested subscriptions come back on the provide response as
[`ProvideResponse::harvested_subscriptions`]; those where
[`HarvestedSubscription::is_advisory`] holds deserve a build warning.

## Failure modes

- **403** — the caller lacks `releaser` (GA provide) or `releaser`/maintainer (retire).
- **404** — the producer or the pinned version does not exist. Fix the pin.
- **409** — the version rules refused the provide; the message carries `proposed_version`.
  The fix is always a version bump in your own spec file; the client never edits it.
- **410** — the pinned version exists but deliberately lacks the requested endpoints.
- **Pre-2.0 server** — after a failure the client makes one lazy `GET /version` and reports
  an old instance as such.

## Licence

Apache-2.0.
