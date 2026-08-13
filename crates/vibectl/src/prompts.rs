//! Rendering for `vibe prompt`. Phase 3 of ADR-0010, and the display §7 names.
//!
//! Core produces the data and no prose; every sentence below is this crate's,
//! per ADR-0001 §4. The Tauri frontend will write its own for the same reasons,
//! in its own register — a state marker and a tooltip where this has a column
//! and an indented line — and that divergence is the system working rather than
//! duplication.
//!
//! # The four things this file exists to get right
//!
//! 1. **An unknown state reaches the reader with its cause.** Six causes with
//!    three different remedies collapse into one state word, so the word alone
//!    is honest and unactionable (§5). The cause is rendered from core's
//!    description plus a remedy this crate owns.
//! 2. **A name shown is never a claim that the name resolves here.** Two things
//!    can make it false: a user-level file of the same name (which core
//!    reports), and a plugin (which core cannot see at all). The second is
//!    unresolvable by the base layer *permanently*, so no listing can ever
//!    promise resolution and this one does not.
//! 3. **`NotAttempted` must read as neither fine nor error.**
//! 4. **An empty list is not automatically a fact.** `is_complete()` is what
//!    separates *"this project defines no prompts"* from *"vibe could not find
//!    out"*, and it is the most direct case of constraint 5 in the tool.

use std::io::Write;
use std::path::Path;

use vibe_core::prompts::{Exposure, Prompt, PromptListing, PromptRoot, RootOutcome};
use vibe_core::{IgnoreState, UnknownCause};

/// The exposure column's word, for each of the four display states.
///
/// **`not asked` rather than `different root`, and rather than `not
/// attempted`.** Two decisions, recorded because §5's label genealogy is four
/// passes of a name reaching one step past the instrument, always towards
/// reassurance:
///
/// - *`different root`* asserts that another repository exists and that this is
///   not it. Vibe compared two path strings and routed; it never invoked git.
///   The sentence that goes false when `~/.claude` is symlinked into the project
///   tree is *"your project's `git add -A` cannot pick this file up"* — which is
///   the reassuring direction. `not asked` names vibe's own action, which vibe
///   observes completely, so there is no step to be shy by.
/// - *`not attempted`* is the word this codebase already uses for the plugin
///   namespace, and reusing it here would be the confusion rather than the
///   consistency. The plugin fact is a footer about the whole listing and is
///   *unanswerable by the base layer permanently*; this is a per-row value whose
///   question has an honest answer that belongs to another repository. A reader
///   who has discharged the footer — *plugins, understood* — would carry that
///   discharge into a row that carries exposure. Reassurance flowing from the
///   harmless instance to the load-bearing one is the direction polarity B
///   cannot take, so the two get different words.
///
/// Note there is no collision in core to inherit: [`Exposure`] expresses the
/// unasked case as `state: None`, not as a `NotAttempted` variant. The display
/// could create the collision and declines to.
fn exposure_label(exposure: &Exposure) -> &'static str {
    let Some(state) = exposure.state() else {
        return "not asked";
    };
    // `IgnoreState` is `#[non_exhaustive]`, so this crate cannot match it
    // exhaustively and a wildcard is unavoidable. What the wildcard must not do
    // is borrow one of the three words: that is the unranked-`Severity` defect
    // (ADR-0009 §4), which was a specific and false claim rather than a hedge.
    //
    // **The wildcard arm is not covered, and it cannot be from here.** Reaching
    // it needs a fourth `IgnoreState` variant, which no build that exists has —
    // the same dependency's-contract shape as `unranked_severity_label`. That
    // one was factored out so its *text* stayed testable; this one has no text
    // to factor, only a literal, so factoring would buy a test asserting a
    // constant equals itself. Recorded as a declared gap rather than closed with
    // machinery that proves nothing.
    match state.key() {
        "ignored" => "ignored",
        "not_ignored" => "not ignored",
        "unknown" => "unknown",
        _ => "unrecognised state",
    }
}

/// What a reader can do about one unknown cause.
///
/// **Three outcomes, not two.** An absent remedy has to be an *answer* rather
/// than a gap, or this is `hint_for`'s `_ => None` again — the wildcard that was
/// found serving `Unverifiable`'s sentence, and its `--force` advice, to every
/// other `RenderState` (ADR-0007 §4).
#[derive(Debug, PartialEq, Eq)]
enum Remedy {
    /// Something the reader can do about it.
    Action(&'static str),
    /// Nothing anyone can do, and saying so is the answer.
    ///
    /// `NoExitCode` and `Unrecognised` are genuinely like this: the first is
    /// *we do not understand this*, the second is a git this build does not
    /// recognise. Offering a remedy for either would be promising a fix for
    /// something not identified.
    NoneAvailable,
    /// A cause this build has no entry for. Not a remedy — the absence of one.
    NoEntry,
}

/// The remedy catalogue, keyed on the wire code.
///
/// **Keyed on `code` because that is the one rule a frontend learns** — the same
/// rule `CoreError` payloads follow, now that each cause carries its own code
/// rather than six sharing `VIBE_S_IGNORE_UNKNOWN`.
///
/// These are **remedies, not descriptions**. Core owns the description and it
/// arrives in `to_wire().message`; writing a second description here would be
/// the duplication ADR-0001 §4 sends toward the frontend, pointed the wrong way.
/// ADR-0010 §5 separates these causes *by remedy* in the first place, which is
/// what makes a remedy catalogue the right thing to key on the split.
fn remedy_for(code: &str) -> Remedy {
    match code {
        "VIBE_S_IGNORE_GIT_NOT_RUN" => Remedy::Action(
            "install git — until it is on PATH, vibe cannot tell you which of these \
             prompts your repository would publish",
        ),
        "VIBE_S_IGNORE_NOT_A_REPOSITORY" => Remedy::Action(
            "run `git init` here, or move the project into a repository. Nothing is \
             published from a directory git does not track, but nothing is checked \
             either",
        ),
        "VIBE_S_IGNORE_PATH_OUTSIDE_REPOSITORY" => Remedy::Action(
            "the project directory and the file disagree about which repository they \
             are in — check that --path names the repository the prompts live in",
        ),
        "VIBE_S_IGNORE_TIMED_OUT" => Remedy::Action(
            "git did not answer in time. A repository mid-operation, or a very large \
             one, will do this; re-run once it settles",
        ),
        // The two with no remedy. Named rather than reached by a wildcard.
        "VIBE_S_IGNORE_NO_EXIT_CODE" => Remedy::NoneAvailable,
        "VIBE_S_IGNORE_UNRECOGNISED" => Remedy::NoneAvailable,
        _ => Remedy::NoEntry,
    }
}

/// The two lines an unknown row gets: core's description, then our remedy.
///
/// `error_lines`' shape, one subject over — `description` then `hint` — and for
/// the same reason: the two must not read as one sentence, because only the
/// second is this build's opinion.
fn unknown_detail(cause: &UnknownCause) -> Vec<String> {
    let wire = cause.to_wire();
    // Core's description. `UnknownCause` has no `Display`, so this is the only
    // English that exists for it and the `message` field is load-bearing rather
    // than a fallback nobody reads.
    let mut lines = vec![format!("    {}", wire.message)];
    lines.push(match remedy_for(wire.code) {
        Remedy::Action(text) => format!("    → {text}"),
        Remedy::NoneAvailable => {
            "    → there is no action here: vibe could not learn anything about this \
             file, and nothing you change locally will alter that"
                .to_owned()
        }
        // Says it does not recognise the cause, rather than printing the code
        // where a sentence belongs (ADR-0001 §4).
        Remedy::NoEntry => "    → this build has no explanation for that cause".to_owned(),
    });
    lines
}

/// Render the listing.
pub fn write_prompt_list_human(
    out: &mut impl Write,
    listing: &PromptListing,
) -> std::io::Result<()> {
    write_rows(out, listing)?;
    write_shadowed(out, listing)?;
    write_user_root_note(out, listing)?;
    write_completeness(out, listing)?;
    write_plugin_note(out)?;
    Ok(())
}

/// The table, or the sentence that stands in for it.
fn write_rows(out: &mut impl Write, listing: &PromptListing) -> std::io::Result<()> {
    if listing.prompts.is_empty() {
        // **The constraint-5 gate.** Zero prompts is a fact only when every root
        // was read to the end. Saying "this project defines no prompts" over an
        // unreadable directory is inventing a value that was not detected, in
        // the one place a reader has no way to check it.
        if listing.is_complete() {
            writeln!(out, "This project defines no prompts.")?;
        } else {
            writeln!(
                out,
                "No prompts were found, and that is not the same as there being none: \
                 vibe could not finish reading every location."
            )?;
        }
        return Ok(());
    }

    let name_w = listing
        .prompts
        .iter()
        .map(|p| p.name.chars().count())
        .max()
        .unwrap_or(4)
        .max(4);

    writeln!(out, "{:<name_w$}  {:<12}  FILE", "NAME", "EXPOSURE")?;
    for prompt in &listing.prompts {
        write_row(out, prompt, name_w)?;
    }

    // **The count is printed only when it is one.** A partial walk still yields
    // rows, and a total over them is a number that reads as complete.
    if listing.is_complete() {
        writeln!(out, "\n{} prompt(s).", listing.prompts.len())?;
    } else {
        writeln!(
            out,
            "\nThis list is partial — see below. The rows above are real; \
                 the set is not known to be all of them."
        )?;
    }
    Ok(())
}

fn write_row(out: &mut impl Write, prompt: &Prompt, name_w: usize) -> std::io::Result<()> {
    writeln!(
        out,
        "{:<name_w$}  {:<12}  {}",
        prompt.name,
        exposure_label(&prompt.exposure),
        prompt.path.display()
    )?;
    if let Some(IgnoreState::Unknown { cause }) = prompt.exposure.state() {
        for line in unknown_detail(cause) {
            writeln!(out, "{line}")?;
        }
    }
    Ok(())
}

/// Shadowed project prompts, which are the sharpest case of a name not
/// resolving to the file it is shown against.
///
/// **They keep their exposure**, and that is the trap this section is written
/// around: unreachable by name is not unexposed. The file is still on disk and
/// still in the repository, so a `git add -A` picks up a `not ignored` shadowed
/// prompt exactly as it picks up a reachable one.
fn write_shadowed(out: &mut impl Write, listing: &PromptListing) -> std::io::Result<()> {
    if listing.shadowed.is_empty() {
        return Ok(());
    }
    writeln!(
        out,
        "\nNot reachable by name — a user-level prompt owns each of these. They are \
         still on disk and still in your repository, so their exposure below still \
         applies:"
    )?;
    for shadowed in &listing.shadowed {
        writeln!(
            out,
            "  {}  {}  {}",
            shadowed.prompt.name,
            exposure_label(&shadowed.prompt.exposure),
            shadowed.prompt.path.display()
        )?;
        writeln!(
            out,
            "      the name runs {}",
            shadowed.shadowed_by.display()
        )?;
        if let Some(IgnoreState::Unknown { cause }) = shadowed.prompt.exposure.state() {
            for line in unknown_detail(cause) {
                writeln!(out, "  {line}")?;
            }
        }
    }
    Ok(())
}

/// Where the `not asked` rows' question belongs.
///
/// §5a requires the root to travel beside the label as an always-present datum —
/// not to explain why, but to tell a reader where to look. Every user-level
/// prompt in one run shares one root, so this is stated **once above the group**
/// rather than per row: a per-row restatement is a per-row opportunity to state
/// it wrongly (ADR-0002 §7).
fn write_user_root_note(out: &mut impl Write, listing: &PromptListing) -> std::io::Result<()> {
    let unasked = listing.prompts.iter().any(|p| !p.exposure.was_asked())
        || listing
            .shadowed
            .iter()
            .any(|s| !s.prompt.exposure.was_asked());
    if !unasked {
        return Ok(());
    }
    let Some(root) = listing
        .roots
        .iter()
        .find(|r| matches!(r.root, PromptRoot::User { .. }))
    else {
        return Ok(());
    };
    writeln!(
        out,
        "\n`not asked` means the exposure question belongs to another repository: these \
         files live under {}, so what this project's `git add -A` would do is not the \
         question to ask about them.",
        root.root.dir().display()
    )
}

/// One sentence per unreadable root, because the two roots fail differently.
///
/// **`is_complete()` is a four-producer observable and one sentence for it would
/// be the collapse phase 2's fifth sabotage found, one level up.** Two roots ×
/// [`RootOutcome::Unreadable`]'s own two branches (the root's own `read_dir`, and
/// the walk not finishing). The two roots have *different consequences*, so they
/// get different sentences:
///
/// - project unreadable → prompts may be **missing** from the list
/// - user unreadable → every project prompt is listed, but any of them may be
///   **shadowed** by a user file vibe could not see
fn write_completeness(out: &mut impl Write, listing: &PromptListing) -> std::io::Result<()> {
    for report in &listing.roots {
        let RootOutcome::Unreadable { why } = &report.outcome else {
            continue;
        };
        match report.root {
            PromptRoot::Project { .. } => writeln!(
                out,
                "\nvibe could not finish reading {}: {why}\n  \
                 Prompts may be missing from this list, and a missing prompt is one \
                 whose exposure nobody sees.",
                report.root.dir().display()
            )?,
            PromptRoot::User { .. } => writeln!(
                out,
                "\nvibe could not finish reading {}: {why}\n  \
                 Every project prompt above is listed, but any of them may be shadowed \
                 by a user-level file vibe could not see.",
                report.root.dir().display()
            )?,
            // `PromptRoot` is `#[non_exhaustive]`, so a root from a later core
            // lands here. It gets its own sentence rather than one of the two
            // above: the whole point of splitting them is that the consequences
            // differ, so handing a third root either consequence would be a
            // specific claim about something this build did not recognise —
            // `unranked_severity_label`'s mistake, one file over.
            _ => writeln!(
                out,
                "\nvibe could not finish reading {}: {why}\n  \
                 This build does not recognise that location, so it cannot say what \
                 the gap costs. Treat the list as incomplete.",
                report.root.dir().display()
            )?,
        }
    }
    Ok(())
}

/// §6's `NotAttempted`, rendered so it reads as neither fine nor error.
///
/// **Unconditional, including when the list is empty**, because the failure this
/// guards against is a reader taking silence for absence. **Once, in the footer,
/// never as a per-row mark** — it is one fact about the whole namespace, and 56
/// copies of it would be the per-site restatement ADR-0002 §7 argues against.
///
/// It names the action rather than a result. *"No conflicts found"* would be
/// false; a warning would be wrong, because nothing failed — this is
/// ADR-0009 §3c's *"we did not detect this"* versus *"there is nothing here"*,
/// and residual failure mode 1 is a reader taking `NotAttempted` for *fine*.
fn write_plugin_note(out: &mut impl Write) -> std::io::Result<()> {
    writeln!(
        out,
        "\nPlugin-supplied prompts were not checked. A plugin can own any of these \
         names, and installing one later changes which file a name runs, with nothing \
         in this project changing."
    )
}

/// `vibe prompt show` — §7's three facts, then the file itself.
///
/// The file goes out **verbatim, frontmatter included**. Frontmatter is stripped
/// from what the model receives but is not inert — `model:` is honoured — so a
/// display that printed the body alone would hide the thing that changes
/// behaviour. And since unknown keys are tolerated, rendering only recognised
/// ones would re-create that omission for every key a future release adds.
pub fn write_prompt_show_human(
    out: &mut impl Write,
    prompt: &Prompt,
    body: &str,
    shadowed_by: Option<&Path>,
) -> std::io::Result<()> {
    writeln!(out, "name      {}", prompt.name)?;
    writeln!(out, "file      {}", prompt.path.display())?;
    writeln!(out, "exposure  {}", exposure_label(&prompt.exposure))?;
    if let Some(IgnoreState::Unknown { cause }) = prompt.exposure.state() {
        for line in unknown_detail(cause) {
            writeln!(out, "{line}")?;
        }
    }

    // **The identity fact, and the reason this command shows it at all.**
    // Showing a file verbatim is honest about content and silent about identity,
    // and identity is what a reader acts on (§7).
    match shadowed_by {
        Some(by) => writeln!(
            out,
            "resolves  no — typing /{} runs {}",
            prompt.name,
            by.display()
        )?,
        None => writeln!(
            out,
            "resolves  not established — plugin-supplied prompts were not checked, and \
             a plugin can own this name"
        )?,
    }

    writeln!(out, "---")?;
    write!(out, "{body}")?;
    if !body.ends_with('\n') {
        writeln!(out)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
