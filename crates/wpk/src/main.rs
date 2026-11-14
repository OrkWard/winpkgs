use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, bail};
use clap::{Parser, Subcommand};
use libwpk::bucket::Bucket;
use libwpk::{Layout, Wpk, installed};

mod ui;

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
    /// Search packages in buckets by name or command
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
    /// Show the target of a command
    Which { name: String },
    /// Rewrite shims and shortcuts of all packages, e.g. after a shim update
    Relink,
}

#[derive(Subcommand)]
enum BucketCommand {
    /// Clone a bucket, at the lowest priority
    Add { name: String, url: String },
    /// Remove a bucket
    Rm { name: String },
    /// List buckets in priority order
    List,
}

#[derive(Subcommand)]
enum CacheCommand {
    /// List cached downloads, of packages matching QUERY if given
    List { query: Option<String> },
    /// Delete cached downloads of packages matching QUERY, or `*` for all
    Rm { query: String },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let layout = Layout::from_env()?;
    let ui = ui::Ui::new();
    match cli.command {
        Command::Init => {
            let wpk = Wpk::init(layout, &ui)?;
            ui.finish();
            println!("wpk is ready in {}", wpk.layout.root().display());
            println!("Add {} to your PATH.", wpk.layout.bin().display());
            Ok(())
        }
        Command::Update => update(&Wpk::open(layout)?),
        Command::Upgrade { packages } => {
            let result = Wpk::open(layout)?.upgrade(&packages, &ui);
            ui.finish();
            Ok(result?)
        }
        Command::Install { packages } => {
            let result = Wpk::open(layout)?.install(&packages, &ui);
            ui.finish();
            Ok(result?)
        }
        Command::Uninstall { packages, purge } => {
            Ok(Wpk::open(layout)?.uninstall(&packages, purge, &ui)?)
        }
        Command::List => list(&layout),
        Command::Search { query } => search(&Wpk::open(layout)?, &query),
        Command::Info { package } => info(&Wpk::open(layout)?, &package),
        Command::Hold { packages } => hold(&layout, &packages, true),
        Command::Unhold { packages } => hold(&layout, &packages, false),
        Command::Bucket(command) => bucket(Wpk::open(layout)?, command),
        Command::Cache(command) => cache(&layout, command),
        Command::Gc => gc(&layout),
        Command::Which { name } => which(&layout, &name),
        Command::Relink => {
            let result = Wpk::open(layout)?.relink(&ui);
            ui.finish();
            Ok(result?)
        }
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
    })?;
    if failed.into_inner() {
        bail!("some buckets failed to update");
    }
    Ok(())
}

fn list(layout: &Layout) -> anyhow::Result<()> {
    let packages = installed::list(layout, |name, e| eprintln!("{name}: {e}"))?;
    let width = packages.iter().map(|p| p.name.len()).max().unwrap_or(0);
    let vwidth = packages
        .iter()
        .map(|p| p.receipt.version.len())
        .max()
        .unwrap_or(0);
    for p in packages {
        let held = if p.held { "  held" } else { "" };
        println!(
            "{:width$}  {:vwidth$}  {}{held}",
            p.name, p.receipt.version, p.receipt.bucket
        );
    }
    Ok(())
}

fn search(wpk: &Wpk, query: &str) -> anyhow::Result<()> {
    let mut found = wpk.search(query)?;
    found.sort_by(|a, b| a.name.cmp(&b.name).then(a.bucket.cmp(&b.bucket)));
    for c in found {
        let note = match c.plan() {
            Ok(_) => String::new(),
            Err(reason) => format!("  (unsupported: {reason})"),
        };
        println!("{}  {}{note}", c.ident(), c.manifest.version);
    }
    Ok(())
}

fn info(wpk: &Wpk, query: &str) -> anyhow::Result<()> {
    let c = wpk.find(query)?;
    let m = &c.manifest;
    println!("Name:        {}", c.name);
    println!("Bucket:      {}", c.bucket);
    println!("Version:     {}", m.version);
    if let Some(d) = &m.description {
        println!("Description: {d}");
    }
    if let Some(h) = &m.homepage {
        println!("Homepage:    {h}");
    }
    if let Some(l) = &m.license {
        println!("License:     {l}");
    }
    if let Some(i) = installed::get(&wpk.layout, &c.name)? {
        let held = if i.held { ", held" } else { "" };
        println!(
            "Installed:   {} from {}{held}",
            i.receipt.version, i.receipt.bucket
        );
    }
    match c.plan() {
        Ok(plan) => {
            let bins: Vec<&str> = plan.bins.iter().map(|b| b.name.as_str()).collect();
            if !bins.is_empty() {
                println!("Commands:    {}", bins.join(", "));
            }
            let shortcuts: Vec<&str> = plan.shortcuts.iter().map(|s| s.name.as_str()).collect();
            if !shortcuts.is_empty() {
                println!("Shortcuts:   {}", shortcuts.join(", "));
            }
        }
        Err(reason) => println!("Unsupported: {reason}"),
    }
    if !m.depends.0.is_empty() {
        println!("Depends:     {}", m.depends.0.join(", "));
    }
    for line in &m.notes.0 {
        println!("Note:        {line}");
    }
    Ok(())
}

fn hold(layout: &Layout, packages: &[String], held: bool) -> anyhow::Result<()> {
    let _lock = libwpk::install::lock(layout)?;
    for name in packages {
        installed::set_held(layout, &name.to_ascii_lowercase(), held)?;
    }
    Ok(())
}

fn bucket(mut wpk: Wpk, command: BucketCommand) -> anyhow::Result<()> {
    match command {
        BucketCommand::Add { name, url } => wpk.bucket_add(&name, &url)?,
        BucketCommand::Rm { name } => wpk.bucket_remove(&name)?,
        BucketCommand::List => {
            for entry in &wpk.config.buckets {
                let state = match Bucket::open(&wpk.layout, &entry.name) {
                    Ok(bucket) => bucket
                        .manifests()
                        .map_or_else(|e| e.to_string(), |m| format!("{} manifests", m.len())),
                    Err(_) => "not cloned, run `wpk update`".to_owned(),
                };
                println!("{}\t{}\t{state}", entry.name, entry.url);
            }
        }
    }
    Ok(())
}

fn cache(layout: &Layout, command: CacheCommand) -> anyhow::Result<()> {
    let entries = |query: Option<&str>| -> anyhow::Result<Vec<(std::path::PathBuf, String, u64)>> {
        let Ok(dir) = std::fs::read_dir(layout.cache()) else {
            return Ok(Vec::new());
        };
        let query = query.filter(|q| *q != "*").map(str::to_ascii_lowercase);
        let mut out = Vec::new();
        for entry in dir.flatten() {
            let file = entry.file_name().to_string_lossy().into_owned();
            let package = file.split('#').next().unwrap_or_default().to_owned();
            if query
                .as_ref()
                .is_some_and(|q| !package.to_ascii_lowercase().contains(q.as_str()))
            {
                continue;
            }
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            out.push((entry.path(), file, size));
        }
        out.sort_by(|a, b| a.1.cmp(&b.1));
        Ok(out)
    };
    match command {
        CacheCommand::List { query } => {
            let files = entries(query.as_deref())?;
            let total: u64 = files.iter().map(|f| f.2).sum();
            for (_, file, size) in &files {
                println!("{:>10}  {file}", ui::bytes(*size));
            }
            println!("{} files, {}", files.len(), ui::bytes(total));
        }
        CacheCommand::Rm { query } => {
            let _lock = libwpk::install::lock(layout)?;
            let files = entries(Some(&query))?;
            for (path, _, _) in &files {
                std::fs::remove_file(path).with_context(|| path.display().to_string())?;
            }
            println!("removed {} files", files.len());
        }
    }
    Ok(())
}

fn gc(layout: &Layout) -> anyhow::Result<()> {
    let _lock = libwpk::install::lock(layout)?;
    let report = libwpk::gc::run(layout)?;
    for path in &report.removed {
        println!("removed {}", path.display());
    }
    for (path, why) in &report.kept {
        println!("kept {}: {why}", path.display());
    }
    Ok(())
}

fn which(layout: &Layout, name: &str) -> anyhow::Result<()> {
    match libwpk::shim::read(&layout.bin(), name)? {
        Some(shim) => {
            println!("{}", shim.path);
            Ok(())
        }
        None => bail!("'{name}' is not a wpk command"),
    }
}
