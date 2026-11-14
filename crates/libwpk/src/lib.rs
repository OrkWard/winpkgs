//! Core of winpkgs: everything `wpk` does that is not terminal I/O.
//!
//! See `DESIGN.md` at the repository root for the file layout and the
//! supported subset of the Scoop manifest format.

pub mod archive;
pub mod bucket;
pub mod config;
pub mod download;
pub mod error;
pub mod fs;
pub mod gc;
mod init;
pub mod install;
pub mod installed;
pub mod layout;
pub mod manifest;
pub mod persist;
pub mod query;
pub mod shim;
pub mod shortcut;
#[cfg(test)]
mod testutil;
mod wpk;

pub use config::Config;
pub use error::{Error, Result};
pub use layout::Layout;
pub use manifest::Manifest;
pub use wpk::Wpk;
