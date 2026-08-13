use clap::{Parser, Subcommand};
use sanshain::{CONFIG_FILE, Client, Config, Stream, resolve_stability};

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

fn main() -> anyhow::Result<()> {
    // Cargo subcommands receive the subcommand name as the first argument when called via `cargo sanshain`
    // but when called directly as `cargo-sanshain`, they don't.
    // Clap's `bin_name = "cargo"` and the `CargoCli` enum handle this correctly.
    let CargoCli::Sanshain(args) = CargoCli::parse();

    let config = Config::from_yaml_file(CONFIG_FILE)?;

    match args.command {
        Commands::Provide { ga, stream } => {
            if config.provides.is_empty() {
                anyhow::bail!(
                    "nothing to provide: no 'provide'/'provides' entries in {CONFIG_FILE}"
                );
            }
            let client = Client::from_config(&config)?.with_stream(stream.resolve()?);
            let stability = resolve_stability(ga);
            for entry in &config.provides {
                if entry.retired {
                    println!(
                        "Retiring {} for '{}'...",
                        entry.api_type, config.service_name
                    );
                    let shed = client.retire(&config.service_name, entry.api_type)?;
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
                    continue;
                }
                println!(
                    "Providing {} as '{}' ({stability})...",
                    entry.file.display(),
                    config.service_name
                );
                let stored = client.provide(&config.service_name, entry, stability)?;
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
            }
        }
        Commands::Require { stream } => {
            if config.requires.is_empty() {
                anyhow::bail!("nothing to require: no 'requires' entries in {CONFIG_FILE}");
            }
            let client = Client::from_config(&config)?.with_stream(stream.resolve()?);
            for entry in &config.requires {
                println!(
                    "Requiring {} endpoint(s) from '{}' at version {}...",
                    entry.endpoints.len(),
                    entry.service_name,
                    entry.version
                );
                let written = client.require(&config.service_name, entry)?;
                println!("Wrote {}", written.display());
            }
        }
    }

    Ok(())
}
