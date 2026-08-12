# ADR-0010: Per-project prompts — index what exists, and make the state visible without being asked

## Status

**Accepted (2026-08-12).** Scoping complete, no code written. This records the
decisions, the measurements they rest on, and the failure modes that survive
them.

Scope is **P6's first feature only** — prompt storage, naming, versioning and
display. Live agent monitoring is the second feature and gets its own document;
the two share a measurement round and nothing else.

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
| path ignored | exit `0` | not tracked |
| path not ignored | exit `1` | **tracked** |
| not a git repository | exit `128` | unknown |
| no path given / path outside the repository | exit `128` | unknown |
| **git absent** | **`127` from the shell — no git status at all** | unknown |

`128` is **three distinct conditions sharing one number**, and absence is not in
that space. Code that reads "nonzero" as *not ignored* turns an error into
*"you're versioned, all good"* — the inventing direction, and the same class as
the exit-code audit in ADR-0002 §7. **Unknown renders as neither tracked nor
untracked**, in both of its conditions.

**The label is `tracked`, never `published`.** Versioned and published are
different facts and vibe can only measure the first: a private remote versions
without publishing. A label claiming publication asserts something about a remote
that has not been checked, and the path that could check it is permanently
untested (ADR-0008 §9).

The positive control on the `127` row is recorded because the first reading of it
was wrong: `env -i … | head -2; echo $?` reported **`head`'s** status, `0`. Re-run
unpiped it is `127`, and `exit 3` through the identical channel returns `3`,
which is what establishes that the `127` is git's absence rather than the channel
flattening everything.

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
absence into the `1` arm: *not ignored*, rendered **tracked**, which is the
inventing direction §5 exists to block and the one that reads as reassurance.

So this is the call site where the negative control matters most, and **its
fixture is not the obvious one: it needs a `PATH` with no `git` on it**,
constructed rather than hoped for — ADR-0002 §7's rule that a precondition you
did not construct is not a precondition. Through a shell that absence measures as
`127`, established here with a positive control (`exit 3` through the identical
channel returns `3`). **The Rust-side observable is owed by that diff, not
asserted here**, and it is owed *with* its control rather than as a claim.

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
  `not tracked` from a fixture where everything is ignored proves nothing about
  the mechanism.
- **The control must be reached.** A fixture where `git` fails for its own
  reasons before the state is computed is the unreached guard, and it fails while
  looking exactly like proof.
- **The `128` and `127` paths need their own assertions**, since they are the
  ones a "nonzero means not-ignored" defect renders indistinguishable from
  tracked. The git-absent control needs a **constructed `PATH` with no `git`**,
  and it must assert the state is `unknown` rather than merely *not tracked* —
  those are one boolean apart and only one of them is a lie.
- **Any further measurement of Claude Code requires a channel control first** —
  a known input through the identical invocation shape — per ADR-0002 §7's
  channel rule, whose base rate was established by this very round.

## The four residual failure modes

These survive every decision above. They are named rather than mitigated,
because writing a mitigation nobody has built is how a gap becomes a claim.

1. **The plugin namespace is unresolvable by the base layer, permanently.** A
   name vibe reports can be owned by a plugin installed later, with nothing in
   the project changing. The base layer says `NotAttempted`, which is honest and
   is not an answer — and a reader who takes `NotAttempted` for *"fine"* gets a
   wrong result from a correct display. The enrichment flag can settle it for a
   user who asks; nothing settles it for one who does not.
2. **`tracked` is not `published`, and the gap is unmeasurable from here.** Vibe
   reads the ignore state; the remote's visibility is a different fact, reachable
   only through a path this project has recorded as permanently untested. A user
   who reads `tracked` as *"safe, it's only in git"* on a public repository is
   wrong, and the label cannot prevent that without asserting something it has
   not checked.
3. **Vibe writes into a directory it does not own, under someone else's
   schema.** There is no mirror, so nothing is overwritten or deleted to keep two
   copies equal — but tolerance of unknown frontmatter keys is a property of
   build 2.1.228, measured once. A release that starts interpreting a key vibe
   writes changes behaviour with no diff on our side.
4. **The unknown state has two conditions with different remedies, and one of
   them is unmeasured.** *Not a repository* and *git absent* both mean "vibe
   cannot say", but a user can act on only one of them. And the Rust-side
   observable for absence is a spawn error rather than a status, which is stated
   as owed rather than known.

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
