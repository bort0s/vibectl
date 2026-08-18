//! `vibe monitor hook` — the process Claude Code spawns when an event fires.
//!
//! ADR-0011 §7a's transport, at the end that actually writes. Everything here
//! is shaped by one fact: **this process is a child of somebody else's tool**,
//! and its exit status is read by that tool rather than by a person.
//!
//! # The exit contract is imposed from outside, which is a new shape here
//!
//! `vibectl`'s own contract (ADR-0002 §3) assigns **2 = partial**. Claude Code
//! reads a hook's exit **2** as a *blocking* error fed back to the agent, which
//! is precisely what ADR-0011 §7a decided this path must never do: the monitor
//! is additive, and *an observer that can stop the subject is not one*.
//!
//! So two contracts meet in one binary and the external one wins on this path.
//! It is enforced structurally rather than remembered: [`HookExit`] has **no
//! variant for 2**, so emitting one does not compile.
//!
//! **And `clap` exits 2 on a usage error**, which would hand Claude Code a
//! blocking error for a typo in a settings file. That is why [`hook_main`] is
//! dispatched from raw argv *before* `Cli::parse()` and parses its own
//! arguments with `try_parse_from`, mapping every failure to exit 1.
//!
//! # A panic is silent loss from our side
//!
//! An unhandled panic exits 101: no record, the agent continues, and nobody
//! learns why. That is §7's non-delivery hazard produced by our own crash
//! rather than by the channel, so the panic hook emits a structured line before
//! the process dies — converting a silent loss into a reported one. It cannot
//! change the exit code, and 101 is at least not 2.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use clap::Parser;

use vibe_core::monitor::{CONTRACT_VERSION, SystemStamps, WriteOutcome, Writer, WriterIdentity};

/// Every exit this path may produce.
///
/// **A closed enum with no `Partial`.** ADR-0005 §10 rule 1's technique and the
/// same one the record filename uses for *"no agent"*: the dangerous value is
/// unrepresentable rather than merely avoided, so a future edit that wants to
/// report *partial* has to change a type — a visible diff — instead of adding a
/// branch nobody reviews.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookExit {
    /// The record is on disk.
    Delivered,
    /// It is not, and the reason went to stderr. Never 2.
    NotDelivered,
}

impl From<HookExit> for ExitCode {
    fn from(e: HookExit) -> Self {
        match e {
            HookExit::Delivered => ExitCode::from(0),
            HookExit::NotDelivered => ExitCode::from(1),
        }
    }
}

/// `vibe monitor hook`.
#[derive(Debug, Parser)]
#[command(
    name = "vibe monitor hook",
    about = "Append one Claude Code hook payload to the sink"
)]
pub struct HookArgs {
    /// The writer identity this hook declares. One per installed hook.
    #[arg(long)]
    pub identity: String,

    /// Where the sink lives.
    ///
    /// **Declared, never resolved.** ADR-0011 §7a: the design's attribution
    /// story is *read the payload, never the environment*, and `ProjectDirs`
    /// would put the sink's location on the environment — in a child process
    /// whose environment ADR-0011 §3 measured as contaminated by its parent's.
    #[arg(long)]
    pub sink: PathBuf,

    /// The contract version the installed config declares.
    #[arg(long)]
    pub contract: String,

    /// Panic deliberately, to prove the panic reporter fires.
    ///
    /// An **argument** rather than an environment variable, so it cannot arrive
    /// ambiently — ADR-0008 §9 rejected a build flag because `RUSTFLAGS` and
    /// `.cargo/config.toml` are inherited from parent directories. An argument
    /// has to be typed into a hook config by someone.
    #[arg(long, hide = true)]
    pub panic_probe: bool,
}

/// Whether raw argv is a hook invocation.
///
/// Read from argv rather than from a parsed `Cli`, because parsing is the thing
/// that can exit 2.
#[must_use]
pub fn is_hook_invocation(args: &[String]) -> bool {
    let mut rest = args.iter().skip(1);
    rest.next().map(String::as_str) == Some("monitor")
        && rest.next().map(String::as_str) == Some("hook")
}

/// The line a panic emits before the process dies.
///
/// Extracted so it can be asserted on directly: the *body* is testable even
/// where installing the hook is harder to observe.
#[must_use]
pub fn panic_report(message: &str, location: Option<(&str, u32)>) -> String {
    let where_ = location.map_or_else(
        || "unknown location".to_owned(),
        |(f, l)| format!("{f}:{l}"),
    );
    format!(
        "vibe monitor hook: PANIC at {where_}: {message}\n\
         The hook crashed, so this event was NOT written and nothing else will \
         report it. The agent is unaffected and continues."
    )
}

fn install_panic_reporter() {
    std::panic::set_hook(Box::new(|info| {
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| (*s).to_owned())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "non-string panic payload".to_owned());
        let location = info.location().map(|l| (l.file(), l.line()));
        let mut stderr = std::io::stderr();
        let _ = writeln!(stderr, "{}", panic_report(&message, location));
    }));
}

/// Run the hook. Returns rather than exits, so it is callable from a test.
pub fn hook_main(argv: &[String]) -> HookExit {
    install_panic_reporter();

    // `try_parse_from` reads its first element as argv0, so the real arguments
    // start after the binary name AND the two subcommand words this path was
    // dispatched on.
    let synthetic =
        std::iter::once("vibe monitor hook".to_owned()).chain(argv.iter().skip(3).cloned());
    let args = match HookArgs::try_parse_from(synthetic) {
        Ok(a) => a,
        Err(e) => {
            // clap would exit 2 here. On this path 2 is a blocking error to the
            // agent, so the usage message goes to stderr and the code is 1.
            let mut stderr = std::io::stderr();
            let _ = writeln!(stderr, "vibe monitor hook: {e}");
            return HookExit::NotDelivered;
        }
    };

    if args.panic_probe {
        panic!("--panic-probe was passed");
    }

    let mut stderr = std::io::stderr();

    // The write half of §7's contract check. The comparison half belongs to
    // `vibe monitor install`, which reads the declared value out of each
    // settings file; what it compares against is this constant, and what it
    // gets from here is the shape: a declared string, reported and never
    // repaired, with the record stamped by the binary that actually wrote it.
    if args.contract != CONTRACT_VERSION {
        let _ = writeln!(
            stderr,
            "vibe monitor hook: contract mismatch — the installed hook declares \
             {declared:?} and this binary implements {implemented:?}. The record \
             is written and stamped {implemented:?}, which is what actually \
             produced the bytes. Nothing is repaired.",
            declared = args.contract,
            implemented = CONTRACT_VERSION
        );
    }

    let identity = match WriterIdentity::parse(&args.identity) {
        Ok(i) => i,
        Err(rejection) => {
            let _ = writeln!(
                stderr,
                "vibe monitor hook: the declared identity {:?} is not usable as \
                 a path component ({}). Validated at write as well as at \
                 install, because hand-installed hooks never pass through \
                 install.",
                args.identity,
                rejection.key()
            );
            return HookExit::NotDelivered;
        }
    };

    let mut payload = String::new();
    if let Err(e) = std::io::stdin().read_to_string(&mut payload) {
        let _ = writeln!(stderr, "vibe monitor hook: could not read stdin: {e}");
        return HookExit::NotDelivered;
    }

    let writer = Writer::new(&args.sink, identity, Arc::new(SystemStamps));
    match writer.append(&payload) {
        WriteOutcome::Written { .. } => HookExit::Delivered,
        WriteOutcome::Refused { reason } => {
            let _ = writeln!(
                stderr,
                "vibe monitor hook: refused ({}) — the event is LOST and nothing \
                 else will report it.",
                reason.key()
            );
            HookExit::NotDelivered
        }
        WriteOutcome::Failed {
            stage, io, path, ..
        } => {
            let _ = writeln!(
                stderr,
                "vibe monitor hook: write failed at {stage:?} on {} ({}, os={:?}) \
                 — the event is LOST and nothing else will report it.",
                path.display(),
                io.kind,
                io.os_code
            );
            HookExit::NotDelivered
        }
        // `WriteOutcome` is `#[non_exhaustive]`. A variant this build does not
        // know is reported as undelivered rather than assumed to be a success —
        // ADR-0001 §4's rule for meeting an unrecognised variant, pointed at the
        // arm where guessing would claim an event reached disk.
        other => {
            let _ = writeln!(
                stderr,
                "vibe monitor hook: this build does not recognise the write                  outcome {:?}, so it cannot report the event as written.                  Treating it as LOST.",
                other.key()
            );
            HookExit::NotDelivered
        }
    }
}
