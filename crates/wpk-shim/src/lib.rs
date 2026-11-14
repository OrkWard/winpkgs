//! Shared code of the console and gui shims.

/// Which shim binary is running.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Variant {
    /// Waits for the target and returns its exit code.
    Console,
    /// Starts the target and exits.
    Gui,
}

/// Run the target of this shim and return the exit code for the shim.
pub fn run(_variant: Variant) -> i32 {
    eprintln!("wpk-shim: not implemented yet");
    1
}
