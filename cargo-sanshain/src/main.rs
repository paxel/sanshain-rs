use anyhow::bail;
use clap::{Parser, Subcommand};
use sanshain::{
    BEST_EFFORT_ENV_VAR, CONFIG_FILE, Client, Config, STRICT_ENV_VAR, Stream, resolve_stability,
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
    },
    /// Download the pinned endpoint snippet(s) declared in sanshain.yaml
    Require {
        #[command(flatten)]
        stream: StreamArgs,
    },
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
    fn resolve(&self) -> anyhow::Result<Stream> {
        Ok(Stream::resolve(self.trunk, self.tag.as_deref())?)
    }
}

/// How this run treats missing configuration and failed calls.
struct Mode {
    /// Missing configuration fails instead of warning and skipping.
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

    let config = match Config::from_yaml_file(CONFIG_FILE) {
        Ok(config) => config,
        // A configuration that cannot be loaded cannot say `bestEffort`, so
        // only the flag and the environment soften it — as in sanshain-js.
        Err(err)
            if resolve_switch_with(
                args.best_effort,
                env(BEST_EFFORT_ENV_VAR).as_deref(),
                false,
            ) =>
        {
            eprintln!("Warning loading config: {err}");
            eprintln!("Continuing (best effort)");
            return Ok(());
        }
        Err(err) => return Err(err.into()),
    };
    let mode = Mode {
        strict: resolve_switch_with(false, env(STRICT_ENV_VAR).as_deref(), config.strict),
        best_effort: resolve_switch_with(
            args.best_effort,
            env(BEST_EFFORT_ENV_VAR).as_deref(),
            config.best_effort,
        ),
    };

    match args.command {
        Commands::Provide { ga, stream } => provide(&config, &mode, ga, &stream),
        Commands::Require { stream } => require(&config, &mode, &stream),
    }
}

/// Missing configuration: a warning and a clean exit, or a failure under
/// strict. Never softened by best effort — it is not a failed call.
fn skip(mode: &Mode, missing: &str, command: &str) -> anyhow::Result<()> {
    if mode.strict {
        bail!("strict: {CONFIG_FILE} has no {missing}");
    }
    eprintln!(
        "No {missing} in {CONFIG_FILE} - skipping {command}. Set strict: true to fail in this case."
    );
    Ok(())
}

/// Under best effort a failed call is reported and the run still succeeds.
fn soften(mode: &Mode, result: anyhow::Result<()>) -> anyhow::Result<()> {
    match result {
        Err(err) if mode.best_effort => {
            eprintln!("{err:#}");
            eprintln!("Continuing (best effort)");
            Ok(())
        }
        other => other,
    }
}

fn provide(config: &Config, mode: &Mode, ga: bool, stream: &StreamArgs) -> anyhow::Result<()> {
    let Some(service_name) = config.service_name.as_deref() else {
        return skip(mode, "serviceName", "provide");
    };
    if config.provides.is_empty() {
        return skip(mode, "provide/provides entries", "provide");
    }
    match provide_all(config, service_name, mode, ga, stream) {
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
) -> anyhow::Result<bool> {
    let client = Client::from_config(config)?.with_stream(stream.resolve()?);
    let stability = resolve_stability(ga);
    let mut provided = false;
    for entry in &config.provides {
        if entry.retired {
            println!("Retiring {} for '{service_name}'...", entry.api_type);
            let shed = client.retire(service_name, entry.api_type)?;
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
        let stored = client.provide(service_name, entry, stability)?;
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
        .and_then(|client| Ok(client.with_stream(stream.resolve()?)))
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
        match client.require(service_name, entry) {
            Ok(written) => println!("Wrote {}", written.display()),
            // Each entry stands alone under best effort: one unreachable pin
            // does not keep the others from downloading.
            Err(err) if mode.best_effort => {
                eprintln!(
                    "Error requiring {} {}: {err}",
                    entry.service_name, entry.version
                );
                eprintln!("Continuing (best effort)");
            }
            Err(err) => return Err(err.into()),
        }
    }
    Ok(())
}
