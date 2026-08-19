//! Emit exactly what `vibe monitor install` will write, so it can be run.
//!
//! ADR-0011 §2's round 3d validated the emitted group end to end — accepted by
//! the loader, firing on all five lifecycle events — against a **hand-written**
//! fixture. The editor now generates that group, and *"the hand-written one
//! works"* is not a statement about the generated one: they are different
//! artifacts and nothing compared them.
//!
//! This exists so the measurement can run against the **editor's own output**,
//! including the case that matters — the group written **into an existing
//! file** rather than into a clean one. It is an example rather than a
//! subcommand because `vibe monitor install` does not exist yet and a flag on
//! the shipped binary that exists only for a measurement is a thing users can
//! find.
//!
//! usage: cargo run -p vibe-core --example emit_install_settings -- <settings.json> <exe> <sink> <identity>
//!
//! Reads the file if it is there, installs into it, and writes it back through
//! the same atomic replacement `apply` uses.

use std::path::PathBuf;

use vibe_core::monitor::{HookSpec, SettingsDocument, WriterIdentity, install, read_document};
use vibe_core::write_atomically;

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(target), Some(exe), Some(sink), Some(identity)) =
        (args.next(), args.next(), args.next(), args.next())
    else {
        eprintln!("usage: emit_install_settings <settings.json> <exe> <sink> <identity>");
        std::process::exit(2);
    };
    let target = PathBuf::from(target);

    let existing = read_document(&target).expect("the settings file is readable");
    let mut doc = match existing.as_deref() {
        Some(text) => match SettingsDocument::parse(text) {
            Ok(doc) => doc,
            Err(refusal) => {
                // Reported, never repaired — and nothing has been written, so
                // the file on disk is exactly as it was.
                eprintln!("refused ({}): {refusal:?}", refusal.key());
                std::process::exit(1);
            }
        },
        None => SettingsDocument::empty(),
    };

    let spec = HookSpec {
        command: PathBuf::from(exe),
        sink: PathBuf::from(sink),
        identity: WriterIdentity::parse(&identity).expect("identity is a path component"),
    };

    match install(&mut doc, &spec) {
        Ok(outcome) => {
            write_atomically(&target, &doc.render()).expect("write");
            println!("{}", outcome.key());
        }
        Err(refusal) => {
            eprintln!("refused ({}): {refusal:?}", refusal.key());
            std::process::exit(1);
        }
    }
}
