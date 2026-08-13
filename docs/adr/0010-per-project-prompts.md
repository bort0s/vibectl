# ADR-0010: Per-project prompts — index what exists, and make the state visible without being asked

## Status

**Accepted (2026-08-12).** This records the decisions, the measurements they
rest on, and the failure modes that survive them.

Scope is **P6's first feature only** — prompt storage, naming, versioning and
display. Live agent monitoring is the second feature and gets its own document;
the two share a measurement round and nothing else.

**Implementation, phase 1 of 3 (2026-08-12).** The ignore-state instrument:
`GitOp::CheckIgnore`, its 4a enumeration, and the outcome-to-state mapping, in
`crates/vibe-core/src/ignore_state.rs` with its controls in
`crates/vibe-core/tests/ignore_state_git.rs`.

**Phase 2 of 3 (2026-08-13).** The filesystem layer, in
`crates/vibe-core/src/prompts.rs` with its controls in
`crates/vibe-core/tests/prompts_listing.rs`: both roots read, the measured name
derivation, §6's precedence applied at read time, the plugin namespace as
`NotAttempted`, and §5a's `(root, Option<IgnoreState>)` pair. Phase 3 is the
display (§7), and this layer renders nothing.

Three things fell out of building it, all recorded where they belong rather than
here: **§5a** settles the per-repository question and the state shape, **§5b**
refuses the batched instrument, and §5's label genealogy gains a fourth pass.
A fourth is smaller and is in §10 — sabotage found a control covering one of two
branches that produce the same outcome.

The instrument comes first because it is the only part that can be *silently*
wrong: §5's whole argument is that the safety property lives in the state being
visible, and a display of a state that quietly reads `not ignored` for a file
nothing was learned about is worse than no feature. Phases 2 and 3 fail
loudly — a prompt is listed or it is not.

Two things in §5 changed rather than being implemented: the `128` row is split
in two, because git's stderr distinguishes causes the exit status merges; and
residual failure mode 4 is discharged and replaced with a narrower one. Both are
amended in place and marked with the date.

## Context

The feature is born from use rather than architecture: prompts used daily on a
given project, managed by `vibe`, surfaced where the agent already looks —
`.claude/commands/*.md` for Claude Code. The registry thesis applies unchanged:
**the registry indexes what exists on disk and does not become a second home for
the same artifact.**

Everything below rests on measurement performed on 2026-08-12, and a property
belongs to a build rather than to a category, so the versions are part of the
claim:

| | |
| --- | --- |
| Claude Code | **2.1.228**, the native binary bundled in the VSCode extension |
| git | **2.54.0.windows.1** |
| OS / shells | Windows 10 Pro 19045; PowerShell 5.1.19041.6456; Git Bash; node v24.16.0 |
| Codex | **not installed — nothing measured, see §1** |

Three prior sessions produced retractions from asserting facts about someone
else's tool from outside. Every behavioural claim in this document was produced
by invoking the tool and reading what it did, and the round that produced them
also produced ADR-0002 §7's channel rule and its base rate, which is the reason
to trust these particular numbers rather than the previous ones.

## Decision

### 1. Claude Code only, and Codex is a recorded blank

P6 ships Claude Code support. **Codex is unmeasured, not assumed symmetric.**
The tool is intended to cover other agents later; that intent is recorded here
so the absence reads as a decision rather than an oversight.

**No adapter abstraction is built.** A second agent does not exist yet, and an
interface designed against one implementation is machinery before its second
caller — the same ground on which ADR-0001 §4 declined a macro.

**But the agent-specific facts live in one module**, not scattered through the
codebase: the `commands/*.md` name mapping, the flat namespace, the
user-shadows-project rule, the frontmatter behaviour, the argument substitution.
A second agent is then a new module rather than an archaeology exercise. That
costs nothing today and is the whole insurance; it is not an abstraction,
because nothing is factored out to be shared.

### 2. The measured facts this rests on

Read off the `system/init` record of `--output-format stream-json`, which
enumerates the resolved command list, and off session transcripts, which record
the expansion as sent rather than as described.

**Naming.** The invoked name is derived from the path:

| On disk | Invoked as |
| --- | --- |
| `commands/probe.md` | `/probe` |
| `commands/Mixed_Case.md` | `/Mixed_Case` — case preserved |
| `commands/two words.md` | `/two words` — space preserved |
| `commands/sub/deep.md` | `/sub:deep` — **separator becomes `:`** |
| `commands/notmd.txt` | not present |

That last row is a negative, so it carries a positive control: the same
enumeration listed 56 commands including `probe` from the same directory.

**Namespace.** Project-level and user-level both load into **one flat
namespace**, and on an identical name **user-level shadows project-level**,
measured as a pair — with `~/.claude/commands/collide.md` present the expansion
was the user's body, and with only that file removed the same input produced the
project's. The shadowed file is unreachable under any other name. Plugin-supplied
entries sit in the same flat namespace unprefixed: **19 of the 56**.

**Frontmatter is stripped from what the model receives but is not inert.**
`model:` is honoured — a command declaring haiku ran on
`claude-haiku-4-5-20251001` while an identical invocation without frontmatter ran
on `claude-opus-5`, neither passing `--model`. An unknown key
(`vibe-unknown-key`) did not prevent loading.

**Body content survives verbatim.** Multi-line prose, single and double quotes, a
trailing backslash, inline backticks, `$(…)`, `%PCT%`, `$HOME`, and fenced code
blocks all arrived unmodified. Only `$ARGUMENTS` and `$N` are substituted, and
**positionals are 0-indexed** — `$0` is the first token. An unfilled positional
stays literal; `$ARGUMENTS` with no arguments becomes empty. Nothing is
shell-interpreted at any point.

### 3. Storage: one file, one location, and the ignore rule is the thing that changes

Three options were on the table: `.claude/commands/` alone but unversioned; an
owned `.vibe/prompts/` synced into `.claude/commands/`; or `.vibe/prompts/` as
source with `.claude/commands/` generated and ignored.

**The framing assumed `.claude/` being gitignored is fixed. It is not**, and the
measurement is what decides this:

| `.gitignore` | `commands/daily.md` | `agents/a.md` | `settings.local.json` |
| --- | --- | --- | --- |
| `.claude/` — the repository's shape before this ADR | ignored | ignored | ignored |
| `.claude/` + `!.claude/commands/` | **ignored** | ignored | ignored |
| `.claude/*` + `!.claude/commands/` | **not ignored** | ignored | ignored |

**The naive negation does nothing**, because git cannot re-include a path under
an *excluded directory*; changing the entry to `.claude/*` re-includes
`commands/` while leaving third-party agent definitions and local settings
ignored. That is the distinction the repository's own ignore comment already
draws — those are not part of the project, and prompts the user authors are.

So: **`.claude/commands/` is the single location, versioned by a `.gitignore`
change, with no second store and no sync.** It is the only one of the three
consistent with the registry thesis, because the other two create the second home
the registry must not become.

**Rejected, with the failure mode that rejects it:**

- **An owned store synced both ways.** The user edits the file the agent reads,
  because that is where they were working; the owned copy is stale; the next sync
  silently reverts the edit. Data loss, quiet, and at read time neither copy
  identifies itself as authoritative.
- **A generated, ignored `.claude/commands/`.** A fresh clone has no commands
  until a sync runs, and *"no commands"* is indistinguishable from *"this project
  defines no prompts"* — constraint 5 seen from the agent's side rather than
  ours. Worse, keeping the mirror true requires deleting files **under a
  directory vibe does not own**, and a stale generated file cannot be told from a
  hand-written one without a provenance marker. A marker is available — unknown
  frontmatter keys are tolerated — but that makes a destructive operation depend
  on a key a future release could start interpreting.

Writes under the chosen shape are user-initiated and singular. **There is no
mirror to maintain, so vibe never overwrites or deletes anything to keep two
things equal**, which is the entire difference from the option above.

### 4. Polarity B: private by default, published under `shared/`

Both polarities were measured, and both work — two-level re-inclusion included:

| `.gitignore` | `commands/daily.md` | `commands/<sub>/x.md` |
| --- | --- | --- |
| **A** — `.claude/*`, `!.claude/commands/`, `.claude/commands/local/` | published | `local/` private |
| **B** — A plus `.claude/commands/*`, `!.claude/commands/shared/` | **private** | `shared/` published |

Under B, `git add -A -n` offers exactly `.claude/commands/shared/deploy.md` from
`.claude/` and nothing else.

**B is chosen on irreversibility, which decides it without needing the other
arguments.** Under A the failure is a prompt published into a public
repository's history by forgetting to place it, and deleting the file does not
undo that. Under B the failure is a shareable prompt that did not get shared,
which is noticed and repaired by moving it. **Only one direction is one-way.**

Three further reasons, recorded because they are not obvious and were added
after the first costing:

1. **Deferring the split is not neutral.** As mechanism, adding it later is one
   line and a rename. But until it exists everything under `commands/` is
   published, so **deferring is choosing A with the irreversible direction
   live**. The cost of waiting is the exposure during the interval, not the
   rename at the end of it.
2. **The directory-name inference inherits the asymmetry.** §5 forbids vibe from
   inferring privacy from the directory name — `shared/` is a convention, the
   ignore rule is the fact. The *user* infers it anyway, and cannot be stopped.
   Under A, reading `local/` as private when the ordering is wrong produces
   disclosure; under B, reading `shared/` as published when it is wrong produces
   an unshared prompt. The asymmetry therefore survives into human reading, not
   only into mechanism.
3. **Ergonomics converge with safety instead of opposing it.** Most daily prompts
   are personal, so under B the majority sit in the bare directory and keep the
   short name — `/daily` — while the shared minority pays the `/shared:` prefix.
   Safety and typing pointing the same way is rare enough to be part of the
   reason rather than a footnote.

**The cost accepted:** the boundary is visible in the invoked name, permanently,
because the directory *is* the namespace. Moving a prompt across the boundary
renames the command, which breaks typed muscle memory and any prompt that refers
to another by name.

### 5. The state is shown by default, because the failure is "I didn't notice"

**The safety property does not come from the `.gitignore` lines.** It comes from
the per-file state being visible without being asked for. A state available on
demand does not guard against not noticing, which is the only failure this is
defending against, so **the state appears in `vibe prompt list` by default and
is not behind a flag.**

**Four states, and the fourth is not decoration.** They are the outcomes measured
from `git check-ignore` on git 2.54.0.windows.1:

| Condition | Observable | Rendered as |
| --- | --- | --- |
| path ignored | exit `0` | **ignored** |
| path not ignored | exit `1` | **not ignored** |
| not a git repository | exit `128` | unknown |
| no path given / path outside the repository | exit `128` | unknown |
| **git absent** | **`127` from the shell — no git status at all** | unknown |

`128` is **three distinct conditions sharing one number**, and absence is not in
that space. Code that reads "nonzero" as *not ignored* turns an error into
*"you're versioned, all good"* — the inventing direction, and the same class as
the exit-code audit in ADR-0002 §7. **Unknown renders as neither ignored nor
not-ignored**, in every one of its conditions.

**One state, but the cause is carried in the message.** Collapsing the sources
into a single state is right for the display — none of them supports a claim
about the file. It is wrong for the user if the cause is dropped: *"git is not
installed"* and *"this is not a repository"* are both `unknown` and have
**opposite remedies**, and an unknown with no cause is honest and unactionable.
So the state is one and the diagnostic is specific, which is also what
`ErrorPayload`'s `{ code, params }` shape (ADR-0005 §6) exists to carry.

**The `128` row above merged two causes under one label, and the instrument
turns out to distinguish them.** *Amended 2026-08-12, from the phase 1 diff.*
The status does not separate them; **stderr does**, and the reason to look was
that the two have different remedies — *initialise, or move* versus *the path is
simply wrong* — which is the same ground on which git-absent was separated from
not-a-repository in the first place. Measured under `LC_ALL=C`, which this
crate sets positively, so the prose is not at the mercy of the user's locale:

| Cause | Exit | stderr | Remedy |
| --- | --- | --- | --- |
| not a git repository | `128` | `fatal: not a git repository (or any of the parent directories): .git` | `git init`, or move the project |
| a `.git` file pointing nowhere | `128` | `fatal: not a git repository: (NULL)` | same — from the caller's side it is the same fact |
| path outside the repository | `128` | `fatal: <p>: '<p>' is outside repository at '<root>'` | the path is wrong |
| no path given | `128` | `fatal: no path specified` | — unreachable; the closed enum always supplies one |
| usage error | **`129`** | `error: unknown option …` | — a bug on our side |

So they become distinct `ErrorPayload` params rather than one merged cause. Two
details make this a smaller claim than it looks:

- **`129` was not in the table at all.** A classifier keyed on `128` would have
  had nowhere to put a usage error, so the implementation keys on *"not `0`, not
  `1`"* instead. That is a fourth number found while checking a claim about
  three, which is the ordinary rate at which these tables turn out to be
  enumerations of what was tried.
- **The unreachable cause gets no variant.** *"No path specified"* is measured
  and is not representable through `GitOp::CheckIgnore`, so giving it an arm
  would be keeping a branch warm for nobody — the same reasoning that leaves
  `FileOp::Delete` unwritten.

**What this costs is a version-dependent instrument, and the cost is bounded by
construction.** Matching another tool's prose is exactly the shape ADR-0005 §4a
warns goes stale on someone else's release schedule. But the residual arm is
`unknown`: an unrecognised message loses the *specific* cause and keeps the
*state*, because `ignored` and `not ignored` are reached only from exit `0` and
exit `1` and never from reading stderr at all. **A git rewording degrades a
diagnostic; it cannot invent a state.** And it gets a trigger rather than a
hope — the causes are asserted against the real `git` on the machine under
`VIBE_REQUIRE_GIT=1`, so a reworded message turns a CI step red instead of
quietly widening the residual arm.

**The labels are `ignored` and `not ignored` — not `tracked`, and not
`published`. This name is on its fourth pass, and every earlier one asserted
more than the instrument measures — always in the same direction.**

- **`published` was too strong.** Versioned and published are different facts and
  vibe measures neither: a private remote versions without publishing, and the
  path that could check a remote's visibility is permanently untested
  (ADR-0008 §9).
- **`tracked` is too strong for the same reason, one step in.** Exit `1` means
  **no ignore rule applies**, which is not the same as git having the file.
  A prompt created ten seconds ago and never `git add`ed is *not ignored* and
  *not tracked* simultaneously. `check-ignore` answers a question about exclusion
  rules; the index is a different question, which is why §4's polarity table was
  measured with `git add -A -n` and not with this command. **Two tools because
  they are two questions**, and borrowing one's label for the other's answer is
  the overclaim.

**`ignored` / `not ignored` is exactly what is measured**, and it is also the
right frame for the question the feature exists to answer. Under polarity B the
hazard is exposure, and **`not ignored` means a `git add -A` picks the file
up** — which is the thing to know. Tracked-ness is not the question; exposure
is.

- **`different root` is the fourth pass, and it repeats the pattern one level
  out.** *Added 2026-08-13, with §5a's decision.* A user-level prompt carries
  that label instead of an exposure state, on the ground that the exposure that
  matters is the project repository's. The overclaim is that **provenance is a
  path, not an inode.** Vibe decides the label by which directory it read the
  file from — `~/.claude/commands` versus `<project>/.claude/commands` — and
  those are two strings. If `~/.claude` sits inside the project tree, or is
  symlinked into it, the project repository's `git add -A` **would** pick the
  file up, and the label says the question belongs somewhere else. Bounded and
  unusual; understating in the direction that matters, which is the one this
  label exists to catch.

**The genealogy is the finding, not the four labels.** Each pass named a fact
one step further in than the instrument reaches, and each was caught only by
asking what the instrument actually touches:

| Pass | Claimed | Instrument reaches |
| --- | --- | --- |
| `published` | a remote shows it to someone | never contacts a remote |
| `tracked` | git has the file | exclusion rules, not the index |
| `ignored` / `not ignored` | an exclusion rule applies | true — **for the root asked**, and only that one |
| `different root` | this repository cannot expose it | two path strings, not two inodes |

Same direction four times, which is what makes it a pattern rather than four
mistakes: **the label reaches one step past the instrument, and every step is
towards reassurance.** The third row is the one to watch, because it is correct
as written and its correctness is conditional on something the label does not
say — which is exactly how the fourth arrived.

The positive control on the `127` row is recorded because the first reading of it
was wrong: `env -i … | head -2; echo $?` reported **`head`'s** status, `0`. Re-run
unpiped it is `127`, and `exit 3` through the identical channel returns `3`,
which is what establishes that the `127` is git's absence rather than the channel
flattening everything.

### 5a. The question is per-repository, and the answer carries which root it was asked against

*Added 2026-08-13, from phase 2's first constraint. Numbered `5a` rather than
inserted as a new §6, for the reason ADR-0005's renumbering note records.*

**`check-ignore` answers a question about one repository, and §6's two
directories are not in the same one.** §5's state table assumes a single root.
`.claude/commands` is in the project; `~/.claude/commands` is not, and is often
itself versioned in a dotfiles repository. Asking about a user-level prompt with
`project_dir = <project>` returns `PathOutsideRepository` — correct, and it
reads as a fault, when the honest answer is that the question belongs elsewhere.

**Decided: exposure is computed only for project prompts, against the project
root.** Under polarity B the exposure that matters is the project repository's,
and a user-level prompt is not exposed by that repository whatever its own does.
User-level prompts carry a distinct label meaning **different root** — not
`unknown`, not an error, and not `PathOutsideRepository`.

**The label carries no cause, and the reason is structural rather than a count.**
`UnknownCause` exists because a *measurement* failed in ways with opposite
remedies — install git, `git init`, fix the path. **`different root` is not a
measurement outcome. It is a routing decision taken before git is invoked, and
git is never invoked at all**, so there is no failure to have causes.

A second reason was looked for and none reaches the label:

- **Plugin-supplied prompts** cannot: §6's base layer never reads plugin
  directories, so they are never listed as files in the first place.
- **A project with no repository** is `unknown { NotARepository }` — a real
  measurement with a real remedy.
- **A project root that differs from the git root** — a subdirectory of a larger
  repository, a worktree — is a question git answers normally.
- **Shadowing is orthogonal.** A shadowed project prompt is still a file in the
  project repository with a real exposure state; §6's resolution state and this
  label do not collide.

**It does carry one datum, and that datum is not a cause: which root the
question belongs to.** Not to explain why — to tell a reader where to look. One
always-present field, no variants. If a second cause ever seems needed here,
that is evidence the state is wrong rather than that it needs enriching.

**The shape is `(root_asked, Option<IgnoreState>)`, and a fourth `IgnoreState`
variant is refused because it forecloses in the dangerous direction.** The
foreseeable next feature is letting a user-level prompt opt into being asked
about *its own* repository — the dotfiles case, which has an honest answer.
Nothing is built for it now; the question is only whether the shape permits it.

- **A fourth variant does not permit it.** `DifferentRoot { root }` carries the
  root inside the variant, so **computing a real answer destroys the field that
  said which root it was about.** The result is `NotIgnored`, byte-identical to
  a project prompt's `NotIgnored`, and under polarity B that reads as *"a
  `git add -A` in my project would pick this up"* — which is false. The opt-in
  would turn into a hazard.
- **The pair does permit it.** `IgnoreState` stays exactly what it is: the answer
  for **one** root. The prompt-level type carries the root beside it, and
  *different root* is the case where the root is the user's and no state was
  computed. The opt-in is then purely additive — fill the `Option`, change no
  variant, break no consumer — and the pair still says which root the answer is
  about, which is the property the variant loses.

**Phase 1 forecloses nothing.** `check_ignore` already takes the root as a
parameter, so asking a dotfiles repository about its own prompt is expressible
today with no change to the instrument. Only the state shape could have closed
this off, and it is the display-adjacent decision rather than the instrument
that had to be got right.

### 5b. N spawns, and no batched instrument — decided on a limit, not on a cost

*Added 2026-08-13, from the measurement phase 2 was told to take first.*

**Decided: N spawns of phase 1's instrument, unchanged. The batched,
stdout-parsing alternative is refused.**

**The reason is structural and does not trade against the timing.** Today
`ignored` and `not ignored` are reachable **only** from exit `0` and exit `1`,
and that is precisely what bounds §5's version-dependence: an unrecognised
stderr message costs a *diagnostic* and cannot invent a *state*. A batched
instrument reads the per-file answer off **stdout** — `::\tplain.md` for a
non-match against `.gitignore:1:secret.md\tsecret.md` for a match, re-measured
on git 2.54.0.windows.1 — so the state itself would come from a text format, and
a format change could move it. **That is a limit surrendered, not a cost paid,
and the two do not weigh on the same scale. Even if the batched form were free,
the answer would be the same.**

**Poisoning compounds it independently.** Measured: one path outside the
repository makes the whole batch exit `128` with **no stdout for the good
paths**. Fifty-six per-file answers become one whole-listing `unknown`, so an
exposed prompt goes invisible because of an unrelated file. Under polarity B
that is the direction this feature cannot take — §5 exists so exposure is
noticed, and this converts a noticed exposure into silence.

**The cost, recorded with the platform it belongs to.** Measured 2026-08-13,
release build, N = 56 (§2's corpus size), five rounds, through the real
`check_ignore` and `SystemRunner` so the constructed environment and
`--no-pager` are included:

| Arm | Median |
| --- | --- |
| 56 spawns via `check_ignore` | **814 ms** — 14.53 ms per spawn |
| 1 spawn via `check_ignore` | 14.2 ms |
| 1 raw batched spawn, `-n -v`, 56 paths | 16.6 ms — a lower bound, it builds no environment |

The one-spawn arm against the per-spawn figure shows the cost **is** process
creation: this crate's environment construction adds nothing measurable. The
positive control ran in the same invocation — `ignored=42, not_ignored=14,
unknown=0`, a real mix rather than 56 failures wearing a timing.

> **This is a Windows number and must not become a fact about three
> platforms.** Windows 10 Pro 19045, git 2.54.0.windows.1. **Linux and macOS are
> unmeasured**, and process-spawn cost is the single thing that varies most
> between them, so the ratio here does not transfer. It is recorded as the
> likely worst case, and *likely* is an inference rather than a measurement.

**The obvious optimisation is unsound, and is named so it is not
rediscovered.** Asking about the two *directories* instead of the 56 files is
two spawns, but `check-ignore` on a directory does not establish that every file
under it shares that answer: one per-file rule, or one negation, makes a file
differ from its directory — which is exactly the case worth catching. This is
not §5's "do not infer privacy from the directory *name*" — it is asking git and
still getting a confidently wrong per-file answer.

### 6. Resolution: filesystem inference is the base, and the plugin namespace is `NotAttempted`

Shadowing is not a collision to check once. It is a **permanent condition**: a
name that resolves to your file today is shadowed tomorrow by installing a
plugin, with nothing in the project changing. So detection belongs at **read
time**; a check at write time is green forever and wrong the next day. And vibe
listing `/daily` while `/daily` resolves elsewhere is constraint 5 turned on our
own output.

**The base layer is filesystem inference** — read `.claude/commands` and
`~/.claude/commands`, apply the measured precedence — and it **asserts nothing
about the plugin namespace**. Not "unshadowed": `NotAttempted`, which is
ADR-0003's line one medium over.

**Asking the subject is optional enrichment behind a flag.** The resolved list is
available from the `system/init` record and is authoritative by construction,
being the resolver's own answer. It cannot be the primary path: it costs a model
turn, therefore quota, and **on a machine with no authentication it fails
outright** — while constraint 1 says everything works without a key and AI
enrichment is optional and additive. This is not a tension to resolve; it is a
shape already decided.

Its other prices, measured: `claude` is **not on `PATH`** on this machine — the
only binary sits inside the VSCode extension at a path containing `2.1.228`, so
discovery is version-dependent and breaks on update. Under ADR-0005 §10 it is a
new `(program, subcommand)` pair in the closed enum with its own §4a enumeration.
And it moves the problem rather than solving it: to index the prompts you must
run the thing you are indexing.

**Not measured, and it is a price on the enrichment path rather than a gate:**
whether a zero-token invocation reaches that record — an empty prompt, or a
subcommand that lists without a turn. `claude doctor` exists and reads settings
without a trust prompt; whether it emits the resolved list is unknown.

**Deliberately not taken: re-implementing the resolver.** Extending the base
layer to read plugin directories and `enabledPlugins` would be an **allowlist of
sources**, which is the shape ADR-0008 §9 rejected for the path-scoped diff: it
goes stale silently the day a source is added, and reports green while covering
less.

### 7. Display: the raw file, the derived name, and the resolution state

`vibe show` displays **the file, not a parsed rendering of it.** Frontmatter is
stripped from what the model receives but is not inert (§2), so a display that
prints the body alone shows exactly what the model gets and hides the thing that
changes behaviour — a withheld value rather than an invented one, and the same
constraint. And since unknown keys are tolerated, **rendering only recognised
keys re-creates the withholding for every key a future release adds**. That is
ADR-0002 §5's unknown-key survival in a display instead of a write.

**The invoked name is not in the file.** It is derived from the path — case
preserved, spaces preserved, `/` → `:` — and whether that name resolves to *this*
file is a third fact the file cannot carry, because a user-level or plugin
command can own it. Showing the file verbatim is therefore honest about content
and silent about identity, and identity is what a reader acts on.

So the display is three things: **the raw file, the derived name, and the
resolution state** — with the plugin part marked `NotAttempted` per §6.

### 8. The git call site is new, and the containment argument is re-made rather than inherited

Asking git rather than parsing `.gitignore` is right precisely because git
honours the redirects a project may legitimately depend on. That is also what
puts the call site in scope for ADR-0005 §10, and **§4a's enumeration belongs in
the diff that adds it**, not inherited from the ambient-`gitconfig` probe already
in §7's named list.

Enumerated by planting a hostile per-user config and pairing it against an empty
`HOME`:

- **`core.fsmonitor` is spawned by `git check-ignore`.**
  `error: cannot spawn vibe-nonexistent-fsmonitor-probe` appears under the
  planted config and is absent under the empty one. A read-only-looking query is
  an execution surface.
- **`core.excludesFile` changes the answer.** A control file went from exit `1`
  to exit `0` between the two configs. This is a redirect that **must be
  honoured**, not neutralised — it is the reason for asking git at all.
- **An alias cannot shadow a builtin.** `alias.check-ignore = !<probe>` never
  fired, and that negative is meaningful only because the *same config file*
  demonstrably took effect through the two rows above. This is ADR-0008 §6's
  question asked of `git`, answered rather than assumed.
- **`core.pager` was not observed** — but stdout was not a TTY, which is how vibe
  will always invoke it. Whether a TTY changes that is unmeasured; neutralising
  it costs nothing, the same argument as `GH_PAGER=`.

**The enumeration splits, and that finding lives in the rule rather than here.**
Execution surfaces are neutralised, one answer-affecting key is deliberately
preserved, and a mechanical application of 4a's original form would have
destroyed the answer while following the rule exactly. **ADR-0005 §4a is amended
with the two-question form and with why this is not the fourth instance it
predicted** — the enumeration turns out to be per *invocation* rather than per
program. Recorded there because the next person applying 4a to a new call site
will read the rule and not this document.

**The neutralisation goes through constructed environment, not `-c`.** Rule 2
rejects `-c` categorically; rule 3's carve-out already permits constants we
construct ourselves, which is how `GIT_CONFIG_NOSYSTEM=1` is set. Writing this as
an exception to rule 2 is how a rule dies.

**The four states do not arrive through one channel, and the unification is where
one of them disappears.** Three come from an exit status — `0`, `1`, `128` — and
the fourth, git absent, arrives in Rust as a **spawn error**, an `Err` raised
before any status exists. Merging two channels into one enum is ordinary-looking
code, and it is exactly where a stray `unwrap_or`, `.ok()` or `?` collapses
absence into the `1` arm: *not ignored*, rendered **not ignored**, which is the
inventing direction §5 exists to block and the one that reads as reassurance.

So this is the call site where the negative control matters most, and **its
fixture is not the obvious one: it needs a `PATH` with no `git` on it**,
constructed rather than hoped for — ADR-0002 §7's rule that a precondition you
did not construct is not a precondition. Through a shell that absence measures as
`127`, established here with a positive control (`exit 3` through the identical
channel returns `3`). **The Rust-side observable is owed by that diff, not
asserted here**, and it is owed *with* its control rather than as a claim.

**Paid, 2026-08-12.** The observable is `Err(DetectError::NotAttempted)` raised
by the spawn — no status, as predicted, so absence really is outside the
exit-code space on the Rust side too. It is asserted against a `PATH` containing
one empty directory, and paired against the same fixture and the same file with
`PATH` left alone, which must answer `ignored`.

**The fixture needed a shape worth recording, because the obvious one is not
available.** A test cannot construct that `PATH` in-process: `std::env::set_var`
is `unsafe` under edition 2024, which this workspace denies, and is racy across
parallel tests besides. Adding a PATH override to `SystemRunner` would be
product API existing for a test. So the test binary **re-executes itself** with
the constructed environment and runs the real entry point through a real
`SystemRunner` in the child — which is not merely a workaround: a test that
rebuilt `child_env`'s allowlist with its own `Command` would be asserting things
about *its copy* of the environment construction, and would keep passing on the
day the real one changed. Same fixture serves §8's 4a controls, for the same
reason.

**And the guard was verified rather than assumed**, the way ADR-0008 §6 verified
`VIBE_REQUIRE_GH`: run against a `PATH` with no `git` and without the variable,
the file reports **8 passed in 0.00s** — the indistinguishable green this whole
discipline exists to catch. With `VIBE_REQUIRE_GIT=1` the same run fails.

### 9. No launch integration, and this is not open

`vibe` does not spawn `claude`, does not open an editor, and **does not print a
paste-ready shell command.** Printing `claude "<prose>"` produces a shell string,
and a backtick inside a prompt becomes command substitution in whatever shell the
user happens to run — a prompt file is measured to preserve backticks and `$(…)`
verbatim (§2), so the hazard is not hypothetical, it is the normal content of
these files.

What the CLI help and any frontend show is **the slash command name to type**. No
shell string is ever produced. This is the same technique as ADR-0001's missing
`FileOp::Delete` and ADR-0005 §10's closed enum: the dangerous artifact is not
produced rather than produced and escaped.

### 10. What the tests must establish, in this project's terms

Recorded now because ADR-0002 §7's history is that intending to get this right
has not been enough.

- **Any assertion that a prompt is private must be negative-controlled** by
  breaking the ignore rule and observing the state flip — a test that reads
  `not ignored` from a fixture where everything is ignored proves nothing about
  the mechanism.
- **The control must be reached.** A fixture where `git` fails for its own
  reasons before the state is computed is the unreached guard, and it fails while
  looking exactly like proof.
- **The `128` and `127` paths need their own assertions**, since they are the
  ones a "nonzero means not-ignored" defect renders indistinguishable from
  not ignored. The git-absent control needs a **constructed `PATH` with no `git`**,
  and it must assert the state is `unknown` rather than merely *not ignored* —
  those are one boolean apart and only one of them is a lie.
- **That control must be paired**, for the reason ADR-0002 §7 gives for every
  one-sided control: `PATH` without `git` must yield `unknown`, **and the same
  input with `git` present must yield `ignored` or `not ignored`**. An
  implementation that returns `unknown` unconditionally — a broken invocation, a
  swallowed error, a state machine wired to one arm — satisfies the unpaired
  half perfectly, and the green would mean nothing.
- **And it must be paired on the second axis too, which the first pair never
  touches.** Both halves above stay inside the `Err`-versus-`Ok` split. The
  collapse that matters is *inside* `Ok`: a `128` read as a `1`, an error
  rendered as **not ignored**. So a second pair, same `git` present: one input
  yielding `128` (a path outside the repository, or no repository) and one
  yielding `1`.
- **There is a third outcome source, and it is the one written by default.**
  `CommandOutput::status` is `Option<i32>`, so the sources are `Err` (spawn
  failed), **`Ok` with no exit code**, and `Ok` with a code. `success()` is
  `false` for the middle one, so the natural `if success() { … } else { … }`
  renders it as **not ignored** — the inventing direction reached without anyone
  making a mistake, by writing the obvious branch.

  **The arm is named for its consequence, not its cause: "no exit code".** On
  Unix the cause is termination by signal; on Windows there are no signals and a
  code is returned for essentially everything, so `None` there is the
  *we-do-not-understand-this* class — which has an even stronger claim to
  `unknown`. The cause varies by platform and the mapping does not.

  **Coverage, and this is not the unreachable-arm case.** The mapping is
  factored into a function over the raw outcome and tested on all three
  platforms with a synthesised no-exit-code value — *and*, because the outcome is
  genuinely reachable on two of the three runners, **a real test that `SIGKILL`s
  the child, gated to Unix**. Windows keeps mapping-only coverage, recorded as a
  declared platform limit rather than a choice. Filing a synthesised value as the
  best available on all three would be calling something unreachable while it is
  in reach on two runners out of three.
- **Any further measurement of Claude Code requires a channel control first** —
  a known input through the identical invocation shape — per ADR-0002 §7's
  channel rule, whose base rate was established by this very round.

**What phase 2 added, and the one thing sabotage found.** *Added 2026-08-13.*
The listing's controls are paired on four axes and each was sabotaged and seen
to go red before being committed: a private prompt listed with its state
flipping when the rule under it changes; user-level shadowing against the same
file removed; a user-level prompt not asked about against a project one in the
same listing that is, with the *invocations* counted rather than inferred from
the result; and a partial walk against a complete one.

**The finding is the fifth sabotage, which came back green.** A root whose
`.claude/commands` could not be read and a walk that did not finish produce the
**same `Unreadable` outcome from two different branches**, and the control that
looked like it covered them covered only the first — the fixture fails at the
root's own `read_dir` and returns before the walk begins. Breaking the mid-walk
branch left the test green. That is the unreached-guard rule (ADR-0002 §7)
*inside* a control that read as complete, and nothing but running the sabotage
would have shown it.

The repair is a second control on a deterministic, portable version of the other
branch — a tree nested past the walk's depth bound, where a permission failure
would be neither. **The general form is worth carrying: one outcome reached by
two branches needs two controls, and the count of branches is not visible from
the assertion.**

## The four residual failure modes

These survive every decision above. They are named rather than mitigated,
because writing a mitigation nobody has built is how a gap becomes a claim.

1. **The plugin namespace is unresolvable by the base layer, permanently.** A
   name vibe reports can be owned by a plugin installed later, with nothing in
   the project changing. The base layer says `NotAttempted`, which is honest and
   is not an answer — and a reader who takes `NotAttempted` for *"fine"* gets a
   wrong result from a correct display. The enrichment flag can settle it for a
   user who asks; nothing settles it for one who does not.
2. **`not ignored` is neither `tracked` nor `published`, and both gaps are
   unmeasurable from here.** Vibe reads the exclusion rules only. Whether git
   *has* the file is the index's question and is not asked; whether a remote
   shows it to anyone is reachable only through a path recorded as permanently
   untested. A user who reads `not ignored` as *"it is in git, so it is safe"* is
   wrong on both counts, and the label cannot prevent that without asserting
   something unchecked. What it does support is the one inference that matters
   under §4: a `git add -A` would pick this file up.
3. **Vibe writes into a directory it does not own, under someone else's
   schema.** There is no mirror, so nothing is overwritten or deleted to keep two
   copies equal — but tolerance of unknown frontmatter keys is a property of
   build 2.1.228, measured once. A release that starts interpreting a key vibe
   writes changes behaviour with no diff on our side.
4. ~~**The unknown state has two conditions with different remedies, and one of
   them is unmeasured.**~~ **Discharged by the phase 1 diff, and replaced by a
   narrower one.** *Not a repository*, *path outside the repository* and *git
   absent* are now three causes with three remedies, separated because git's
   stderr separates them (§5). The Rust-side observable for absence is paid too:
   it is `Err(DetectError::NotAttempted)` from the spawn, asserted against a
   constructed `PATH` with no `git` and paired against the same input with `git`
   present.

   **What survives is smaller and is a different kind of thing.** Two of those
   causes are told apart by matching another tool's prose, so the *taxonomy* is
   pinned to git 2.54.0.windows.1 in a way the *states* are not. A reworded
   message costs specificity and cannot cost correctness, and CI turns it red
   rather than letting it degrade quietly — but "red on the version that changes
   it" is a trigger, not immunity, and the enumeration behind it is still the
   list of conditions somebody thought to try.

   **And one merge remains, named rather than papered over.** `git` absent and
   this crate's own argument refusal both surface as `NotAttempted`, and the
   error *type* does not distinguish them. The second is unreachable from
   `GitOp::CheckIgnore` for any path containing a separator, so the cause is one
   variant carrying the difference in prose — an honest merge, on the same rule
   that split the other two: separate what the instrument separates, and say so
   when it does not.

**A fifth is mitigated rather than residual, and the distinction is the point.**
`.gitignore` ordering is silent — the four lines work only in that order, and the
naive negation looks right while doing nothing. §5 is what contains it: because
the per-file state is displayed by default, breaking the order shows up as
prompts changing state in a listing the user already sees. What remains is the
interval before anyone looks, which is exactly the class §5 exists to shorten and
cannot close.

## Revisit triggers

Observable events, not judgements that go quiet when they start mattering.

- **A second agent is actually to be supported.** That is when §1's single module
  becomes a second module, and the moment to ask whether anything is genuinely
  shared. Not before: nothing about Codex has been measured.
- **The first frontmatter key vibe writes itself.** That is when residual 3's
  tolerance stops being background and becomes load-bearing, and it arrives as a
  diff someone reviews.
- **The zero-token question being settled**, either way, changes §6's enrichment
  price and nothing else.
- **`.claude/commands/` ceasing to be where Claude Code reads prompts.** The
  whole design is "index what already exists"; if that stops being true the
  feature is not adjusted, it is re-decided.

## Consequences

**Easier:** there is no store, no sync, no schema, and no second copy — the
feature is a reader over a directory that already exists, plus one `.gitignore`
change the user makes once. The registry thesis is preserved rather than argued
about.

**Harder:** the privacy boundary is visible in every invoked name, and moving a
prompt across it renames the command. Display is three facts rather than one, and
one of them is permanently `NotAttempted`.

**Trade-off accepted:** vibe reports state and does not guard it. It cannot stop
a prompt being committed, cannot enforce the ignore ordering, and cannot check
what a remote does with what it receives. Every one of those would require either
asserting something unmeasured or taking ownership of a directory that belongs to
another tool, and both are worse than the visible, honest, partial answer.

**Not decided here, deliberately:** live agent monitoring, which is P6's second
feature and rests on hooks rather than on files. It shares this document's
measurement round and none of its decisions.

**One caveat travels with those shared measurements and must not be dropped on
the way.** The hook payloads were captured from a fixture **nested inside a
Claude Code session**, and the child's environment was measured to be
contaminated by the parent's — `CLAUDE_CODE_EXECPATH` named 2.1.227, the version
the outer session ran, not the 2.1.228 binary under test. That caveat was
*dissolved for attribution*, and only for attribution: `cwd`, `transcript_path`
and `session_id` are inside the payload JSON, so a design that reads the payload
and never the environment does not inherit it. `CLAUDE_PID` survives too, having
been cross-checked five times against the PID the launcher reported. **Anything
else in the monitoring design that depends on how the agent was launched is
still holding the caveat and must be labelled as measured-from-a-nested-fixture
until it is re-established from a plain one.**
