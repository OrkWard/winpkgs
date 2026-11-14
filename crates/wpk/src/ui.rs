//! Terminal output for sync events.

use std::collections::BTreeMap;
use std::io::{IsTerminal, Write};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use libwpk::install::{Event, Reporter};

pub struct Ui {
    tty: bool,
    progress: Mutex<Progress>,
}

#[derive(Default)]
struct Progress {
    /// Bytes done and total per download.
    files: BTreeMap<(String, String), (u64, Option<u64>)>,
    drawn: Option<Instant>,
    line: bool,
}

impl Ui {
    pub fn new() -> Ui {
        Ui {
            tty: std::io::stderr().is_terminal(),
            progress: Mutex::new(Progress::default()),
        }
    }

    /// Clear the progress line.
    pub fn finish(&self) {
        let mut p = self.progress.lock().unwrap();
        clear(&mut p);
    }

    fn print(&self, text: &str) {
        let mut p = self.progress.lock().unwrap();
        clear(&mut p);
        println!("{text}");
    }
}

fn clear(p: &mut Progress) {
    if p.line {
        eprint!("\r\x1b[2K");
        let _ = std::io::stderr().flush();
        p.line = false;
    }
}

impl Reporter for Ui {
    fn event(&self, event: Event<'_>) {
        match event {
            Event::Planned {
                name,
                version,
                from,
            } => match from {
                Some(from) => self.print(&format!("upgrade {name} {from} -> {version}")),
                None => self.print(&format!("install {name} {version}")),
            },
            Event::Skipped { name, reason } => self.print(&format!("{name}: skipped, {reason}")),
            Event::Progress {
                name,
                file,
                done,
                total,
            } => {
                if !self.tty {
                    return;
                }
                let mut p = self.progress.lock().unwrap();
                p.files
                    .insert((name.to_owned(), file.to_owned()), (done, total));
                if p.drawn
                    .is_some_and(|t| t.elapsed() < Duration::from_millis(100))
                {
                    return;
                }
                p.drawn = Some(Instant::now());
                let done: u64 = p.files.values().map(|f| f.0).sum();
                let total: u64 = p.files.values().filter_map(|f| f.1).sum();
                let count = p.files.len();
                eprint!(
                    "\r\x1b[2Kdownloading {count} file(s): {} / {}",
                    bytes(done),
                    bytes(total)
                );
                let _ = std::io::stderr().flush();
                p.line = true;
            }
            Event::Downloaded { name, file } => {
                let mut p = self.progress.lock().unwrap();
                if let Some(f) = p.files.get_mut(&(name.to_owned(), file.to_owned())) {
                    f.0 = f.1.unwrap_or(f.0);
                }
            }
            Event::Installed { name, version } => {
                self.print(&format!("{name} {version} installed"))
            }
            Event::Uninstalled { name } => self.print(&format!("{name} uninstalled")),
            Event::Relinked { name } => self.print(&format!("{name} relinked")),
            Event::Failed { name, error } => {
                let mut p = self.progress.lock().unwrap();
                clear(&mut p);
                eprintln!("error: {name} failed: {error}");
            }
            Event::Notes { name, lines } => {
                let mut text = format!("notes from {name}:");
                for line in lines {
                    text.push_str(&format!("\n  {line}"));
                }
                self.print(&text);
            }
            Event::EnvSet { name, vars } => {
                let mut text =
                    format!("{name} needs these environment variables; set them in your shell:");
                for (key, value) in vars {
                    text.push_str(&format!("\n  {key}={value}"));
                }
                self.print(&text);
            }
            Event::Suggest { name, packages } => {
                self.print(&format!("{name} suggests: {}", packages.join(", ")));
            }
            Event::Warning(message) => {
                let mut p = self.progress.lock().unwrap();
                clear(&mut p);
                eprintln!("warning: {message}");
            }
        }
    }
}

pub fn bytes(n: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut value = n as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{n} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}
