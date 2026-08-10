//! `vibe` — command-line frontend for [`vibe_core`].
//!
//! This crate owns argument parsing, terminal rendering, and human-facing error
//! messages. All logic lives in `vibe-core`.

fn main() {
    // P0 is the workspace skeleton: no commands are wired up yet. `clap` and the
    // command tree arrive in P1 alongside `vibe new`.
    eprintln!(
        "vibe {} — skeleton build, no commands implemented yet.",
        env!("CARGO_PKG_VERSION")
    );
    eprintln!("Planned CLI surface: new, scan, list, show, sync, render, archive.");
    eprintln!("See README.md for status.");
}
