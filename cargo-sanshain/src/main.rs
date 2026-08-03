use clap::{Parser, Subcommand};
use sanshain_rs::{resolve_stability, Client, Config};

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
    },
    /// Download the pinned endpoint snippet(s) declared in the manifest
    Require,
}

fn main() -> anyhow::Result<()> {
    // Cargo subcommands receive the subcommand name as the first argument when called via `cargo sanshain`
    // but when called directly as `cargo-sanshain`, they don't.
    // Clap's `bin_name = "cargo"` and the `CargoCli` enum handle this correctly.
    let CargoCli::Sanshain(args) = CargoCli::parse();

    let config = Config::from_manifest()?;
    let client = Client::from_config(&config)?;

    match args.command {
        Commands::Provide { ga } => {
            if config.provides.is_empty() {
                anyhow::bail!(
                    "nothing to provide: no 'provide'/'provides' entries in [package.metadata.sanshain]"
                );
            }
            let stability = resolve_stability(ga);
            for entry in &config.provides {
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
            }
        }
        Commands::Require => {
            if config.requires.is_empty() {
                anyhow::bail!(
                    "nothing to require: no 'requires' entries in [package.metadata.sanshain]"
                );
            }
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
