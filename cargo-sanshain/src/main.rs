// Shipped code may not use `unsafe` and may not panic: no unwrap, expect or
// panic or unreachable, no indexing or slicing that can go out of bounds, no
// arithmetic that can overflow. Unit tests are exempt via `cfg(test)`; integration tests are
// separate crates.
#![forbid(unsafe_code)]
#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::todo,
        clippy::unimplemented,
        clippy::unreachable,
        clippy::indexing_slicing,
        clippy::string_slice,
        clippy::arithmetic_side_effects
    )
)]

use anyhow::bail;
use clap::{Parser, Subcommand};
use sanshain::{
    Annotation, BEST_EFFORT_ENV_VAR, CONFIG_FILE, Client, Config, Origin, OriginFlags,
    ProvideEntry, STRICT_ENV_VAR, Stability, Stream, fill_annotations, puzzled_cat,
    resolve_switch_with,
};

#[derive(Parser)]
#[command(name = "cargo")]
#[command(bin_name = "cargo")]
enum CargoCli {
    Sanshain(SanshainArgs),
}

#[derive(clap::Args)]
#[command(
    author,
    version,
    about = "Manage Sanshain API specs (speaks Sanshain Service 2.x)"
)]
struct SanshainArgs {
    /// Turn failed calls, and a missing or broken sanshain.yaml, into
    /// warnings and exit 0 (same as SANSHAIN_BEST_EFFORT=true; bestEffort:
    /// true in sanshain.yaml counts for failed calls only)
    #[arg(long, global = true)]
    best_effort: bool,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Publish the configured spec(s) under the version declared in each spec
    /// file; snapshot stability unless switched to GA (or autoGa on a trunk or
    /// tag build)
    Provide {
        /// Publish as immutable GA (same switch as SANSHAIN_GA=true; beats
        /// SANSHAIN_GA and autoGa)
        #[arg(long)]
        ga: bool,
        #[command(flatten)]
        stream: StreamArgs,
        #[command(flatten)]
        origin: OriginArgs,
        #[command(flatten)]
        sets: SetArgs,
    },
    /// Download the pinned endpoint snippet(s) declared in sanshain.yaml
    Require {
        #[command(flatten)]
        stream: StreamArgs,
        #[command(flatten)]
        sets: SetArgs,
    },
    /// Push the Backstage catalog-info.yaml named by catalogInfo, so the graph
    /// shows the peripheral resources this system depends on (needs Sanshain
    /// Service 2.4.0 or later)
    Catalog {
        #[command(flatten)]
        stream: StreamArgs,
        #[command(flatten)]
        sets: SetArgs,
    },
}

/// The values of the `{{NAME}}` placeholders in the annotations of
/// sanshain.yaml (Sanshain Service 2.13.0 or later).
#[derive(clap::Args)]
struct SetArgs {
    /// Fill the {{NAME}} placeholders of the annotations in sanshain.yaml;
    /// repeatable (beats SANSHAIN_SET_NAME)
    #[arg(long = "set", value_name = "NAME=value", value_parser = parse_set)]
    set: Vec<(String, String)>,
}

/// `NAME=value`, split at the first `=`, so the value may hold more.
fn parse_set(arg: &str) -> Result<(String, String), String> {
    arg.split_once('=')
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .ok_or_else(|| format!("expected NAME=value, got '{arg}'"))
}

impl SetArgs {
    /// The annotations with their placeholders filled. Every entry left out
    /// is a warning; under strict the first fails the command before any
    /// request, and best effort does not soften that.
    fn resolve(&self, config: &Config, mode: &Mode) -> anyhow::Result<Vec<Annotation>> {
        let filled = fill_annotations(&config.annotations, &self.set);
        for warning in &filled.warnings {
            eprintln!("warning: {warning}");
        }
        if let (true, Some(first)) = (mode.strict, filled.warnings.first()) {
            return Err(StrictFailure(format!("strict: {first}")).into());
        }
        Ok(filled.annotations)
    }
}

/// Why this build provides — stored with the version as its origin. The build
/// and change URL default to the runner's own variables (GitHub Actions,
/// GitLab CI, Jenkins); tickets and the note come only from the pipeline.
#[derive(clap::Args)]
struct OriginArgs {
    /// Where this build ran (same as SANSHAIN_BUILD_URL; default: the runner's own variable)
    #[arg(long, value_name = "URL")]
    build_url: Option<String>,
    /// The pull or merge request behind this build (same as SANSHAIN_CHANGE_URL; default: the runner's own variable)
    #[arg(long, value_name = "URL")]
    change_url: Option<String>,
    /// Ticket ids this build closes, comma-separated (same as SANSHAIN_TICKETS)
    #[arg(long, value_name = "IDS")]
    tickets: Option<String>,
    /// One line of intent for this provide (same as SANSHAIN_NOTE)
    #[arg(long, value_name = "TEXT")]
    note: Option<String>,
}

impl OriginArgs {
    fn resolve(&self) -> Origin {
        Origin::resolve(&OriginFlags {
            build_url: self.build_url.as_deref(),
            change_url: self.change_url.as_deref(),
            tickets: self.tickets.as_deref(),
            note: self.note.as_deref(),
        })
    }
}

/// Which graph this build's calls belong to. Declared by the pipeline, never
/// committed to sanshain.yaml — the same checkout is built by trunk CI and by a
/// developer's laptop, and only the pipeline knows which speaks for trunk.
#[derive(clap::Args)]
struct StreamArgs {
    /// This build is trunk's: its versions and pins maintain the main graph
    /// (same switch as SANSHAIN_TRUNK=true)
    #[arg(long)]
    trunk: bool,
    /// This build belongs to a sanshain-branch — a release or hotfix pipeline
    /// updating that graph instead of trunk (same as SANSHAIN_TAG)
    #[arg(long, value_name = "BRANCH")]
    tag: Option<String>,
}

impl StreamArgs {
    /// Both trunk and a tag is no error: the build is recorded in neither,
    /// with a warning — and under strict, a failure before any request.
    fn resolve(&self, mode: &Mode) -> anyhow::Result<Stream> {
        Ok(self.resolve_declared(mode)?.0)
    }

    /// [`StreamArgs::resolve`], plus whether the build declared a stream at
    /// all: trunk, a tag, or both (which sends neither) — what `autoGa` asks.
    fn resolve_declared(&self, mode: &Mode) -> anyhow::Result<(Stream, bool)> {
        let (stream, warning) = Stream::resolve(self.trunk, self.tag.as_deref());
        if let Some(warning) = warning {
            eprintln!("warning: {warning}");
            if mode.strict {
                return Err(StrictFailure(format!("strict: {warning}")).into());
            }
        }
        let declared = stream != Stream::None || warning.is_some();
        Ok((stream, declared))
    }
}

/// A failure strict mode demands. Best effort never softens it.
#[derive(Debug)]
struct StrictFailure(String);

impl std::fmt::Display for StrictFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for StrictFailure {}

/// Prints each warning Sanshain sent with a call, then hands back the call's
/// result — unless strict mode turns the first warning into a failure, which
/// beats both the result and best effort.
fn checked<T>(mode: &Mode, warnings: &[String], result: sanshain::Result<T>) -> anyhow::Result<T> {
    for warning in warnings {
        eprintln!("warning: {warning}");
    }
    if let (true, Some(first)) = (mode.strict, warnings.first()) {
        if let Err(err) = &result {
            eprintln!("{err}");
        }
        return Err(StrictFailure(format!("strict: Sanshain warned: {first}")).into());
    }
    Ok(result?)
}

/// How this run treats missing configuration and failed calls.
struct Mode {
    /// Missing configuration, and any warning Sanshain sends, fails instead
    /// of warning (and skipping).
    strict: bool,
    /// Failed calls become warnings and the run exits 0.
    best_effort: bool,
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

fn main() -> anyhow::Result<()> {
    // Cargo subcommands receive the subcommand name as the first argument when called via `cargo sanshain`
    // but when called directly as `cargo-sanshain`, they don't.
    // Clap's `bin_name = "cargo"` and the `CargoCli` enum handle this correctly.
    let CargoCli::Sanshain(args) = CargoCli::parse();

    let strict_env = env(STRICT_ENV_VAR);
    // A missing or broken sanshain.yaml cannot be trusted to say `bestEffort`
    // or `strict`, so only the flag and the environment decide for it: best
    // effort makes it a warning and a clean exit, strict from the
    // environment makes it fail anyway.
    let outside = Mode {
        strict: resolve_switch_with(false, strict_env.as_deref(), false),
        best_effort: resolve_switch_with(
            args.best_effort,
            env(BEST_EFFORT_ENV_VAR).as_deref(),
            false,
        ),
    };

    // Missing or unreadable file, invalid YAML or a legacy field: nothing
    // else is known.
    let loaded = match Config::load_yaml_file(CONFIG_FILE) {
        Ok(loaded) => loaded,
        Err(err) => return soften(&outside, Err(broken(&outside, err))),
    };

    // Once per run, right after the legacy rejections and before any other
    // validation, so a typo shows even when it causes the next error. Under
    // strict an unknown field fails here, and best effort does not soften it
    // — unless the file does not validate and best effort comes from
    // outside, where the file's own strict does not count.
    let file_strict = loaded.strict && (loaded.config.is_ok() || !outside.best_effort);
    let strict = resolve_switch_with(false, strict_env.as_deref(), file_strict);
    if let Some(cat) = puzzled_cat(&loaded.unknown_fields, strict) {
        eprintln!("{cat}");
        if strict {
            bail!("strict: {CONFIG_FILE} has unknown fields");
        }
    }

    let config = match loaded.config {
        Ok(config) => config,
        Err(err) => return soften(&outside, Err(broken(&outside, err))),
    };
    let mode = Mode {
        strict,
        best_effort: resolve_switch_with(
            args.best_effort,
            env(BEST_EFFORT_ENV_VAR).as_deref(),
            config.best_effort,
        ),
    };

    match args.command {
        Commands::Provide {
            ga,
            stream,
            origin,
            sets,
        } => provide(&config, &mode, ga, &stream, &origin, &sets),
        Commands::Require { stream, sets } => require(&config, &mode, &stream, &sets),
        Commands::Catalog { stream, sets } => catalog(&config, &mode, &stream, &sets),
    }
}

/// A missing or broken sanshain.yaml, as a failure strict mode demands when
/// strict comes from the environment.
fn broken(outside: &Mode, err: sanshain::Error) -> anyhow::Error {
    if outside.strict {
        StrictFailure(format!("strict: {err}")).into()
    } else {
        err.into()
    }
}

/// Missing configuration: a warning and a clean exit, or a failure under
/// strict. Never softened by best effort — it is not a failed call.
fn skip(mode: &Mode, missing: &str, command: &str) -> anyhow::Result<()> {
    if mode.strict {
        bail!("strict: {CONFIG_FILE} has no {missing}");
    }
    let hint = if command == "catalog" {
        ""
    } else {
        " Set strict: true to fail in this case."
    };
    eprintln!("No {missing} in {CONFIG_FILE} - skipping {command}.{hint}");
    Ok(())
}

/// The client for this run's calls. A missing URL is missing configuration:
/// under strict it fails, also with best effort.
fn client(config: &Config, mode: &Mode) -> anyhow::Result<Client> {
    Client::from_config(config).map_err(|err| {
        if mode.strict {
            StrictFailure(format!("strict: {err}")).into()
        } else {
            err.into()
        }
    })
}

/// Under best effort a failed call is reported and the run still succeeds.
fn soften(mode: &Mode, result: anyhow::Result<()>) -> anyhow::Result<()> {
    match result {
        Err(err) if mode.best_effort && !err.is::<StrictFailure>() => {
            eprintln!("{err:#}");
            eprintln!("Continuing (best effort)");
            Ok(())
        }
        other => other,
    }
}

fn provide(
    config: &Config,
    mode: &Mode,
    ga: bool,
    stream: &StreamArgs,
    origin: &OriginArgs,
    sets: &SetArgs,
) -> anyhow::Result<()> {
    let Some(service_name) = config.service_name.as_deref() else {
        return skip(mode, "serviceName", "provide");
    };
    if config.provides.is_empty() {
        return skip(mode, "provide/provides entries", "provide");
    }
    match provide_all(config, service_name, mode, ga, stream, origin, sets) {
        Ok(true) => Ok(()),
        // Only reachable when best effort skipped every missing spec file.
        Ok(false) if mode.strict => bail!("No specification files found to provide."),
        Ok(false) => {
            eprintln!(
                "No specification files found to provide. Skipping. Set strict: true to fail in this case."
            );
            Ok(())
        }
        Err(err) => soften(mode, Err(err)),
    }
}

/// Provides (or retires) every entry; whether any entry had something to
/// provide or retire (a missing spec file skipped by best effort has not).
fn provide_all(
    config: &Config,
    service_name: &str,
    mode: &Mode,
    ga: bool,
    stream: &StreamArgs,
    origin: &OriginArgs,
    sets: &SetArgs,
) -> anyhow::Result<bool> {
    // The stream first: autoGa makes a provide GA on a build that declares one.
    let (stream, declared) = stream.resolve_declared(mode)?;
    let stability = Stability::resolve(ga.then_some(true), config.auto_ga, declared);
    let annotations = sets.resolve(config, mode)?;
    let client = client(config, mode)?
        .with_stream(stream)
        .with_origin(origin.resolve())
        .with_annotations(annotations);
    let mut provided = false;
    // Each entry stands alone under best effort: one failed call does not
    // keep the next entry from being provided.
    for entry in &config.provides {
        if entry.retired {
            provided = true;
            soften(mode, retire_one(&client, mode, service_name, entry))?;
            continue;
        }
        if !entry.file.exists() {
            let message = format!("Spec file not found: {}", entry.file.display());
            if !mode.best_effort {
                bail!(message);
            }
            eprintln!("{message}");
            continue;
        }
        provided = true;
        soften(
            mode,
            provide_one(&client, mode, service_name, entry, stability),
        )?;
    }
    Ok(provided)
}

/// Retires one `retired: true` entry.
fn retire_one(
    client: &Client,
    mode: &Mode,
    service_name: &str,
    entry: &ProvideEntry,
) -> anyhow::Result<()> {
    println!("Retiring {} for '{service_name}'...", entry.api_type);
    let shed = client.retire(service_name, entry.api_type);
    let shed = checked(mode, &client.take_warnings(), shed)?;
    match &shed.tag_cleared {
        Some(tag) => println!(
            "Retired: cleared the '{tag}' capability, closed {} trunk pin(s), released {} contract(s). History and existing pins are untouched.",
            shed.trunk_pins_closed, shed.contracts_released
        ),
        None => println!(
            "Retired: closed {} trunk pin(s), released {} contract(s). History and existing pins are untouched.",
            shed.trunk_pins_closed, shed.contracts_released
        ),
    }
    Ok(())
}

/// Provides one entry whose spec file exists.
fn provide_one(
    client: &Client,
    mode: &Mode,
    service_name: &str,
    entry: &ProvideEntry,
    stability: Stability,
) -> anyhow::Result<()> {
    println!(
        "Providing {} as '{service_name}' ({stability})...",
        entry.file.display()
    );
    let stored = client.provide(service_name, entry, stability);
    let stored = checked(mode, &client.take_warnings(), stored)?;
    println!(
        "Stored version {} ({}): {} inserts, {} updates, {} deletes [{}]",
        stored.version,
        stored.stability,
        stored.changes.inserts,
        stored.changes.updates,
        stored.changes.deletes,
        stored.content_hash
    );
    // Without this the harvest is invisible: the server records the
    // consumer edges either way, so a subscription expecting a field
    // no contract guarantees would surface only later, on the GA
    // provide that refuses it. Never fatal here — that 409 is the
    // server's job, and failing twice helps nobody.
    for sub in &stored.harvested_subscriptions {
        if sub.is_advisory() {
            eprintln!("warning: subscription {sub}");
        } else {
            println!("  subscription {sub}");
        }
    }
    Ok(())
}

fn require(
    config: &Config,
    mode: &Mode,
    stream: &StreamArgs,
    sets: &SetArgs,
) -> anyhow::Result<()> {
    let Some(service_name) = config.service_name.as_deref() else {
        return skip(mode, "serviceName", "require");
    };
    if config.requires.is_empty() {
        return skip(mode, "requires entries", "require");
    }
    let client = match client(config, mode).and_then(|client| {
        Ok(client
            .with_stream(stream.resolve(mode)?)
            .with_annotations(sets.resolve(config, mode)?))
    }) {
        Ok(client) => client,
        Err(err) => return soften(mode, Err(err)),
    };
    for entry in &config.requires {
        println!(
            "Requiring {} endpoint(s) from '{}' at version {}...",
            entry.endpoints.len(),
            entry.service_name,
            entry.version
        );
        let written = client.require(service_name, entry);
        match checked(mode, &client.take_warnings(), written) {
            Ok(written) => println!("Wrote {}", written.display()),
            // Each entry stands alone under best effort: one unreachable pin
            // does not keep the others from downloading.
            Err(err) if mode.best_effort && !err.is::<StrictFailure>() => {
                eprintln!(
                    "Error requiring {} {}: {err}",
                    entry.service_name, entry.version
                );
                eprintln!("Continuing (best effort)");
            }
            Err(err) => return Err(err),
        }
    }
    Ok(())
}

/// Strict checks only `serviceName` and `catalogInfo` here: a project with no
/// provides or requires still has a periphery worth recording.
fn catalog(
    config: &Config,
    mode: &Mode,
    stream: &StreamArgs,
    sets: &SetArgs,
) -> anyhow::Result<()> {
    let Some(service_name) = config.service_name.as_deref() else {
        return skip(mode, "serviceName", "catalog");
    };
    let Some(catalog_info) = config.catalog_info.as_deref() else {
        return skip(mode, "catalogInfo", "catalog");
    };
    let push = || -> anyhow::Result<()> {
        let client = client(config, mode)?
            .with_stream(stream.resolve(mode)?)
            .with_annotations(sets.resolve(config, mode)?);
        let recorded = client.catalog(service_name, catalog_info);
        let mut warnings = client.take_warnings();
        // The header names the reason; a service sending none still warns.
        if recorded.as_ref().is_ok_and(|r| !r.recorded) && warnings.is_empty() {
            warnings.push(format!(
                "Sanshain did not record {}",
                catalog_info.display()
            ));
        }
        let recorded = checked(mode, &warnings, recorded)?;
        if !recorded.recorded {
            return Ok(());
        }
        let graphs = if recorded.trunk { "main+dev" } else { "dev" };
        println!(
            "Catalog recorded for {} ({graphs}): {} resources",
            recorded.system,
            recorded.resources.len()
        );
        for resource in &recorded.resources {
            println!("  {resource}");
        }
        Ok(())
    };
    soften(mode, push())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_set_splits_at_the_first_equals_sign() {
        assert_eq!(parse_set("A=x=y"), Ok(("A".to_string(), "x=y".to_string())));
        assert_eq!(parse_set("A="), Ok(("A".to_string(), String::new())));
        assert_eq!(
            parse_set("A"),
            Err("expected NAME=value, got 'A'".to_string())
        );
    }
}
