//! Core library for `vibectl` — a registry for the half-finished projects a
//! developer already has on disk.
//!
//! # Crate contract
//!
//! This crate is the reusable engine: manifest types, parsing, project
//! detection, and the render engine. Two rules define its boundary. Both are
//! checked in CI; neither is fully enforced by `cargo build` alone, so the
//! exact coverage is worth stating.
//!
//! 1. **No terminal I/O.** Nothing here writes to stdout or stderr. Results and
//!    diagnostics are *returned*, never printed — which is what allows a future
//!    Tauri frontend to consume the same code paths the CLI uses.
//!
//!    The `deny` attributes below fail the build **under `cargo clippy`**, not
//!    under a plain `cargo build`: these are clippy lints, and rustc ignores
//!    them. CI runs clippy with `-D warnings`, so the rule holds there. They
//!    also only cover the `print!`/`println!` macro family — an explicit
//!    `writeln!(std::io::stdout(), ..)` slips past them and is caught by review
//!    alone.
//! 2. **No argument parsing.** `clap` belongs to the `vibectl` crate. Nothing
//!    in the language prevents adding it here, so this half of the boundary is
//!    asserted by a CI step that inspects `cargo tree -p vibe-core`.
//!
//! # Status
//!
//! Pre-alpha. The workspace skeleton is in place (P0); manifest types land in
//! P1. There is no public API yet.

#![deny(clippy::print_stdout, clippy::print_stderr)]
