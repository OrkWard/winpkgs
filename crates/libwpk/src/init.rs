//! `wpk init`: the layout, wpk itself and the default buckets.

use crate::install::{Event, Reporter};
use crate::installed::{self, Receipt};
use crate::layout::SELF_PACKAGE;
use crate::manifest::Arch;
use crate::shim::{ShimExes, ShimFile};
use crate::{Config, Layout, Result, Wpk, shim};

/// Files wpk runs from, copied into `opt/wpk/<version>`.
const SELF_FILES: [&str; 3] = ["wpk.exe", "wpk-shim.exe", "wpk-shimw.exe"];

/// Packages `wpk init` installs so every archive format can be extracted.
pub const EXTRACTORS: [&str; 2] = ["7zip", "innounp"];

impl Wpk {
    /// Create the layout, install the running wpk into it, clone the
    /// configured buckets and install the extractors. Safe to run again.
    pub fn init(layout: Layout, reporter: &dyn Reporter) -> Result<Wpk> {
        for dir in [layout.bin(), layout.opt(), layout.var(), layout.state(), layout.cache().to_owned()] {
            crate::fs::create_dir_all(&dir)?;
        }
        let config_path = layout.config();
        let config = if config_path.exists() {
            Config::load(&config_path)?
        } else {
            let config = Config::default();
            config.save(&config_path)?;
            config
        };
        let shims = {
            let _lock = crate::install::lock(&layout)?;
            install_self(&layout)?
        };
        let wpk = Wpk { layout, config, shims };

        wpk.update(|name, result| match result {
            Ok(()) => {}
            Err(e) => reporter.event(Event::Warning(format!("bucket {name}: {e}"))),
        })?;
        let missing: Vec<String> = EXTRACTORS
            .iter()
            .filter(|p| installed::current(&wpk.layout, p).ok().flatten().is_none())
            .map(|p| p.to_string())
            .collect();
        for package in missing {
            if let Err(e) = wpk.install(std::slice::from_ref(&package), reporter) {
                reporter.event(Event::Warning(format!("{package}: {e}")));
            }
        }
        Ok(wpk)
    }
}

/// Copy the running wpk into `opt/wpk/<version>` unless that version is
/// already installed, and point `bin/wpk` at it. Returns the shims of the
/// installed copy.
fn install_self(layout: &Layout) -> Result<ShimExes> {
    let version = env!("CARGO_PKG_VERSION");
    let dir = layout.version(SELF_PACKAGE, version);
    let exe = std::env::current_exe().map_err(crate::fs::io_error(&dir))?;
    let from = exe.parent().expect("an exe has a parent dir");
    if installed::current(layout, SELF_PACKAGE)?.as_deref() != Some(version) {
        crate::fs::create_dir_all(&dir)?;
        for file in SELF_FILES {
            let (src, dest) = (from.join(file), dir.join(file));
            if src != dest {
                std::fs::copy(&src, &dest).map_err(crate::fs::io_error(&src))?;
            }
        }
        installed::write_receipt(
            layout,
            SELF_PACKAGE,
            &Receipt {
                version: version.to_owned(),
                bucket: SELF_PACKAGE.to_owned(),
                commit: None,
                arch: Arch::host(),
                installed_at: jiff::Timestamp::now(),
                explicit: true,
                bins: vec![SELF_PACKAGE.to_owned()],
                launchers: Vec::new(),
                shortcuts: Vec::new(),
            },
        )?;
    }
    let shims = ShimExes {
        console: dir.join("wpk-shim.exe"),
        gui: dir.join("wpk-shimw.exe"),
    };
    let target = ShimFile {
        path: dir.join("wpk.exe").display().to_string(),
        args: None,
        dir: None,
    };
    shim::write(&layout.bin(), SELF_PACKAGE, &target, &shims)?;
    installed::set_current(layout, SELF_PACKAGE, version)?;
    Ok(shims)
}
