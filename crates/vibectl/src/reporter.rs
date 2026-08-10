use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};

use vibe_core::{Diagnostic, Event, Reporter};

use crate::output;

/// Streams progress to **stderr**, so `--json` on stdout stays clean.
///
/// Also owns the cancellation flag. Core polls `should_cancel` at operation
/// boundaries and returns a *successful* report saying where it stopped —
/// Ctrl-C is not an error here, it is a user decision.
///
/// Diagnostics are accumulated rather than printed as they arrive, because the
/// coalescing decision belongs here: core emits one per manifest since it does
/// not know what a "run" is, and a scan over thirty forward-versioned manifests
/// must print one line rather than thirty (ADR-0002 §3).
#[derive(Debug)]
pub struct TermReporter {
    quiet: bool,
    cancelled: AtomicBool,
    diagnostics: std::sync::Mutex<Vec<Diagnostic>>,
}

impl TermReporter {
    pub fn new(quiet: bool) -> Self {
        Self {
            quiet,
            cancelled: AtomicBool::new(false),
            diagnostics: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// One line per distinct diagnostic code, with a count.
    #[must_use]
    pub fn summarize(&self) -> Vec<String> {
        let diagnostics = self
            .diagnostics
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();

        let mut by_code: std::collections::BTreeMap<&str, usize> =
            std::collections::BTreeMap::new();
        for d in &diagnostics {
            *by_code.entry(d.code).or_default() += 1;
        }
        by_code
            .into_iter()
            .filter_map(|(code, count)| {
                let mut d = diagnostics.iter().find(|d| d.code == code)?.clone();
                // The subject is dropped deliberately: naming one of thirty
                // files would imply the others are fine.
                d.subject = None;
                d.params.insert("count".to_owned(), count.to_string());
                Some(output::diagnostic_line(&d))
            })
            .collect()
    }

    /// Print the coalesced diagnostics. Call once, at the end of a command.
    pub fn flush(&self) {
        if self.quiet {
            return;
        }
        for line in self.summarize() {
            let _ = writeln!(std::io::stderr(), "{line}");
        }
    }
}

impl Reporter for TermReporter {
    fn event(&self, ev: Event) {
        if let Event::Diagnostic(d) = ev {
            self.diagnostics
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(d);
        }
    }

    fn should_cancel(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }
}
