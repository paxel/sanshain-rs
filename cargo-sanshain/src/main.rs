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
    BEST_EFFORT_ENV_VAR, CONFIG_FILE, Client, Config, Origin, OriginFlags, STRICT_ENV_VAR, Stream,
    puzzled_cat, resolve_stability, resolve_switch_with,
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
    /// Turn failed calls into warnings and exit 0 (same as
    /// SANSHAIN_BEST_EFFORT=true or bestEffort: true in sanshain.yaml)
    #[arg(long, global = true)]
    best_effort: bool,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Publish the configured spec(s) under the version declared in each spec
    /// file; snapshot stability unless switched to GA
    Provide {
        /// Publish as immutable GA (same switch as SANSHAIN_GA=true)
        #[arg(long)]
        ga: bool,
        #[command(flatten)]
        stream: StreamArgs,
        #[command(flatten)]
        origin: OriginArgs,
    },
    /// Download the pinned endpoint snippet(s) declared in sanshain.yaml
    Require {
        #[command(flatten)]
        stream: StreamArgs,
    },
    /// Push the Backstage catalog-info.yaml named by catalogInfo, so the graph
    /// shows the peripheral resources this system depends on (needs Sanshain
    /// Service 2.4.0 or later)
    Catalog {
        #[command(flatten)]
        stream: StreamArgs,
    },
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
        let (stream, warning) = Stream::resolve(self.trunk, self.tag.as_deref());
        if let Some(warning) = warning {
            eprintln!("warning: {warning}");
            if mode.strict {
                return Err(StrictFailure(format!("strict: {warning}")).into());
            }
        }
        Ok(stream)
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

    // A configuration that cannot be loaded cannot say `bestEffort`, so only
    // the flag and the environment soften it — as in sanshain-js.
    let fallback =
        || resolve_switch_with(args.best_effort, env(BEST_EFFORT_ENV_VAR).as_deref(), false);
    let warn_and_continue = |err: sanshain::Error| {
        eprintln!("Warning loading config: {err}");
        eprintln!("Continuing (best effort)");
    };
    // Unreadable file, invalid YAML or a legacy field: nothing else is known.
    let loaded = match Config::load_yaml_file(CONFIG_FILE) {
        Ok(loaded) => loaded,
        Err(err) if fallback() => {
            warn_and_continue(err);
            return Ok(());
        }
        Err(err) => return Err(err.into()),
    };

    // Once per run, right after the legacy rejections and before any other
    // validation, so a typo shows even when it causes the next error. Under
    // strict an unknown field fails here, and best effort does not soften it.
    let strict = resolve_switch_with(false, env(STRICT_ENV_VAR).as_deref(), loaded.strict);
    if let Some(cat) = puzzled_cat(&loaded.unknown_fields, strict) {
        eprintln!("{cat}");
        if strict {
            bail!("strict: {CONFIG_FILE} has unknown fields");
        }
    }

    let config = match loaded.config {
        Ok(config) => config,
        Err(err) if fallback() => {
            warn_and_continue(err);
            return Ok(());
        }
        Err(err) => return Err(err.into()),
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
        Commands::Provide { ga, stream, origin } => provide(&config, &mode, ga, &stream, &origin),
        Commands::Require { stream } => require(&config, &mode, &stream),
        Commands::Catalog { stream } => catalog(&config, &mode, &stream),
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
) -> anyhow::Result<()> {
    let Some(service_name) = config.service_name.as_deref() else {
        return skip(mode, "serviceName", "provide");
    };
    if config.provides.is_empty() {
        return skip(mode, "provide/provides entries", "provide");
    }
    match provide_all(config, service_name, mode, ga, stream, origin) {
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

/// Provides (or retires) every entry; whether anything was provided at all.
fn provide_all(
    config: &Config,
    service_name: &str,
    mode: &Mode,
    ga: bool,
    stream: &StreamArgs,
    origin: &OriginArgs,
) -> anyhow::Result<bool> {
    let client = Client::from_config(config)?
        .with_stream(stream.resolve(mode)?)
        .with_origin(origin.resolve());
    let stability = resolve_stability(ga);
    let mut provided = false;
    for entry in &config.provides {
        if entry.retired {
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
            provided = true;
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
        provided = true;
    }
    Ok(provided)
}

fn require(config: &Config, mode: &Mode, stream: &StreamArgs) -> anyhow::Result<()> {
    let Some(service_name) = config.service_name.as_deref() else {
        return skip(mode, "serviceName", "require");
    };
    if config.requires.is_empty() {
        return skip(mode, "requires entries", "require");
    }
    let client = match Client::from_config(config)
        .map_err(anyhow::Error::from)
        .and_then(|client| Ok(client.with_stream(stream.resolve(mode)?)))
    {
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
fn catalog(config: &Config, mode: &Mode, stream: &StreamArgs) -> anyhow::Result<()> {
    let Some(service_name) = config.service_name.as_deref() else {
        return skip(mode, "serviceName", "catalog");
    };
    let Some(catalog_info) = config.catalog_info.as_deref() else {
        return skip(mode, "catalogInfo", "catalog");
    };
    let push = || -> anyhow::Result<()> {
        let client = Client::from_config(config)?.with_stream(stream.resolve(mode)?);
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
