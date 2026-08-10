use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};

use vibe_core::{Event, Reporter};

use crate::output;

/// Streams progress to **stderr**, so `--json` on stdout stays clean.
///
/// Also owns the cancellation flag. Core polls `should_cancel` at operation
/// boundaries and returns a *successful* report saying where it stopped —
/// Ctrl-C is not an error here, it is a user decision.
///
/// Note on what is deliberately not here yet: ADR-0002 §3 requires that a run
/// over many forward-versioned manifests print **one** warning rather than one
/// per file, and that the coalescing decision live in this crate because core
/// does not know what a "run" is. No P1 command reads an existing manifest, so
/// nothing emits that diagnostic and there is nothing to coalesce. The
/// grouping lands in P3 alongside `list`/`scan`, which are its first callers —
/// writing it now would mean shipping a tested function with no caller.
#[derive(Debug)]
pub struct TermReporter {
    quiet: bool,
    cancelled: AtomicBool,
}

impl TermReporter {
    pub fn new(quiet: bool) -> Self {
        Self {
            quiet,
            cancelled: AtomicBool::new(false),
        }
    }
}

impl Reporter for TermReporter {
    fn event(&self, ev: Event) {
        if self.quiet {
            return;
        }
        if let Event::Diagnostic(d) = ev {
            let _ = writeln!(std::io::stderr(), "{}", output::diagnostic_line(&d));
        }
    }

    fn should_cancel(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }
}
