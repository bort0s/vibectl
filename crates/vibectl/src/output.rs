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

/// Render a scan for a human.
///
/// The design rule this obeys: an undetectable field prints as `—`, never as a
/// plausible-looking value and never as the word "unknown" dressed up as data.
/// `--suggestions` is what surfaces the things that were found but deliberately
/// not written.
pub fn write_scan_human(
    out: &mut impl Write,
    report: &vibe_core::ScanReport,
    show_suggestions: bool,
) -> std::io::Result<()> {
    if report.projects.is_empty() {
        writeln!(out, "No projects found in {}.", report.roots.join(", "))?;
        return write_depth_note(out, report);
    }

    for p in &report.projects {
        let runtime = detected_or_dash(&p.detection.runtime);
        writeln!(out, "{}  {}", p.name, p.path)?;
        writeln!(out, "  stack     {runtime}")?;
        if !p.detection.frameworks.is_empty() {
            writeln!(out, "  uses      {}", p.detection.frameworks.join(", "))?;
        }
        if !p.detection.services.is_empty() {
            writeln!(out, "  services  {}", p.detection.services.join(", "))?;
        }
        writeln!(out, "  remote    {}", detected_or_dash(&p.detection.remote))?;
        writeln!(
            out,
            "  commit    {}",
            detected_or_dash(&p.detection.last_commit)
        )?;
        if !p.detection.env_required.is_empty() {
            writeln!(out, "  env       {}", p.detection.env_required.join(", "))?;
        }
        if let Some(err) = &p.manifest_error {
            writeln!(out, "  manifest  unreadable: {}", err.message)?;
        }
        if p.index_truncated {
            writeln!(
                out,
                "  note      too many files to index fully; absence is not evidence here"
            )?;
        }

        if show_suggestions && !p.detection.suggestions.is_empty() {
            writeln!(out, "  not written:")?;
            for s in &p.detection.suggestions {
                writeln!(
                    out,
                    "    {:?} = {}  ({:?}, from {})",
                    s.field, s.value, s.why, s.detector
                )?;
            }
        }
        writeln!(out)?;
    }

    writeln!(
        out,
        "{} project(s) in {}ms",
        report.projects.len(),
        report.elapsed_ms
    )?;
    let unreadable = report.unreadable();
    if unreadable > 0 {
        writeln!(out, "{unreadable} manifest(s) could not be read")?;
    }
    let suggestions = report.suggestion_count();
    if suggestions > 0 && !show_suggestions {
        writeln!(
            out,
            "{suggestions} value(s) found but not written — re-run with --suggestions"
        )?;
    }
    write_depth_note(out, report)
}

/// Say where the walk stopped looking.
///
/// "No projects found" after declining to descend is the same substitution the
/// detectors are forbidden from making — absence of a look reported as absence
/// of a thing.
fn write_depth_note(out: &mut impl Write, report: &vibe_core::ScanReport) -> std::io::Result<()> {
    let n = report.depth_limited.len();
    if n == 0 {
        return Ok(());
    }
    writeln!(
        out,
        "{n} director{} not searched below the depth limit. A project there would not have been found; raise --depth to look.",
        if n == 1 { "y was" } else { "ies were" }
    )?;
    for dir in report.depth_limited.iter().take(3) {
        writeln!(out, "  {dir}")?;
    }
    if n > 3 {
        writeln!(out, "  ...and {} more", n - 3)?;
    }
    Ok(())
}

/// An em dash, not a guess and not the string "unknown".
///
/// Printing `unknown` in a value column invites it being read as data — and
/// somebody would eventually grep for it. A dash reads as absence.
fn detected_or_dash<T: std::fmt::Display>(d: &vibe_core::Detected<T>) -> String {
    match d.value() {
        Some(v) => v.to_string(),
        None => "—".to_owned(),
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
