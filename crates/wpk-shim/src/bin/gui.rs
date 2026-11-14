#![windows_subsystem = "windows"]

fn main() {
    std::process::exit(wpk_shim::run(wpk_shim::Variant::Gui));
}
