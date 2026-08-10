//! The six `vibe agents` command bodies.
//!
//! Every one follows the P3 shape: build a plan or a report in `vibe-core`,
//! render it here, and — for writes — stop before `apply` when `--dry-run` is
//! set. Nothing here decides policy; the ADR-0006 §5 state table does, in core.
//!
//! Two rules this module is responsible for that the others are not:
//!
//! - **The store-age line.** Whenever anything is `NotInStore` and the store is
//!   stale, the age is printed *regardless of the usual quiet rules*
//!   (ADR-0006 §6). "This agent does not exist" and "this machine has not
//!   fetched for twelve days" are different claims, and printing the first
//!   without the second turns a fact about the machine into a claim about the
//!   project.
//! - **A refusal is not a failure.** `sync` installs what it can and reports
//!   what it cannot; exit `2`, not `1`.

use std::io::Write;

use vibe_core::agents::{AgentPlan, AgentState, Staleness, StoreConfig};
use vibe_core::{Config, CoreError, Registry};

use crate::cli::{
    AgentsAddArgs, AgentsListArgs, AgentsRemoveArgs, AgentsStatusArgs, AgentsSyncArgs,
    AgentsUpdateArgs, StoreFlags,
};
use crate::exit::Exit;
use crate::{output, reporter};

fn store_config(flags: &StoreFlags) -> StoreConfig {
    let mut cfg = StoreConfig::default();
    if let Some(url) = &flags.store_url {
        cfg = cfg.with_upstream(url.clone());
    }
    if let Some(path) = &flags.store_path {
        cfg = cfg.with_path(path.clone());
    }
    if let Some(days) = flags.stale_after {
        cfg = cfg.with_stale_after_days(days);
    }
    cfg
}

pub fn update(args: &AgentsUpdateArgs) -> Result<Exit, CoreError> {
    let registry = Registry::open(Config::discover());
    let store = store_config(&args.store);
    let report = registry.agents_update_store(&store)?;

    let mut stdout = std::io::stdout();
    if args.format.json {
        let payload = serde_json::json!({
            "path": report.path,
            "cloned": report.cloned,
            "from_rev": report.from_rev,
            "to_rev": report.to_rev,
            "agents": report.agents,
            "changed": report.changed(),
        });
        let _ = writeln!(stdout, "{}", pretty(&payload));
        return Ok(Exit::Success);
    }

    if report.cloned {
        let _ = writeln!(
            stdout,
            "Cloned the agent store into {} ({} agent(s)).",
            report.path, report.agents
        );
    } else if report.changed() {
        let _ = writeln!(
            stdout,
            "Updated {} -> {} ({} agent(s)).",
            short(report.from_rev.as_deref()),
            short(report.to_rev.as_deref()),
            report.agents
        );
        // Installed agents are NOT rewritten by this command. Saying so is the
        // difference between a user knowing to run `sync --update` and a user
        // wondering why nothing changed in their project.
        let _ = writeln!(
            stdout,
            "Installed agents are unchanged - run `vibe agents sync --update` to apply."
        );
    } else {
        let _ = writeln!(
            stdout,
            "Already up to date ({}).",
            short(report.to_rev.as_deref())
        );
    }
    Ok(Exit::Success)
}

pub fn list(args: &AgentsListArgs) -> Result<Exit, CoreError> {
    let registry = Registry::open(Config::discover());
    let store = store_config(&args.store);
    let today = registry.config().today_utc();
    let catalogue = registry.agents_list(Some(&args.path), &store, &today)?;
    let listings = &catalogue.listings;

    let mut stdout = std::io::stdout();
    let mut stderr = std::io::stderr();
    if args.format.json {
        // `store_age` alongside the agents, as `status`, `add`, `remove` and
        // `sync` already do. A bare array was the odd one out, and it was the
        // shape that made the age impossible to report.
        let payload = serde_json::json!({
            "agents": listings,
            "store_age": staleness_json(catalogue.staleness),
        });
        let _ = writeln!(stdout, "{}", pretty(&payload));
        return Ok(Exit::Success);
    }

    if listings.is_empty() {
        // Never "there are no agents": we do not know that. We know this store
        // is empty, which is a fact about this machine.
        let _ = writeln!(
            stdout,
            "The agent store is empty or has never been fetched.\n\
             Run `vibe agents update` first."
        );
        return Ok(Exit::Success);
    }

    let width = listings.iter().map(|l| l.name.len()).max().unwrap_or(4);
    for l in listings {
        let mark = if l.declared { "*" } else { " " };
        let desc = l.description.as_deref().unwrap_or("—");
        let _ = writeln!(stdout, "{mark} {:<width$}  {desc}", l.name, width = width);
    }
    let declared = listings.iter().filter(|l| l.declared).count();
    let _ = writeln!(
        stdout,
        "\n{} agent(s); {declared} declared by this project (*)",
        listings.len()
    );

    // ADR-0006 §7. `list` is the command where this matters most: every name
    // above is a fact about this machine's copy of the store, and a reader with
    // no age has no way to tell a complete list from a twelve-day-old one.
    // Skipped in the empty-store branch above, which already says it in prose.
    if catalogue.staleness.worth_reporting() {
        let _ = write_store_age(&mut stderr, catalogue.staleness);
    }
    Ok(Exit::Success)
}

pub fn status(args: &AgentsStatusArgs) -> Result<Exit, CoreError> {
    let registry = Registry::open(Config::discover());
    let store = store_config(&args.store);
    let today = registry.config().today_utc();
    let status = registry.agents_status(&args.path, &store, &today)?;

    let mut stdout = std::io::stdout();
    let mut stderr = std::io::stderr();

    if args.format.json {
        let payload = serde_json::json!({
            "agents": status.report.agents,
            "ownership_known": status.report.ownership_known,
            "lock_note": status.report.lock_note,
            "store_rev": status.store_rev,
            "store_age": staleness_json(status.staleness),
        });
        let _ = writeln!(stdout, "{}", pretty(&payload));
    } else if status.report.agents.is_empty() {
        let _ = writeln!(stdout, "This project declares no agents.");
    } else {
        let width = status
            .report
            .agents
            .iter()
            .map(|a| a.name.len())
            .max()
            .unwrap_or(4);
        for a in &status.report.agents {
            let mut line = format!("{:<width$}  {}", a.name, a.state.as_str(), width = width);
            if a.outdated {
                line.push_str("  (store has a newer revision)");
            }
            if let Some(to) = &a.renamed_to {
                line.push_str(&format!("  (upstream renamed it to {to})"));
            }
            let _ = writeln!(stdout, "{line}");
        }
    }

    // The lockfile note is the load-bearing one: it says *why* nothing may be
    // written, and without it "refused" reads as a bug.
    if let Some(note) = &status.report.lock_note {
        let _ = writeln!(stderr, "warning: {note}");
    }
    if status.report.any(AgentState::NotInStore) {
        let _ = write_store_age(&mut stderr, status.staleness);
    }

    if !status.report.ownership_known || status.report.any(AgentState::NotInStore) {
        return Ok(Exit::Partial);
    }
    Ok(Exit::Success)
}

pub fn add(args: &AgentsAddArgs) -> Result<Exit, CoreError> {
    let registry = Registry::open(Config::discover());
    let store = store_config(&args.store);
    let today = registry.config().today_utc();
    let plan =
        registry.plan_agents_add(&args.path, &args.names, args.force.force, &store, &today)?;
    finish(
        &registry,
        &plan,
        args.write.dry_run,
        args.format.json,
        "add",
    )
}

pub fn remove(args: &AgentsRemoveArgs) -> Result<Exit, CoreError> {
    let registry = Registry::open(Config::discover());
    let store = store_config(&args.store);
    let today = registry.config().today_utc();
    let plan = registry.plan_agents_remove(&args.path, &args.names, &store, &today)?;
    finish(
        &registry,
        &plan,
        args.write.dry_run,
        args.format.json,
        "remove",
    )
}

pub fn sync(args: &AgentsSyncArgs) -> Result<Exit, CoreError> {
    let registry = Registry::open(Config::discover());
    let store = store_config(&args.store);
    let today = registry.config().today_utc();
    let plan =
        registry.plan_agents_sync(&args.path, args.update, args.force.force, &store, &today)?;
    finish(
        &registry,
        &plan,
        args.write.dry_run,
        args.format.json,
        "sync",
    )
}

/// Render a plan, apply it unless this is a dry run, and pick the exit code.
///
/// Shared by the three write commands so the dry-run gate cannot be present on
/// two of them and missing from the third.
fn finish(
    registry: &Registry,
    agent_plan: &AgentPlan,
    dry_run: bool,
    json: bool,
    verb: &str,
) -> Result<Exit, CoreError> {
    let mut stdout = std::io::stdout();
    let mut stderr = std::io::stderr();

    if json {
        let payload = serde_json::json!({
            "plan": agent_plan.plan,
            "refused": agent_plan.refused,
            "agents": agent_plan.report.agents,
            "store_age": staleness_json(agent_plan.staleness),
        });
        let _ = writeln!(stdout, "{}", pretty(&payload));
    } else if agent_plan.is_empty() {
        let _ = writeln!(stdout, "Nothing to {verb}.");
    } else {
        let _ = output::write_plan_human(&mut stdout, &agent_plan.plan);
    }

    // Refusals go to stderr on every path, `--json` included: they are the
    // difference between "nothing to do" and "there was something to do and we
    // declined", and a user who cannot tell those apart cannot act.
    for r in &agent_plan.refused {
        let _ = writeln!(
            stderr,
            "refused {} ({}): {}",
            r.name,
            r.state.as_str(),
            r.why
        );
    }
    // ADR-0006 §7: any command that read the store says so when it is stale.
    // This is broader than what was here before, which fired only for §6's
    // `NotInStore` case — so `add`, `remove` and `sync` used to install from a
    // twelve-day-old store without mentioning it, as long as every name
    // resolved. §6 is the sharper case *inside* this one
    // (`store_age_must_not_be_suppressed`), and the two agree exactly while
    // there is no quiet flag for §6 to override.
    if agent_plan.staleness.worth_reporting() {
        let _ = write_store_age(&mut stderr, agent_plan.staleness);
    }

    if dry_run {
        if !json {
            let _ = writeln!(stderr, "\ndry run - nothing was written");
        }
        return Ok(exit_for(agent_plan));
    }
    if agent_plan.is_empty() {
        return Ok(exit_for(agent_plan));
    }

    let rep = reporter::TermReporter::new(json);
    let report = registry.apply(&agent_plan.plan, &rep)?;
    if !json {
        let _ = output::write_apply_human(&mut stdout, &report);
    }
    rep.flush();
    Ok(exit_for(agent_plan))
}

/// A refusal is a *partial* result, never a failure. The other agents installed
/// and the registry is intact; collapsing that into `1` would make a project
/// with one hand-edited agent indistinguishable from a broken command.
fn exit_for(plan: &AgentPlan) -> Exit {
    if plan.partial() {
        Exit::Partial
    } else {
        Exit::Success
    }
}

/// The store-age line.
///
/// The wording is the point. A store that has *never* been fetched is said
/// explicitly rather than reported as an age, and an age we could not read is
/// said too — reporting either as "0 days" would claim the store is current
/// when the truth is that we do not know.
fn write_store_age(out: &mut impl Write, staleness: Staleness) -> std::io::Result<()> {
    match staleness {
        Staleness::NeverUpdated => writeln!(
            out,
            "The agent store has never been fetched; run `vibe agents update` first."
        ),
        Staleness::Unknown => writeln!(
            out,
            "The agent store's age could not be read; run `vibe agents update` to be sure."
        ),
        Staleness::Days { days, stale: true } => writeln!(
            out,
            "The store was last updated {days} day(s) ago; run `vibe agents update` first."
        ),
        Staleness::Days { stale: false, .. } => Ok(()),
        // `Staleness` is #[non_exhaustive]. Saying nothing about a case this
        // build cannot describe would be worse than saying we cannot describe
        // it, because silence here reads as "the store is fresh".
        _ => writeln!(
            out,
            "The agent store's age is reported in a form this build does not understand."
        ),
    }
}

fn staleness_json(staleness: Staleness) -> serde_json::Value {
    match staleness {
        Staleness::NeverUpdated => serde_json::json!({ "state": "never_updated" }),
        Staleness::Unknown => serde_json::json!({ "state": "unknown" }),
        Staleness::Days { days, stale } => {
            serde_json::json!({ "state": "known", "days": days, "stale": stale })
        }
        _ => serde_json::json!({ "state": "unrecognised" }),
    }
}

fn pretty(v: &serde_json::Value) -> String {
    serde_json::to_string_pretty(v).expect("plain data serialises")
}

/// A revision, shortened for display only. Nothing is ever addressed by this.
fn short(rev: Option<&str>) -> String {
    rev.map_or_else(|| "—".to_owned(), |r| r.get(..10).unwrap_or(r).to_owned())
}
