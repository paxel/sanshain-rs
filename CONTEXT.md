# sanshain-rs

The Rust client of the Sanshain Service. What a producer, a consumer, a provide, a require,
a version, a stability or a stream is, the service decides: see the
[glossary of sanshain-service](https://github.com/paxel/sanshain-service/blob/summer/CONTEXT.md).
The terms below are the client's own.

## Language

**Library**:
The crate `sanshain`. It reads `sanshain.yaml` and makes the calls to the service; a build
that embeds the client uses it directly.
_Avoid_: SDK, core, API crate

**Subcommand**:
The crate `cargo-sanshain`, run as `cargo sanshain`. It wraps the library for a build and
adds nothing the library cannot do.
_Avoid_: CLI tool, plugin, binary

**Strict mode**:
Missing configuration, an unknown field in `sanshain.yaml` or a warning of the service fails
the command instead of being printed and skipped. Set by `strict` in the file or by
`SANSHAIN_STRICT`, which wins.
_Avoid_: Hard mode, fail-fast

**Best effort**:
A failure is printed as a warning and the command exits successfully, so that a build does
not stop because the service is away. Set by `--best-effort`, `SANSHAIN_BEST_EFFORT` or
`bestEffort` in the file. Where it meets strict mode is listed on the Rust client page of
the Sanshain site.
_Avoid_: Soft fail, lenient mode, ignore errors

**Puzzled cat**:
The notice a command prints once when `sanshain.yaml` holds fields no Sanshain client
knows. Under strict mode the command fails after printing it.
_Avoid_: Unknown-field warning, lint

**Warning**:
A line the service sent in a `Sanshain-Warning` response header. The library keeps the lines
until the caller takes them; printing them, and failing under strict mode, is the caller's
job.
_Avoid_: Notice, message, log line

**Placeholder**:
`{{NAME}}` in the value of an annotation in `sanshain.yaml`, filled from `--set NAME=value`
or the environment variable `SANSHAIN_SET_NAME`. An annotation with an unfilled placeholder
is left out with a warning of the client.
_Avoid_: Variable, template, macro

**Origin**:
Where a build came from, sent with a provide: the build URL, the change URL, tickets and a
note, from flags or `SANSHAIN_` variables. The two URLs are also read from the CI runner's
own variables; nothing is read from git.
_Avoid_: Metadata, build info, provenance (that is the release's attestation)
