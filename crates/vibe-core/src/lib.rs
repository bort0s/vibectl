//! Core library for `vibectl` — a registry for the half-finished projects a
//! developer already has on disk.
//!
//! # Crate contract
//!
//! This crate is the reusable engine: manifest types, parsing, project
//! detection, and the render engine. Two rules define its boundary, and both
//! are enforced mechanically rather than by convention:
//!
//! 1. **No terminal I/O.** Nothing here writes to stdout or stderr. Results and
//!    diagnostics are *returned*, never printed. The `deny` attributes below
//!    make a stray `println!` a compile error. This is what allows a future
//!    Tauri frontend to consume the same code paths the CLI uses.
//! 2. **No argument parsing.** `clap` is a dependency of the `vibectl` crate
//!    only. This crate knows nothing about how it was invoked.
//!
//! # Status
//!
//! Pre-alpha. The workspace skeleton is in place (P0); manifest types land in
//! P1. There is no public API yet.

#![deny(clippy::print_stdout, clippy::print_stderr)]
