# ADR-0009: Ensō (GUI) — deferred to after P7, and the constraints that are not stylistic

## Status

**Deferred (2026-08-12).** Not started, and deliberately not designed here. This
records the constraints that must hold *whatever* the design turns out to be, so
that they are decided before a brief is written rather than argued after a
screen exists.

First ADR to name the desktop frontend **Ensō**; earlier ADRs call it "a Tauri
frontend" or "the desktop consumer", and those references mean this.

## Context

ADR-0001 split `vibe-core` from `vibectl` so a desktop app could consume the
same code paths, and ADR-0005 amended the core API for exactly that consumer.
Neither said anything about what the interface should look like, which is
correct — but "nothing recorded" and "no constraints" are different things, and
the difference is what this document is.

The GUI comes **after P7**. What it will stand on already exists: `vibe scan`
and `vibe list` produce `ScanReport` and `ProjectSummary` today, which is why
one piece of the work (design tokens) does not have to wait for P6 or P7 at all.

## Decision

### 1. No imposed visual direction, and one acceptance criterion

When work starts, Claude Code's design/UX/UI agents propose the interface. No
style is prescribed here. The acceptance criterion is **clean, high quality**,
judged on the result rather than against a specification written in advance by
someone who is not designing it.

**The known failure mode, named so it is not mistaken for the agents'
limitation:** an open brief tends to produce the default — dark mode, cards,
hairline borders, the house style of every developer tool screenshot. If that is
what comes back, **the brief was underspecified; the agents did what an open
brief asks for.** The repair is a better brief, not a different agent, and not a
round of stylistic notes on a proposal that answered the question it was given.

### 2. The order: tokens as their own artefact, then one screen

**Ask for the design tokens first, as a standalone deliverable.** They do not
depend on P6 or P7 — they rest on `scan` and `list`, which exist. Only after the
token system is settled does a screen get designed.

The reason is not tidiness. **A system before a composition, or the second
screen renegotiates the first.** A palette chosen while laying out one screen is
a palette chosen to make that screen look good, and every subsequent screen
either re-opens it or works around it. Tokens first makes the trade-offs visible
once, where they are cheap.

### 3. Four constraints that are not aesthetic, and hold under any style

**a. One visual mark per row, not a composition per project.** With 34 projects
in a list, **legibility is the binding constraint**, not expressiveness. A row
is scanned, not read. Anything that makes a single row more interesting at the
cost of the list being scannable is the wrong trade, however good it looks in a
mock-up of three rows.

**b. Colour encodes status; opacity encodes staleness of the last commit.**
Data encoding, not decoration. Two channels, two variables, and neither channel
is available for anything else — no colour for emphasis, no opacity for visual
hierarchy. See §4, which is where this constraint turns out to be harder than it
sounds.

**c. Honest detection survives into the UI.** A stack that was not inferred is an
**empty field, flagged as not-detected**. No plausible placeholder, no `Unknown`
rendered where a value goes, no greyed-out guess. This is constraint 5 — the
tool never invents a value it did not detect — and it is the one a design agent
violates by instinct, because empty cells look unfinished and every design
instinct is to fill them.

The distinction to hold: *"we did not detect this"* and *"this project has no
stack"* are different facts and must not render the same. That is the same
`NotAttempted`-versus-`NoEvidence` line ADR-0003 draws, one medium over.

**d. Core mute, frontend speaking.** `vibe-core` produces no prose; Ensō writes
its own sentences exactly as `vibectl` writes its own. Already decided and
argued in **ADR-0001 §4**, including why two frontends writing different
sentences for the same reason is correct rather than duplication — cross-
referenced here, not restated.

### 4. What the existing types already force, measured rather than assumed

Written down because these are the details a brief will get wrong, and every one
of them is readable off the code today.

**`Status` has six cases, not five.** `Idea`, `Active`, `Paused`, `Shipped`,
`Dead` — and `Other(String)`, which exists so a value from a future build
round-trips instead of being destroyed (ADR-0002 §5). So a five-colour palette
is not a complete mapping, and **an unrecognised status must not borrow one of
the five colours.** Doing so is precisely the defect fixed in `vibectl` on
2026-08-11, where an unknown `Severity` was rendered as `warning`: not a hedge,
but a specific and false claim. The sixth case needs a treatment that reads as
*"this build does not recognise this value"* — and it must not read as an error,
because it is not one.

**Staleness can be absent.** `ProjectSummary::last_commit` is an `Option`. A
project whose last commit date could not be read has **no position on a
fresh-to-stale scale**, and rendering it at either end is the same invention as
above. Absence needs its own treatment, distinct from both "fresh" and "stale".

**An unreadable project is a row, not a gap.** `ProjectSummary::error` is
populated when the manifest could not be read, and ADR-0002 §3 requires the
project to still appear — an error row, never a missing entry. It is a third
visual state alongside "recognised" and "unrecognised status", and it is the one
that *is* an error.

**`archived` is orthogonal to `status`.** ADR-0002 §6: every combination is
legal, `shipped + archived` and `dead + archived` are different states. Archived
is therefore a **second dimension** and cannot be folded into the status colour
without destroying information the manifest deliberately keeps separate.

**`from_cache` is not staleness.** The flag means the row came from the cache,
whose witness matched — the value is still believed correct. Conflating it with
commit staleness would report a fact about *our read* as a fact about *the
project*, which is the same category error §3c is about.

## Consequences

**Easier:** the token system is buildable now, against data that already exists,
without waiting for P6 or P7. And a design proposal can be judged against
something: four constraints and five type-level facts, rather than taste alone.

**Harder:** §3b spends both cheap visual channels on data, so a designer wanting
emphasis has to find a third channel — weight, spacing, a mark — and §3a limits
what that can be. This is a real constraint on the design space and is accepted
deliberately: a list of 34 projects that reads at a glance is the product, and a
list where each row is individually beautiful is not.

**Trade-off accepted:** *the tokens are specified before anyone has seen a
screen.* That is the point, and it costs something — the first screen may want a
token that does not exist, and adding one is a change to a system rather than a
local decision. Accepted because the alternative is a palette that fits screen
one and is renegotiated by screen two.

**Not decided here, deliberately:** every question of layout, typography,
density, motion, light/dark, and platform idiom. Those belong to whoever designs
it. If this document grows to answer them, it has stopped being a list of
constraints and become the brief it declines to write.
