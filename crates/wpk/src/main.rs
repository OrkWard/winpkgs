use anyhow::bail;
use clap::{Parser, Subcommand};

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
    let _cli = Cli::parse();
    let _layout = libwpk::Layout::from_env()?;
    bail!("not implemented yet")
}
