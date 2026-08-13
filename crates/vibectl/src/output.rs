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

/// The label for a severity this build cannot place.
///
/// **Factored out so the text is testable.** The arm that reaches it needs a
/// `Severity` variant that cannot exist yet — a dependency's-contract case, like
/// `gh` honouring an alias — but the *body* has no such requirement. Splitting
/// them turns "the behaviour this change exists for is uncovered" into "the
/// message is covered, the one line of dispatch is not", which is a much smaller
/// thing for ADR-0008 §9 to carry.
///
/// The property it exists to hold: **it claims no rank.** `warning` and `note`
/// are positions in an ordering this build knows; an unrecognised severity has
/// none, and borrowing one is the constraint-5 substitution this whole change
/// removes.
fn unranked_severity_label(name: Option<&str>) -> String {
    // Backticked, because it is the name core sent rather than a word chosen
    // here — the same reason unknown manifest keys are quoted (ADR-0002 §5).
    name.map_or_else(
        || "unrecognised severity".to_owned(),
        |name| format!("unrecognised severity `{name}`"),
    )
}

/// Turn a structured diagnostic into a sentence.
///
/// The catalogue lives here, not in core: core emits a stable code plus named
/// params and this decides the English.
pub fn diagnostic_line(d: &Diagnostic) -> String {
    // **This label is a claim about the diagnostic, so an unknown severity does
    // not get to borrow one.** The previous version rendered any unrecognised
    // severity as `warning`, reasoning that under-reporting was the worse
    // error. That reasoning is sound and the conclusion was still wrong: it is
    // constraint 5 — inventing a plausible value for something that was not
    // inferred — committed in the renderer rather than in a manifest. A
    // `critical` from a newer core would have printed as `warning`, which is
    // not a hedge but a specific and false statement of rank.
    //
    // The honest label names the rank core actually sent, marked as one this
    // build cannot place. It is louder than `warning`, so nothing is
    // under-reported, and it claims no position in an ordering this build does
    // not know.
    let unranked;
    let label = match d.severity {
        Severity::Note => "note",
        Severity::Warn => "warning",
        _ => {
            // The variant names itself through its `Serialize` impl, which is
            // data rather than prose — the same route ADR-0002 §5 uses to
            // report keys a build does not understand. A serialisation that
            // fails leaves us with no name, and saying so beats guessing one.
            let name = serde_json::to_value(d.severity).ok();
            unranked = unranked_severity_label(name.as_ref().and_then(|v| v.as_str()));
            unranked.as_str()
        }
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
        // The two refusals must not read the same. One is a flag away; the
        // other is not reachable by any flag, and telling a user to try
        // `--force` on a file vibe will never adopt would be worse than saying
        // nothing (ADR-0007 §4).
        CoreError::RenderRefused { state, .. } => Some(match state {
            vibe_core::RenderState::Foreign => {
                "vibe did not generate this file, so it will not overwrite it — \
                 not even with --force. Move it aside if you want a generated one."
            }
            vibe_core::RenderState::Modified => {
                "you edited this generated file. Re-run with --force to discard \
                 those edits, or move the file aside to keep them."
            }
            // Named, rather than left to the wildcard. This arm's text was
            // written for `Unverifiable` and reached it through a `_`, which
            // meant the same sentence — and the same `--force` advice — was
            // also served to `Absent`, `Generated`, and every future variant.
            // Promising a remedy for a state you have not identified is the
            // `--force` half of the ADR-0007 §4 mistake: the two refusals must
            // not read the same, and neither must a third nobody has looked at.
            vibe_core::RenderState::Unverifiable => {
                "vibe's marker on this file carries a claim this build cannot \
                 evaluate — most likely a newer marker version. Upgrade vibe, or \
                 move the file aside. `--force` overwrites it."
            }
            // The genuine unknown: a state from a newer core. No remedy is
            // offered because none can be known to apply.
            _ => {
                "vibe refused to write this file for a reason this build does not \
                 understand. Upgrade vibe; do not assume --force applies."
            }
        }),
        _ => None,
    }
}

/// The registry table.
pub fn write_list_human(
    out: &mut impl Write,
    report: &vibe_core::ListReport,
) -> std::io::Result<()> {
    if report.projects.is_empty() {
        writeln!(out, "No projects.")?;
        if report.archived_hidden > 0 {
            writeln!(
                out,
                "{} archived - pass --all to include them.",
                report.archived_hidden
            )?;
        }
        return Ok(());
    }

    let w_name = report
        .projects
        .iter()
        .map(|p| p.name.len())
        .max()
        .unwrap_or(4)
        .max(4);
    writeln!(
        out,
        "{:<w$}  {:<16}  {:<10}  {:<12}  REMOTE",
        "NAME",
        "STACK",
        "STATUS",
        "COMMIT",
        w = w_name
    )?;
    for p in &report.projects {
        writeln!(
            out,
            "{:<w$}  {:<16}  {:<10}  {:<12}  {}",
            p.name,
            p.runtime.as_deref().unwrap_or("—"),
            if p.error.is_some() {
                "unreadable"
            } else {
                &p.status
            },
            p.last_commit
                .as_deref()
                .map_or("—", |c| c.get(..10).unwrap_or(c)),
            p.remote.as_deref().unwrap_or("—"),
            w = w_name
        )?;
    }

    writeln!(out)?;
    let mut parts = vec![format!("{} project(s)", report.projects.len())];
    if report.from_cache > 0 {
        // The "list may be stale and says so" half of the ADR-0004 contract.
        parts.push(format!("{} from cache", report.from_cache));
    }
    if report.archived_hidden > 0 {
        parts.push(format!("{} archived hidden", report.archived_hidden));
    }
    writeln!(out, "{}", parts.join(", "))?;

    if let Some(note) = &report.cache_note {
        writeln!(out, "note: {note}")?;
    }
    Ok(())
}

/// One project in full, formatted to be pasted at an agent.
pub fn write_show_human(
    out: &mut impl Write,
    view: &vibe_core::ProjectView,
) -> std::io::Result<()> {
    let m = &view.manifest;
    writeln!(out, "# {}", m.project.name)?;
    writeln!(out, "path:        {}", view.path)?;
    writeln!(
        out,
        "status:      {}{}",
        m.project.status,
        if m.project.archived {
            " (archived)"
        } else {
            ""
        }
    )?;
    if let Some(d) = &m.project.description {
        writeln!(out, "description: {d}")?;
    }
    if let Some(c) = &m.project.created {
        writeln!(out, "created:     {c}")?;
    }
    writeln!(out)?;
    writeln!(
        out,
        "runtime:     {}",
        m.stack.runtime.as_deref().unwrap_or("—")
    )?;
    if !m.stack.frameworks.is_empty() {
        writeln!(out, "frameworks:  {}", m.stack.frameworks.join(", "))?;
    }
    if !m.stack.services.is_empty() {
        writeln!(out, "services:    {}", m.stack.services.join(", "))?;
    }
    writeln!(
        out,
        "remote:      {}",
        m.repo.remote.as_deref().unwrap_or("—")
    )?;
    if let Some(u) = &m.deploy.url {
        writeln!(out, "deploy:      {u}")?;
    }
    if !m.deploy.env_required.is_empty() {
        writeln!(out, "env:         {}", m.deploy.env_required.join(", "))?;
    }
    if !m.context.decisions.is_empty() {
        writeln!(out, "\ndecisions:")?;
        for d in &m.context.decisions {
            writeln!(out, "  - {d}")?;
        }
    }
    if !m.context.next.is_empty() {
        writeln!(out, "\nnext:")?;
        for n in &m.context.next {
            writeln!(out, "  - {n}")?;
        }
    }

    // Surfaced rather than hidden: the user can see what an upgrade unlocks.
    if !view.unknown_keys.is_empty() {
        writeln!(
            out,
            "\n{} key(s) this build does not understand:",
            view.unknown_keys.len()
        )?;
        for k in &view.unknown_keys {
            writeln!(out, "  {}", k.dotted_path)?;
        }
    }
    Ok(())
}

/// What `sync` declined to do.
///
/// The split matters: `kept` and `unreadable` describe the user's project and
/// they can act on both. `not_attempted` is a fact about this machine, and
/// presenting it as a property of the repository would send them looking for a
/// problem that is not there.
pub fn write_sync_notes(out: &mut impl Write, notes: &vibe_core::SyncNotes) -> std::io::Result<()> {
    if notes.is_empty() {
        return Ok(());
    }
    if !notes.kept.is_empty() {
        writeln!(
            out,
            "kept existing value(s) for {} - detection had nothing better to offer:",
            notes.kept.len()
        )?;
        for f in &notes.kept {
            writeln!(out, "  {f}")?;
        }
    }
    for u in &notes.unreadable {
        writeln!(out, "warning: could not read {u}")?;
    }
    if !notes.not_attempted.is_empty() {
        writeln!(
            out,
            "note: some detection could not run on this machine ({}) - \
             this says nothing about the project itself",
            notes.not_attempted.join("; ")
        )?;
    }
    Ok(())
}

/// The facts [`write_repo_message`] renders from.
///
/// A plain local struct rather than [`vibe_core::RepoReport`] directly, and the
/// reason is a design constraint working correctly rather than an inconvenience:
/// `RepoReport` is `#[non_exhaustive]` (ADR-0005 §3) because consumers *read*
/// reports, so a consumer crate cannot build one in a struct literal — including
/// in its own tests.
///
/// Splitting the decision out means the message's branches are testable without
/// weakening that, and without core growing a constructor whose only caller is a
/// test. [`RepoMessage::from_report`] is the one place the two shapes meet.
struct RepoMessage<'a> {
    initialised: bool,
    already_a_repository: bool,
    committed: bool,
    identity_missing: bool,
    branch: Option<&'a str>,
    project_name: &'a str,
    /// `None` means no remote was asked for — which is not the same as one
    /// having failed, and must not render the same.
    remote_requested: bool,
    remote_created: bool,
    remote_blocked: Option<vibe_core::RemoteBlocked>,
    remote_url: Option<&'a str>,
}

impl<'a> RepoMessage<'a> {
    fn from_report(report: &'a vibe_core::RepoReport, project_name: &'a str) -> Self {
        Self {
            initialised: report.initialised,
            already_a_repository: report.already_a_repository,
            committed: report.committed,
            identity_missing: report.commit_blocked
                == Some(vibe_core::repo::CommitBlocked::NoAuthorIdentity),
            branch: report.branch.as_deref(),
            project_name,
            remote_requested: report.remote_requested.is_some(),
            remote_created: report.remote_created,
            remote_blocked: report.remote_blocked,
            remote_url: report.remote_url.as_deref(),
        }
    }
}

/// What `vibe new --git` did, and precisely what is left to do.
///
/// **This message is the entire quality of the `gh`-absent path** (ADR-0008
/// §3). The tool creates no remote and pushes nothing there, so a vague report
/// makes it the worst path in the product and an exact one makes it fine.
pub fn write_repo_human(
    out: &mut impl Write,
    report: &vibe_core::RepoReport,
    project_name: &str,
) -> std::io::Result<()> {
    write_repo_message(out, &RepoMessage::from_report(report, project_name))
}

/// Two rules this obeys:
///
/// - *"`gh` is not installed"* is a fact about **this machine**, worded so it
///   cannot be read as "this project cannot have a remote" — the same
///   distinction `SyncNotes::not_attempted` draws.
/// - The branch is whatever `git` reported. `git init` honours the user's
///   `init.defaultBranch`, so printing `main` when we did not read one would be
///   a plausible-looking guess in the one output whose whole purpose is being
///   correct enough to paste.
fn write_repo_message(out: &mut impl Write, m: &RepoMessage<'_>) -> std::io::Result<()> {
    if m.already_a_repository {
        writeln!(out, "Already a git repository - left alone.")?;
    } else if m.initialised {
        match m.branch {
            Some(b) => writeln!(out, "Initialised a git repository on branch {b}.")?,
            // Not "on branch main". We did not read one, so we do not name one.
            None => writeln!(out, "Initialised a git repository.")?,
        }
    }
    if m.committed {
        writeln!(out, "Committed the scaffold.")?;
    }

    // A machine with no `user.email` is common on a fresh install, and it is
    // not ours to fix: choosing a name and address on the user's behalf would
    // stamp an invented person into their history. So it is reported, with the
    // commands that fix it, and the run still succeeds.
    if m.identity_missing {
        writeln!(
            out,
            "\nThe scaffold was staged but not committed: git has no author \
             identity configured on this machine, and vibe will not invent one."
        )?;
        writeln!(out, "To fix, then commit:")?;
        writeln!(out, "  git config --global user.name \"Your Name\"")?;
        writeln!(out, "  git config --global user.email \"you@example.com\"")?;
        writeln!(out, "  git commit -m \"Initial commit\"")?;
    }

    // Nothing was asked of `gh`, so nothing is said about it — including on a
    // machine where it is missing. A limitation nobody reached is not a
    // limitation worth reporting, and `--git` on its own is a request for a
    // local repository.
    if !m.remote_requested {
        return Ok(());
    }

    if m.remote_created {
        match m.remote_url {
            // Read back from `git remote get-url`, never parsed out of `gh`'s
            // prose: this is what `origin` actually points at.
            Some(url) => writeln!(out, "Created the remote repository and pushed to {url}.")?,
            None => writeln!(out, "Created the remote repository and pushed.")?,
        }
        return Ok(());
    }

    // The honest half. Every branch below names a fact about this machine or
    // this run, and none of them says anything about the project.
    let reason = match m.remote_blocked {
        Some(vibe_core::RemoteBlocked::GhMissing) | None => {
            "gh was not found on this machine, so vibe did not create a remote \
             repository and did not push."
                .to_owned()
        }
        Some(vibe_core::RemoteBlocked::NotAuthenticated) => {
            "gh is installed but not authenticated for the environment vibe runs \
             it in, so no remote repository was created and nothing was pushed. \
             vibe clears the environment it hands to subprocesses, so a GH_TOKEN \
             exported in your shell is deliberately not passed on; `gh auth \
             login` stores a credential gh finds on its own."
                .to_owned()
        }
        Some(vibe_core::RemoteBlocked::NothingToPush) => {
            "there is no commit to push, so vibe did not create a remote \
             repository - an empty repository on your account is not a better \
             outcome than none."
                .to_owned()
        }
        // `RemoteBlocked` is `#[non_exhaustive]`, so this arm is required and
        // is reachable only from a core newer than this binary.
        //
        // It says what it does not know rather than rendering the reason's key
        // as though it were a sentence. `not_authenticated` is an identifier,
        // and printing it in the position where a sentence belongs would be
        // this build asserting a specific meaning it does not have — the same
        // substitution ADR-0002 §5 refuses for unknown manifest keys, and the
        // one constraint 5 exists to prevent.
        Some(other) => format!(
            "vibe did not create a remote repository and did not push. This \
             build has no explanation for the reason it was given (`{}`) — \
             upgrade vibe to get one.",
            other.key()
        ),
    };
    writeln!(out, "\n{reason}")?;

    writeln!(out, "To finish:")?;
    if m.remote_blocked == Some(vibe_core::RemoteBlocked::NotAuthenticated) {
        writeln!(out, "  gh auth login")?;
    }
    writeln!(
        out,
        "  gh repo create <owner>/{} --source=. --push",
        m.project_name
    )?;
    writeln!(
        out,
        "  # or, if you create the repository on github.com first:"
    )?;
    writeln!(
        out,
        "  git remote add origin git@github.com:<owner>/{}.git",
        m.project_name
    )?;
    match m.branch {
        Some(b) => writeln!(out, "  git push -u origin {b}")?,
        None => writeln!(out, "  git push -u origin <branch>")?,
    }
    Ok(())
}

#[cfg(test)]
mod degradation_tests {
    use vibe_core::{CoreError, RenderState};

    /// Every refusal state gets its own hint, and no two read the same.
    ///
    /// ADR-0007 §4's rule is that the refusals must not read alike, because one
    /// is a `--force` away and the other is not reachable by any flag. That
    /// applied to two states; `Unverifiable` was reaching the wildcard and
    /// being told the same thing as a state nobody had looked at.
    #[test]
    fn each_render_refusal_says_something_different() {
        let hint = |state| {
            super::hint_for(&CoreError::RenderRefused {
                path: std::path::PathBuf::from("CLAUDE.md"),
                state,
            })
            .expect("every refusal has a hint")
        };

        let foreign = hint(RenderState::Foreign);
        let modified = hint(RenderState::Modified);
        let unverifiable = hint(RenderState::Unverifiable);

        assert_ne!(foreign, modified);
        assert_ne!(modified, unverifiable);
        assert_ne!(
            foreign, unverifiable,
            "Unverifiable borrowed another state's hint"
        );

        // The one that must never promise a flag will help.
        assert!(foreign.contains("not even with --force"), "{foreign}");
        // And the one that must say what it could not evaluate.
        assert!(
            unverifiable.contains("cannot") && unverifiable.contains("marker"),
            "{unverifiable}"
        );
    }

    /// The unrankable label claims no rank — the property the whole severity
    /// change exists for, tested directly rather than through a variant that
    /// cannot be constructed.
    #[test]
    fn an_unplaceable_severity_borrows_no_rank() {
        let named = super::unranked_severity_label(Some("critical"));
        assert!(named.contains("critical"), "{named}");
        assert!(named.contains('`'), "the name must read as data: {named}");
        assert!(named.contains("unrecognised"), "{named}");
        // The substitution this replaced: a rank it does not have.
        for rank in ["warning", "note"] {
            assert!(!named.contains(rank), "borrowed the rank `{rank}`: {named}");
        }

        // And with no name available, it says less rather than guessing.
        let anonymous = super::unranked_severity_label(None);
        assert!(anonymous.contains("unrecognised"), "{anonymous}");
        for rank in ["warning", "note"] {
            assert!(
                !anonymous.contains(rank),
                "borrowed the rank `{rank}`: {anonymous}"
            );
        }
    }

    /// The `Warn` label is unchanged by the severity fix.
    ///
    /// **Two things this cannot reach, stated rather than left as holes.**
    /// `Severity` is `#[non_exhaustive]` with exactly two variants, so no test
    /// in this workspace can construct a third: **the unrecognised-severity arm
    /// — the one this change exists for — is unreachable until a newer core
    /// exists.** And `Diagnostic` is `#[non_exhaustive]` too, so it cannot be
    /// built by struct literal here at all; `Diagnostic::warn` is the only
    /// constructor core exposes, which means `Severity::Note` has no path into
    /// this renderer from a test either.
    ///
    /// What is left is the half that matters for a regression: the fix did not
    /// silently relabel the severity that every diagnostic in the product
    /// actually carries.
    #[test]
    fn the_warn_label_is_unchanged() {
        let line = super::diagnostic_line(&vibe_core::Diagnostic::warn("VIBE_W_TEST"));
        assert!(line.starts_with("warning"), "{line}");
        assert!(!line.contains("unrecognised"), "{line}");
    }
}

#[cfg(test)]
mod repo_message_tests {
    use super::RepoMessage;

    /// Unit tests rather than CLI tests, on purpose. The `gh`-absent branch can
    /// only be exercised end-to-end on a machine without `gh`, and every CI
    /// runner has one — so a CLI test of it skips exactly where it most needs to
    /// run, which is a skip masquerading as a pass (ADR-0002 §7). Driving the
    /// renderer directly makes both branches deterministic everywhere.
    fn render(m: &RepoMessage<'_>) -> String {
        let mut buf = Vec::new();
        super::write_repo_message(&mut buf, m).expect("writes");
        String::from_utf8(buf).expect("utf8")
    }

    /// A local-only run: `--git` with no visibility flag.
    fn base() -> RepoMessage<'static> {
        RepoMessage {
            initialised: true,
            already_a_repository: false,
            committed: true,
            identity_missing: false,
            branch: Some("trunk"),
            project_name: "demo",
            remote_requested: false,
            remote_created: false,
            remote_blocked: None,
            remote_url: None,
        }
    }

    /// The same run with a remote asked for and not created.
    fn wanted_remote(why: vibe_core::RemoteBlocked) -> RepoMessage<'static> {
        RepoMessage {
            remote_requested: true,
            remote_blocked: Some(why),
            ..base()
        }
    }

    /// **The frontend half of the chain in ADR-0001 §4.**
    ///
    /// New variant → core's `key()` match stops compiling → author adds a key
    /// and extends `ALL` → *this* goes red until someone writes the sentence.
    /// Without it, a new reason reaches users as the fallback's "this build has
    /// no explanation", which is honest but is not the sentence the reason
    /// deserves.
    ///
    /// It is driven by `RemoteBlocked::ALL`, and its strength rests on that
    /// list being complete — which is not mechanically guaranteed. Core's
    /// `all_lists_every_variant_and_every_variant_has_a_key` puts the
    /// compiler's objection beside the list; this test cannot see past it.
    #[test]
    fn every_blocked_reason_has_a_sentence_of_its_own() {
        for reason in vibe_core::RemoteBlocked::ALL {
            let text = render(&wanted_remote(reason));
            assert!(
                !text.contains("has no explanation for the reason"),
                "`{}` fell through to the unknown-reason fallback: it needs a \
                 sentence in write_repo_message (ADR-0001 §4)\n{text}",
                reason.key()
            );
            // And the sentence must be its own, not another reason's.
            assert!(
                !text.contains(reason.key()),
                "`{}` was rendered by printing its key, which is an identifier \
                 standing where a sentence belongs\n{text}",
                reason.key()
            );
        }
    }

    /// The paired half: a reason this build genuinely does not know must reach
    /// the fallback, or the test above is asserting against a renderer that
    /// cannot fail. `RemoteBlocked` is `#[non_exhaustive]`, so no such variant
    /// can be constructed here — the fallback is instead exercised through the
    /// one input that reaches it today.
    #[test]
    fn a_reason_with_no_sentence_says_so_rather_than_printing_its_key() {
        let text = render(&RepoMessage {
            remote_requested: true,
            remote_blocked: None,
            remote_created: false,
            ..base()
        });
        // `None` takes the gh-missing branch, which is the documented
        // degradation for "blocked, reason unstated".
        assert!(text.contains("gh was not found"), "{text}");
    }

    #[test]
    fn a_local_only_run_says_nothing_about_remotes() {
        let text = render(&base());
        assert!(text.contains("branch trunk"), "{text}");
        assert!(text.contains("Committed the scaffold"), "{text}");
        // The paired half of every assertion below: none of the finish-the-job
        // advice appears when there is nothing to finish.
        assert!(!text.contains("gh was not found"), "{text}");
        assert!(!text.contains("will not invent one"), "{text}");
        // And nothing about a remote nobody asked for. `--git` is a request for
        // a local repository; volunteering what `gh` would have done is nagging
        // about a problem the user does not have.
        assert!(!text.to_lowercase().contains("remote"), "{text}");
        assert!(!text.contains("gh repo create"), "{text}");
    }

    #[test]
    fn without_gh_it_names_the_machine_not_the_project() {
        let text = render(&wanted_remote(vibe_core::RemoteBlocked::GhMissing));
        assert!(text.contains("gh was not found on this machine"), "{text}");
        assert!(text.contains("gh repo create <owner>/demo"), "{text}");
        assert!(text.contains("git push -u origin trunk"), "{text}");

        // The distinction the whole path rests on: a fact about the machine's
        // tooling, never a claim that the project cannot have a remote — the
        // same substitution `NotAttempted` versus `NoEvidence` prevents.
        let lower = text.to_lowercase();
        for claim in ["cannot have a remote", "has no remote", "project has no"] {
            assert!(!lower.contains(claim), "{claim}: {text}");
        }
    }

    /// `gh` present but unusable must not read as `gh` missing. They need
    /// different commands from the user, and one of them is a consequence of
    /// vibe's own containment rather than of the machine's tooling.
    #[test]
    fn an_unauthenticated_gh_is_not_reported_as_a_missing_one() {
        let text = render(&wanted_remote(vibe_core::RemoteBlocked::NotAuthenticated));
        assert!(!text.contains("gh was not found"), "{text}");
        assert!(text.contains("not authenticated"), "{text}");
        // The advice that actually fixes it, first.
        assert!(text.contains("gh auth login"), "{text}");
        // And the reason it can happen on a machine where `gh auth status` is
        // green: the environment vibe hands to subprocesses is constructed.
        assert!(text.contains("GH_TOKEN"), "{text}");
    }

    #[test]
    fn nothing_to_push_says_so_rather_than_blaming_gh() {
        let text = render(&RepoMessage {
            committed: false,
            ..wanted_remote(vibe_core::RemoteBlocked::NothingToPush)
        });
        assert!(text.contains("no commit to push"), "{text}");
        assert!(!text.contains("gh was not found"), "{text}");
        assert!(!text.contains("not authenticated"), "{text}");
    }

    /// The success path names what `origin` actually points at, read back from
    /// `git` rather than parsed out of `gh`'s output.
    #[test]
    fn a_created_remote_reports_the_url_and_asks_for_nothing() {
        let text = render(&RepoMessage {
            remote_requested: true,
            remote_created: true,
            remote_url: Some("https://github.com/you/demo.git"),
            ..base()
        });
        assert!(text.contains("https://github.com/you/demo.git"), "{text}");
        assert!(!text.contains("To finish"), "{text}");
        assert!(!text.contains("gh repo create"), "{text}");

        // A create whose URL could not be read says less rather than guessing
        // one, the same rule the branch follows.
        let unknown = render(&RepoMessage {
            remote_requested: true,
            remote_created: true,
            remote_url: None,
            ..base()
        });
        assert!(
            unknown.contains("Created the remote repository"),
            "{unknown}"
        );
        assert!(!unknown.contains("github.com"), "{unknown}");
    }

    #[test]
    fn an_unknown_branch_is_never_rendered_as_main() {
        let text = render(&RepoMessage {
            branch: None,
            ..wanted_remote(vibe_core::RemoteBlocked::GhMissing)
        });
        assert!(
            !text.contains("origin main"),
            "a branch we did not read was printed as `main`, which is a \
             plausible-looking guess in the one output whose purpose is being \
             correct enough to paste:\n{text}"
        );
        assert!(text.contains("git push -u origin <branch>"), "{text}");
        assert!(!text.contains("on branch"), "{text}");
    }

    #[test]
    fn a_missing_identity_is_advice_and_a_clean_run_is_silence() {
        let missing = render(&RepoMessage {
            committed: false,
            identity_missing: true,
            ..base()
        });
        assert!(missing.contains("will not invent one"), "{missing}");
        assert!(missing.contains("user.email"), "{missing}");

        // Paired: nothing-to-commit is a no-op and must produce no advice at
        // all, or a second `vibe new --git` nags about a problem nobody has.
        let quiet = render(&RepoMessage {
            committed: false,
            identity_missing: false,
            ..base()
        });
        assert!(!quiet.contains("will not invent one"), "{quiet}");
        assert!(!quiet.contains("user.email"), "{quiet}");
    }

    #[test]
    fn an_existing_repository_is_reported_as_left_alone() {
        let text = render(&RepoMessage {
            initialised: false,
            already_a_repository: true,
            ..base()
        });
        assert!(text.contains("left alone"), "{text}");
        assert!(!text.contains("Initialised"), "{text}");
    }
}
