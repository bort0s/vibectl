//! Everything the user sees. `vibe-core` produced the values; this module
//! decides the wording.
//!
//! Progress and diagnostics go to **stderr**, data goes to **stdout**, always —
//! so `vibe scan --json > out.json` works with a live progress bar.

use std::io::Write;

use vibe_core::{ApplyOutcome, ApplyReport, CoreError, Diagnostic, FileOp, Severity, WritePlan};

/// Render a plan for a human, the way `--dry-run` shows it.
pub fn write_plan_human(out: &mut impl Write, plan: &WritePlan) -> std::io::Result<()> {
    if plan.is_empty() {
        return writeln!(out, "Nothing to do.");
    }

    writeln!(out, "Plan ({} operations):", plan.ops.len())?;
    for op in &plan.ops {
        match op {
            FileOp::CreateDir { path } => {
                writeln!(out, "  create dir   {}", path.display())?;
            }
            FileOp::CreateFile { path, contents } => {
                writeln!(
                    out,
                    "  create file  {} ({} lines)",
                    path.display(),
                    contents.lines().count()
                )?;
            }
            FileOp::UpdateFile { path, .. } => {
                writeln!(out, "  update file  {}", path.display())?;
            }
            // `FileOp` is #[non_exhaustive]; P5 adds validated git operations.
            // Printing the path rather than skipping the line matters: a
            // dry-run that silently omitted an operation would understate what
            // is about to happen, which is the one thing a dry run must never
            // do.
            other => {
                writeln!(out, "  (operation)  {}", other.path().display())?;
            }
        }
    }

    // Show the contents of a file being created. Seeing the manifest that is
    // about to be written is most of the value of a dry run.
    for op in &plan.ops {
        if let FileOp::CreateFile { path, contents } = op {
            writeln!(out, "\n--- {} ---", path.display())?;
            for line in contents.lines() {
                writeln!(out, "  {line}")?;
            }
        }
    }
    Ok(())
}

pub fn write_apply_human(out: &mut impl Write, report: &ApplyReport) -> std::io::Result<()> {
    for path in &report.applied {
        writeln!(out, "  wrote {path}")?;
    }
    for skipped in &report.skipped {
        writeln!(out, "  skipped {} (already present)", skipped.path)?;
    }
    match &report.outcome {
        ApplyOutcome::Completed => Ok(()),
        // Cancellation is a success outcome. It says how far it got so the user
        // can re-run, rather than reading as a failure.
        ApplyOutcome::Cancelled { after_ops } => writeln!(
            out,
            "\nCancelled after {after_ops} operations. Re-run to finish."
        ),
        _ => writeln!(
            out,
            "\nFinished with an outcome this build cannot describe."
        ),
    }
}

/// Turn a structured diagnostic into a sentence.
///
/// The catalogue lives here, not in core: core emits a stable code plus named
/// params and this decides the English.
pub fn diagnostic_line(d: &Diagnostic) -> String {
    let label = match d.severity {
        Severity::Note => "note",
        Severity::Warn => "warning",
        // A severity this build does not know is reported at the higher of the
        // two it does. Under-reporting an unknown severity is the worse error.
        _ => "warning",
    };
    let body = match d.code {
        vibe_core::report::W_SCHEMA_MINOR_NEWER => {
            let found = d.params.get("found").map_or("?", String::as_str);
            format!(
                "{found} manifests use a newer schema minor than this build knows; \
                 unrecognised fields were left untouched. Upgrade vibe to use them."
            )
        }
        other => other.to_string(),
    };
    match &d.subject {
        Some(s) => format!("{label}: {body} ({s})"),
        None => format!("{label}: {body}"),
    }
}

/// Human-facing rendering of a core error, including the remediation sentence
/// that `vibe-core` deliberately does not carry.
pub fn error_lines(err: &CoreError) -> String {
    let mut s = format!("error: {err}");
    if let Some(hint) = hint_for(err) {
        s.push_str(&format!("\n  hint: {hint}"));
    }
    s
}

fn hint_for(err: &CoreError) -> Option<&'static str> {
    match err {
        CoreError::SchemaMajorMismatch { .. } => {
            Some("upgrade vibe; this manifest was written by a newer build")
        }
        CoreError::TargetExists { .. } => {
            Some("`vibe new` will not adopt an existing directory — use `vibe scan` for that")
        }
        CoreError::PlanStale { .. } => Some("the file changed after the plan was built; re-run"),
        CoreError::PathInsideGitDir { .. } => {
            Some("this is a bug in vibe — it should never plan a write inside .git/")
        }
        _ => None,
    }
}
