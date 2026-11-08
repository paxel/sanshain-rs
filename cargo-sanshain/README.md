# cargo-sanshain

The `cargo sanshain` subcommand: publish this crate's API specifications to a
[Sanshain Service](https://github.com/paxel/sanshain-service), and download the pinned
endpoint snippets it depends on, as part of the build.

**Compatibility: `cargo-sanshain` 4.x speaks Sanshain Service 2.x.** The library behind it
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

Relative paths in it (`file`, `outputDirectory`) resolve against the directory containing
`sanshain.yaml`, not the working directory.

A field no Sanshain client knows is reported once per run on stderr — before any other
validation error it may cause — and the command carries on:

```text
   /\_/\  ?
  ( O_o )   sanshain.yaml: unknown field 'catalogInf' - ignored
 ---U-U---
```

Under strict mode the same cat ends in `rejected (strict)` and the command fails; best effort
does not soften that.

## Commands

```console
$ cargo sanshain provide          # publish as a snapshot (the default)
$ cargo sanshain provide --ga     # publish as an immutable GA release
$ cargo sanshain require          # download the pinned snippets
$ cargo sanshain catalog          # push the Backstage catalog-info.yaml
```

`require` writes one file per entry into its `outputDirectory`. A single endpoint is fetched
with `GET /require`; several arrive as one merged specification via `POST /require-bundle`,
so shared schemas are emitted once and generated types stay shared.

`catalog` pushes the Backstage file named by `catalogInfo` in `sanshain.yaml` (resolved
like every path there) with `PUT /catalog/<serviceName>`, so the network graph shows the
peripheral resources — databases, buckets, queues — the system depends on:

```yaml
catalogInfo: catalog-info.yaml
```

```console
$ cargo sanshain catalog --trunk
Catalog recorded for orders-service (main+dev): 1 resources
  resource:default/orders-db
```

A trunk build (`--trunk` / `SANSHAIN_TRUNK`) updates the main graph and the dev graph; any
other build only the dev graph. Without `catalogInfo` it warns and skips (fails under strict;
strict checks only `serviceName` and `catalogInfo` here). A file Sanshain cannot read as this
project's Component — usually its `metadata.name` is not the `serviceName` — records nothing
and keeps the previous periphery: `catalog` prints Sanshain's reason as a warning and exits 0
(fails under strict).
**`catalog` needs Sanshain Service 2.4.0 or later**; an older service answers `404`, reported as
such. Every other command works with any 2.x service.

## AsyncAPI as OpenAPI (`format`)

A `requires` entry with `apiType: asyncapi` may add `format: openapi`: Sanshain then answers
the pinned channels as an OpenAPI document of their message schemas, for OpenAPI code
generators. It is written under the same file name as without `format`. `openapi` is the only
value and AsyncAPI the only API type it combines with; anything else fails before any request.
To get both formats, list the entry twice with different `outputDirectory`s:

```yaml
requires:
  - serviceName: user-events
    apiType: asyncapi
    version: 1.4.0
    outputDirectory: generated/user-events/asyncapi
    endpoints:
      - method: SUB
        path: user/signup
  - serviceName: user-events
    apiType: asyncapi
    format: openapi
    version: 1.4.0
    outputDirectory: generated/user-events/openapi
    endpoints:
      - method: SUB
        path: user/signup
```

`format` needs **Sanshain Service 2.4.0 or later**. An older service ignores it and answers
AsyncAPI; the client notices, writes nothing, and fails with
`format: openapi needs Sanshain Service 2.4.0 or later (server is <version>)` (a warning
under best effort).

An older service rejects the single-endpoint request or returns AsyncAPI; the client reports
that it needs 2.4.0.

## Flags and environment

| Flag       | Environment        | Effect                                                          |
|------------|--------------------|-----------------------------------------------------------------|
| `--ga`     | `SANSHAIN_GA`      | Publish as immutable GA instead of a snapshot; `SANSHAIN_GA=false` keeps a build a snapshot despite `autoGa`. |
| `--trunk`  | `SANSHAIN_TRUNK`   | This build is trunk's: it maintains the main graph.             |
| `--tag=<branch>` | `SANSHAIN_TAG` | This build belongs to a sanshain-branch (release or hotfix).  |
| —          | `SANSHAIN_URL`     | Server base URL; overrides `sanshainUrl` from `sanshain.yaml`.  |
| —          | `SANSHAIN_TOKEN`   | API token, sent as `Authorization: Bearer` on every request.    |
| —          | `SANSHAIN_STRICT`  | Fail on missing configuration instead of warning (`strict`).    |
| `--best-effort` | `SANSHAIN_BEST_EFFORT` | Failed calls become warnings; exit 0 (`bestEffort`). |
| `--build-url=<url>` | `SANSHAIN_BUILD_URL` | Where this build ran; default: the runner's own variable. |
| `--change-url=<url>` | `SANSHAIN_CHANGE_URL` | The pull or merge request behind it; default: the runner's own variable. |
| `--tickets=<ids>` | `SANSHAIN_TICKETS` | Ticket ids this build closes, comma-separated. |
| `--note=<text>` | `SANSHAIN_NOTE` | One line of intent for this provide. |
| `--set NAME=value` | `SANSHAIN_SET_NAME` | Fills the `{{NAME}}` placeholders of the `annotations`; repeatable. |

A switch variable (`SANSHAIN_GA`, `SANSHAIN_TRUNK`, `SANSHAIN_STRICT`, `SANSHAIN_BEST_EFFORT`) is on for `true` or `1`, in any case and
with surrounding whitespace ignored; any other value leaves it off.

Stability and stream are declared by the pipeline, never committed to `sanshain.yaml` — the
same checkout is built by trunk CI and on a developer's laptop, and only the pipeline knows
which of them speaks for trunk. The one rule `sanshain.yaml` may state is `autoGa` (below). Declaring both `--trunk` and `--tag` records the build in
neither graph: it prints
`warning: trunk and tag are mutually exclusive - this build is recorded in neither` and sends
neither (fails before any request under strict).

### Origin: why this build provides

`provide` sends what the pipeline knows about why it runs (Sanshain Service 2.9.0 and later);
Sanshain stores it with the version and shows it on the version row, in the diff and the blame.
The build and change URL default to the runner's own variables on GitHub Actions
(`GITHUB_SERVER_URL`, `GITHUB_REPOSITORY`, `GITHUB_RUN_ID`, a pull request's `GITHUB_REF`), GitLab
CI (`CI_JOB_URL`, `CI_MERGE_REQUEST_PROJECT_URL`, `CI_MERGE_REQUEST_IID`) and Jenkins (`BUILD_URL`,
`CHANGE_URL`); tickets and the note come only from the pipeline. The latest provide that carries
any of it wins; a byte-identical re-provide only fills an empty origin. Nothing is read from git. A service before 2.9.0
refuses the origin fields; the subcommand then warns and repeats the provide without them, so
upgrading the client first never fails a build.

A flag wins over its `SANSHAIN_*` variable, which wins over the runner's variable. A blank value
counts as unset, so an empty flag or variable does not switch the runner's URL off; only unsetting
the runner variable does. Other CI systems (Azure Pipelines, Bitbucket Pipelines, CircleCI,
Buildkite, TeamCity, …) fill nothing: set `SANSHAIN_BUILD_URL` and `SANSHAIN_CHANGE_URL` from their
variables. The service accepts http(s) URLs, at most 20 ticket ids made of letters, digits, `-`, `_`
and `.`, and a note of at most 200 characters.

This client sends no `dry_run`. A pull-request pipeline that wants Sanshain to check the version
rules without storing anything calls the route with `curl` and `"dry_run": true` (see
[CI integration](https://paxel.github.io/sanshain-service/developer/ci-integration.html#dry-runs-for-pull-requests)).

### Annotations

`annotations` in `sanshain.yaml` are named values Sanshain shows with the provided version
(`scope: version`) or with the system (`scope: system`), for example a link to the
requirements a build implements and to the system's Jenkins job:

```yaml
annotations:
  - scope: version
    name: requirements
    value: "http://solar.system/id-{{AREA}}/requirements/{{ID}}"
  - scope: system
    name: jenkins
    value: "https://jenkins.corp/job/payments"
```

Each entry has exactly `scope` (`version` or `system`), `name` and `value`, all strings; a
missing field, another type or another scope fails before any request, naming the entry. The
service checks the limits (20 per scope, names of at most 40 letters, digits, `-` and `_`,
values of at most 2048 characters) and answers `400` naming the entry.

A placeholder `{{NAME}}` (letters, digits and `_` only; `{{ A }}` or `{{a-b}}` stays literal
text) is filled by the build: `--set NAME=value` on `provide`, `require` and `catalog`
(repeatable, split at the first `=`), else the environment variable `SANSHAIN_SET_NAME`. An
empty value counts as unset — an empty `--set` does not fall back to the variable:

```console
$ cargo sanshain provide --set AREA=7 --set ID=42
```

An entry with an unfilled placeholder is left out with a warning, and the others are sent:

```text
warning: annotation 'requirements' left out: {{ID}} is not set (--set ID=… or SANSHAIN_SET_ID)
```

Under strict mode that warning fails the command before any request, also with best effort.

`provide` sends every entry; `require`, its bundles and `catalog` send only the `system`
ones; retiring sends none. **Annotations need Sanshain Service 2.13.0 or later.** An older
service refuses them; the command then repeats the call without them, sends none for the rest
of the run, and warns once:
`warning: Sanshain Service before 2.13.0 does not take annotations; sending without them`
(strict mode does not fail on this one).

## Strict mode and best effort

By default a command warns and skips when its configuration is missing — no `serviceName`, no
`provide`/`provides` entries for `provide`, no `requires` entries for `require` — and exits 0.
That makes it safe to run both commands in every build, even where only one applies. To fail
instead, enable strict mode in `sanshain.yaml` or the environment:

```yaml
strict: true          # or SANSHAIN_STRICT=true
```

Best effort goes the other way: a failed call (server error, unreachable server, missing spec
file) is printed as a warning followed by `Continuing (best effort)`, and the build exits 0.
`require` carries on with the next entry; `provide` skips a missing spec file and stops at the
first failed call. A `sanshain.yaml` that cannot be loaded is softened only by `--best-effort`
or `SANSHAIN_BEST_EFFORT`, since the file itself cannot say so.

```yaml
bestEffort: true      # or SANSHAIN_BEST_EFFORT=true, or --best-effort
```

Best effort never softens strict: missing configuration under strict still fails.

## Warnings

The dependency graph is informational and never fails a build. Where Sanshain does not record
a call in a graph — an unknown `--tag`, a `catalog-info.yaml` it cannot read — the call
succeeds and Sanshain sends a `Sanshain-Warning` header. Each one is printed as
`warning: <text>` and the build carries on. Under strict mode a warning fails the command after
the call, with `strict: Sanshain warned: <text>` — best effort does not soften that.

A set environment variable overrides the `sanshain.yaml` value either way
(`SANSHAIN_STRICT=false` turns off `strict: true`); `--best-effort` always turns best effort on.

### autoGa: GA on every trunk or tag build

```yaml
autoGa: true
```

With `autoGa: true` a provide is GA whenever the build declares a stream: trunk (`--trunk` /
`SANSHAIN_TRUNK`) or a tag (`--tag` / `SANSHAIN_TAG`), also when it declares both. Without a
stream it stays a snapshot, so a developer's laptop never releases. The first of these that is
set decides:

1. `--ga`;
2. `SANSHAIN_GA` (an explicit `false` counts; empty or absent does not);
3. `autoGa: true` with trunk or a tag;
4. `snapshot`.

There is still no branch detection: CI declares trunk and tag on its protected pipelines only,
and that is what makes them GA. The CI token's user needs the `releaser` role, and a trunk
build that changes the spec without a version bump fails with `409 immutable_ga` and the
proposed version: bump the version in the spec file and build again. A retire never carries a
stability, `autoGa` or not.

## In CI

```yaml
# trunk pipeline
- run: cargo sanshain require --trunk
- run: cargo sanshain provide --trunk --ga
- run: cargo sanshain catalog --trunk      # needs Sanshain Service 2.4.0+

# release-branch hotfix
- run: cargo sanshain provide --tag "Release-Maribou" --ga
```

With `autoGa: true` in `sanshain.yaml` the `--ga` can go: `--trunk` and `--tag` make those
provides GA by themselves.

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
