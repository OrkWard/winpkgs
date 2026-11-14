//! Core of winpkgs: everything `wpk` does that is not terminal I/O.
//!
//! See `DESIGN.md` at the repository root for the file layout and the
//! supported subset of the Scoop manifest format.

pub mod config;
pub mod error;
pub mod layout;
pub mod manifest;

pub use config::Config;
pub use error::{Error, Result};
pub use layout::Layout;
pub use manifest::Manifest;
