//! The P3 command bodies.
//!
//! Each follows the same shape: build a plan or a report in `vibe-core`, render
//! it here, and — for writes — stop before `apply` when `--dry-run` is set.

use std::io::Write;

use vibe_core::{Config, CoreError, Query, Registry, ScanRequest};

use crate::cli::{ArchiveArgs, ListArgs, RenderArgs, ShowArgs, SyncArgs};
use crate::exit::Exit;
use crate::{output, reporter};

pub fn list(args: &ListArgs) -> Result<Exit, CoreError> {
    let registry = Registry::open(Config::discover());
    let req = ScanRequest::new(&args.path).with_max_depth(args.depth);
    let query = Query::default()
        .with_archived(args.all)
        .with_status(args.status.clone());

    let rep = reporter::TermReporter::new(args.format.json);
    let report = registry.list(&req, &query, &rep);

    let mut stdout = std::io::stdout();
    if args.format.json {
        let json = serde_json::to_string_pretty(&report)
            .expect("ListReport is a plain data structure and always serialises");
        let _ = writeln!(stdout, "{json}");
    } else {
        let _ = output::write_list_human(&mut stdout, &report);
    }

    // A project whose manifest could not be read is a partial result, not a
    // failure: the registry was read and the rest is fine.
    rep.flush();
    if report.unreadable() > 0 {
        return Ok(Exit::Partial);
    }
    Ok(Exit::Success)
}

pub fn show(args: &ShowArgs) -> Result<Exit, CoreError> {
    let registry = Registry::open(Config::discover());
    // Never cached. `show` is the thing you paste at an agent.
    let view = registry.show(&args.path)?;

    let mut stdout = std::io::stdout();
    if args.format.json {
        let json = serde_json::to_string_pretty(&view)
            .expect("ProjectView is a plain data structure and always serialises");
        let _ = writeln!(stdout, "{json}");
    } else {
        let _ = output::write_show_human(&mut stdout, &view);
    }
    Ok(Exit::Success)
}

pub fn sync(args: &SyncArgs) -> Result<Exit, CoreError> {
    let registry = Registry::open(Config::discover());
    let rep = reporter::TermReporter::new(args.format.json);
    let (plan, notes) = registry.plan_sync(&args.path, &rep)?;

    let mut stdout = std::io::stdout();
    let mut stderr = std::io::stderr();

    if args.format.json {
        let payload = serde_json::json!({ "plan": plan, "notes": notes });
        let _ = writeln!(
            stdout,
            "{}",
            serde_json::to_string_pretty(&payload).expect("plain data serialises")
        );
    } else if plan.is_empty() {
        let _ = writeln!(stdout, "Already up to date.");
    } else {
        let _ = output::write_plan_human(&mut stdout, &plan);
    }

    // Notes go to stderr even on a no-op plan: "nothing changed" and "nothing
    // changed because two manifests disagree" are different situations, and
    // only the second is worth the user's attention.
    if !args.format.json {
        let _ = output::write_sync_notes(&mut stderr, &notes);
    }
    // One line per distinct diagnostic, once per run. Core emits one per
    // manifest because it does not know what a run is.
    rep.flush();

    if args.write.dry_run {
        if !args.format.json {
            let _ = writeln!(stderr, "\ndry run - nothing was written");
        }
        return Ok(Exit::Success);
    }
    if plan.is_empty() {
        return Ok(Exit::Success);
    }

    let report = registry.apply(&plan, &rep)?;
    if !args.format.json {
        let _ = output::write_apply_human(&mut stdout, &report);
    }
    Ok(Exit::Success)
}

pub fn archive(args: &ArchiveArgs, archived: bool) -> Result<Exit, CoreError> {
    let registry = Registry::open(Config::discover());
    let plan = registry.plan_archive(&args.path, archived)?;

    let mut stdout = std::io::stdout();
    let mut stderr = std::io::stderr();
    let verb = if archived { "archived" } else { "unarchived" };

    // Already in the target state. A no-op, not an error — running `archive`
    // twice is a reasonable thing for a person to do.
    if plan.is_empty() {
        let _ = writeln!(stdout, "Already {verb} - nothing to do.");
        return Ok(Exit::Success);
    }

    if args.format.json {
        let _ = writeln!(
            stdout,
            "{}",
            serde_json::to_string_pretty(&plan).expect("plain data serialises")
        );
    } else {
        let _ = output::write_plan_human(&mut stdout, &plan);
    }

    if args.write.dry_run {
        if !args.format.json {
            let _ = writeln!(stderr, "\ndry run - nothing was written");
        }
        return Ok(Exit::Success);
    }

    let rep = reporter::TermReporter::new(args.format.json);
    let report = registry.apply(&plan, &rep)?;
    if !args.format.json {
        let _ = writeln!(stdout, "Project {verb}.");
        let _ = output::write_apply_human(&mut stdout, &report);
    }
    Ok(Exit::Success)
}

/// Generate one file from the manifest.
///
/// The refusal happens in core, at plan time, so a `--dry-run` never shows an
/// op that `apply` would decline. Everything this adds is the sentence
/// explaining which of the two refusals happened, because "you edited this,
/// pass --force" and "this is not ours and --force will not help" must not read
/// the same.
pub fn render(args: &RenderArgs) -> Result<Exit, CoreError> {
    let registry = Registry::open(Config::discover());
    let plan = registry.plan_render(&args.path, args.target, args.force)?;

    let mut stdout = std::io::stdout();
    let mut stderr = std::io::stderr();

    if args.format.json {
        let json = serde_json::to_string_pretty(&plan).expect("plain data serialises");
        let _ = writeln!(stdout, "{json}");
    } else if plan.is_empty() {
        // Byte-identical to what is already there. Not an error, and not a
        // write - saying so beats showing a diff with nothing in it.
        let _ = writeln!(stdout, "{} is already up to date.", args.target);
        return Ok(Exit::Success);
    } else {
        let _ = output::write_plan_human(&mut stdout, &plan);
    }

    if args.write.dry_run {
        if !args.format.json {
            let _ = writeln!(
                stderr,
                "
dry run - nothing was written"
            );
        }
        return Ok(Exit::Success);
    }
    if plan.is_empty() {
        return Ok(Exit::Success);
    }

    let rep = reporter::TermReporter::new(args.format.json);
    let report = registry.apply(&plan, &rep)?;
    if !args.format.json {
        let _ = output::write_apply_human(&mut stdout, &report);
    }
    Ok(Exit::Success)
}
