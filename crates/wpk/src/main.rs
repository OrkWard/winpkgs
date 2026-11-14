use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::bail;
use clap::{Parser, Subcommand};
use libwpk::{Layout, Wpk};

#[derive(Parser)]
#[command(name = "wpk", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create the wpk layout, install wpk itself, 7zip and innounp
    Init,
    /// Update buckets
    Update,
    /// Upgrade packages, all that are not held when none is given
    Upgrade { packages: Vec<String> },
    /// Install packages
    Install {
        #[arg(required = true)]
        packages: Vec<String>,
    },
    /// Uninstall packages
    Uninstall {
        #[arg(required = true)]
        packages: Vec<String>,
        /// Also delete persisted data in var
        #[arg(long)]
        purge: bool,
    },
    /// List installed packages
    List,
    /// Search packages in buckets
    Search { query: String },
    /// Show package details
    Info { package: String },
    /// Keep packages at their installed version
    Hold {
        #[arg(required = true)]
        packages: Vec<String>,
    },
    /// Allow held packages to upgrade again
    Unhold {
        #[arg(required = true)]
        packages: Vec<String>,
    },
    /// Manage buckets
    #[command(subcommand)]
    Bucket(BucketCommand),
    /// Manage the download cache
    #[command(subcommand)]
    Cache(CacheCommand),
    /// Remove old versions now
    Gc,
    /// Show the target of a shim
    Which { name: String },
}

#[derive(Subcommand)]
enum BucketCommand {
    Add { name: String, url: String },
    Rm { name: String },
    List,
}

#[derive(Subcommand)]
enum CacheCommand {
    List { query: Option<String> },
    Rm { query: String },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let layout = Layout::from_env()?;
    match cli.command {
        Command::Update => update(&Wpk::open(layout)?),
        Command::Bucket(command) => bucket(Wpk::open(layout)?, command),
        _ => bail!("not implemented yet"),
    }
}

fn update(wpk: &Wpk) -> anyhow::Result<()> {
    let failed = AtomicBool::new(false);
    wpk.update(|name, result| match result {
        Ok(()) => println!("{name}: updated"),
        Err(e) => {
            eprintln!("{name}: {e}");
            failed.store(true, Ordering::Relaxed);
        }
    });
    if failed.into_inner() {
        bail!("some buckets failed to update");
    }
    Ok(())
}

fn bucket(mut wpk: Wpk, command: BucketCommand) -> anyhow::Result<()> {
    match command {
        BucketCommand::Add { name, url } => wpk.bucket_add(&name, &url)?,
        BucketCommand::Rm { name } => wpk.bucket_remove(&name)?,
        BucketCommand::List => {
            for entry in &wpk.config.buckets {
                let state = match libwpk::bucket::Bucket::open(&wpk.layout, &entry.name) {
                    Ok(bucket) => bucket.manifests().map_or_else(
                        |e| e.to_string(),
                        |m| format!("{} manifests", m.len()),
                    ),
                    Err(_) => "not cloned, run `wpk update`".to_owned(),
                };
                println!("{}\t{}\t{state}", entry.name, entry.url);
            }
        }
    }
    Ok(())
}
