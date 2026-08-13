# ADR-0002: Manifest Schema Versioning and Forward Compatibility

## Status

Accepted (2026-08-10). **Revised (2026-08-10)** by product-owner decision, before
any implementation: `schema_version` changed from a single integer to
`major.minor`, and the archive semantics in §6 were completed. §1, §2, §3, §6 and
§7 are the revised text; the rest stands as originally accepted.

## Context

`.vibe/project.toml` is user-owned data. It lives inside the user's repo, it will
be committed, it will be hand-edited, and it will be read by different `vibectl`
versions on a laptop, a desktop, a CI runner, and eventually a Tauri app — often
out of sync with each other. It will also be edited by AI agents that were handed
a rendered `CLAUDE.md`.

That gives us three failure modes to design against, in descending order of
severity:

1. **`vibe sync` silently drops a key it did not understand.** This is data loss
   in a file the user has committed. It is the failure we must make impossible.
2. **A newer `vibectl` writes something an older one then misreads and
   "corrects."** Two machines fight over a file.
3. **A hand-written or pre-versioning manifest is rejected outright,** turning a
   registry that is supposed to embrace an existing mess into one that demands
   conformance first.

`toml_edit` (rather than `toml`) is already mandated for writes, which gives us
the machinery to preserve comments and formatting — but `toml_edit` only
preserves what you don't overwrite. Preservation is an API-design property, not a
free property of the crate.

## Decision

### 1. The field: `major.minor` as a string, first line of the file

```toml
schema_version = "1.0"

[project]
name = "macroring"
...
```

**Top-level, not inside `[project]`.** It is metadata about the *file*, not a
property of the project. Putting it in `[project]` would mean it appears in the
domain model, in `vibe show` output, and in every render template, where it does
not belong. A bare key before any table header is also unambiguously first in
TOML, so a short prefix read can determine readability before committing to a
full parse.

**A quoted string, not a TOML float.** `schema_version = 1.10` would parse as
`1.1` and silently order *before* `1.9`. A string parsed into `(u16, u16)` has no
such trap and is what a hand-editing human types.

**Two components, not one.** An earlier draft of this ADR used a single integer on
the theory that the only question is "can this build read this file", which wants
a total order. That was wrong, and the reason is §2: it conflated *additive* and
*breaking* change into one number, which forced every additive field to either
bump nothing (leaving no way to describe the file) or break every older binary
in the wild. Two components let the compatibility rule be read directly off the
version.

**Patch is deliberately absent.** There is no third component because there is no
third class of change: a manifest edit either preserves the meaning of existing
keys or it does not.

### 2. The bump rule

- **Minor bump — additive only.** A new optional key or a new table. Older builds
  read the file correctly, ignore what they do not recognize, and preserve it on
  write (§5).
- **Major bump — any change to the meaning of an existing key.** A rename, a
  removal, a type change, a units change, or a change in how a value is
  interpreted. An older build cannot read such a file correctly and must not try.

Stated as one sentence: **minor means "there is more here than you know about",
major means "what you think you know is wrong."**

### 3. Reading a manifest with a version this build does not fully support

| Case | Behaviour |
| --- | --- |
| `major` > supported major | **Refuse the manifest.** `CoreError::SchemaMajorMismatch`. |
| `major` < supported major | Migrate in memory (§4). |
| `major` equal, `minor` > supported minor | **Proceed.** Ignore unrecognized keys, preserve them on write, emit one warning. |
| `major` equal, `minor` <= supported minor | Normal path. |

**On a major mismatch, the *manifest* is refused — not the whole command.** This
distinction is load-bearing and is an interpretation of the decision worth
stating explicitly, because "refuse on every command that reads a manifest" could
be read either way. Aborting `vibe list` across 30 projects because one of them
has a manifest from a future build would break the registry premise in exactly
the multi-machine situation the tool exists for. So:

- `vibe list` renders that project as an error row (name and path from the cache
  or the directory, every derived column replaced with an error marker) and
  continues. The process exit code reflects that something was unreadable.
- `vibe show <that project>` fails with `SchemaMajorMismatch`.
- Every command that would write to it — `sync`, `archive`, `render` — fails with
  the same error.
- `--json` carries the error object in that project's slot, so a script sees a
  structured failure rather than a missing entry.

**Exit codes.** "Something was unreadable" must be distinguishable from "the
command failed", or a script cannot tell a partially-read registry from a broken
one. The full table, owned by `vibectl/src/exit.rs`:

| Code | Meaning | Emitted by |
| --- | --- | --- |
| `0` | Success. Every requested project was read and every requested write applied. | any command |
| `1` | Failure. The command produced no useful result — bad arguments, no registry, an unreadable cache that could not be rebuilt, or a write that was refused or aborted. | any command |
| `2` | **Partial.** Results were produced, but at least one entry could not be read. | read commands only |

`2` is reachable only from `list`, `show` and `scan`. Write commands never exit
`2`: per ADR-0001 §3 a `WritePlan` is all-or-nothing at the decision level, so a
write either applied or did not. A major-schema mismatch on the *target* of a
write is therefore a `1`, while the same manifest encountered while listing 30
projects is a `2`.

`2` is not exclusive to schema mismatches — an unreadable directory or a manifest
with a TOML syntax error produces it too. The rule is about the shape of the
outcome, not its cause.

**Error text names both numbers and the action.** The `Display` impl produces:

```
manifest uses schema 3.0, this build reads 2.x — upgrade vibe
```

`CoreError` carries `found: (u16, u16)` and `supported_major: u16` as data; the
sentence above is `vibectl`'s rendering of it, per ADR-0001 §4.

**The minor-newer warning is emitted once per run, not once per manifest.** Core
emits a `Diagnostic { code: "VIBE_W_SCHEMA_MINOR_NEWER", .. }` per affected
manifest, because core does not know what a "run" is; `vibectl` coalesces them
into a single stderr line naming the count. A `vibe scan` over 30 forward-versioned
manifests must not print 30 warnings.

**There is no override flag.** The earlier draft had `--allow-newer-schema`; it is
removed. Under the revised rule the case it existed for — an additive field in a
newer manifest — now proceeds by default, and the remaining case (major mismatch)
is precisely the one where the build cannot know what it misparsed. In
particular there is **no `--force` on `render`**: rendered `CLAUDE.md` and
`AGENTS.md` are fed into an AI agent's context, and a half-understood manifest
propagated into an agent's context is worse than a refusal. If it renders, it
renders clean.

### 4. Reading an OLDER manifest, or one with no version at all

- **Missing `schema_version` is treated as `1.0`, not as an error.** Hand-written
  manifests and manifests from the pre-versioning prototype must work. Being
  strict here would punish exactly the users this product targets.
- **Older majors migrate in memory, on every read, always.** Migration is a chain
  of pure functions `migrate_v1_to_v2(&mut DocumentMut) -> Result<(), CoreError>`.
  They operate on the `toml_edit::DocumentMut`, **not** on the typed `Manifest` —
  a migration must carry comments and formatting forward too, and a typed struct
  has already thrown those away. An older *minor* needs no migration by
  construction, since minor changes are additive.
- **In-memory migration is never written back implicitly.** A read command leaves
  the file byte-identical. Migration reaches disk only as part of an explicit
  write command, and appears in the `WritePlan` as its own visible `UpdateFile`
  op with `EditReason::SchemaMigration { from, to }`, so `vibe sync --dry-run`
  shows the user their file is about to be upgraded before it happens.

### 5. Unknown-key survival: the typed struct is not the write source

**`Manifest` is a read projection with no way to become TOML.** It does not
implement `Display`. It has no `to_toml()`. Its `Serialize` impl exists only for
`--json`/IPC and is not wired to any TOML serializer. There is no code path from
`Manifest` back to a `.vibe/project.toml`.

Writes go through the only type that can produce TOML:

```rust
// vibe-core/src/manifest/document.rs
pub struct ManifestDocument { path: PathBuf, doc: toml_edit::DocumentMut, original: String }

impl ManifestDocument {
    pub fn open(path: &Path) -> Result<Self, CoreError>;
    pub fn parse(&self) -> Result<Manifest, CoreError>;         // read projection
    pub fn apply(&mut self, edit: FieldEdit) -> Result<(), CoreError>;  // targeted mutation
    pub fn render(&self) -> String;                              // the loaded doc, re-serialized
    pub fn into_op(self) -> Option<FileOp>;                      // None if render() == original
}

/// The complete, closed set of mutations any command may perform.
pub enum FieldEdit {
    SetProjectStatus(Status),
    SetArchived(bool),
    SetDescription(String),
    SetStackRuntime(Detected<String>),
    ReplaceStackFrameworks(Vec<String>),
    ReplaceStackServices(Vec<String>),
    SetRepoRemote(Detected<String>),
    SetRepoVisibility(Visibility),
    SetDeployUrl(Detected<String>),
    ReplaceDeployEnvRequired(Vec<String>),
    SetSchemaVersion { major: u16, minor: u16 },
}
```

Every edit is a targeted mutation of one addressed path in the live `DocumentMut`
(`doc["stack"]["frameworks"]`). Keys nobody addressed are untouched by
construction, because nothing ever rebuilds the document. Comments, key order,
whitespace, and inline-vs-multiline array style survive because they are still
the same `toml_edit` document that was read off disk.

**`vibe new` is the sole exception,** and it is not really one: it creates a file
that does not exist, from a template, so there is nothing to preserve.

Two supporting rules:

- **`Manifest` records what it did not understand,** as
  `unknown: Vec<UnknownKey { dotted_path, kind }>`. This is used *only* for
  reporting — `vibe show` can say "3 keys this build does not understand:
  `deploy.regions`, `context.risks`, `ai.summary`" — never for round-tripping.
  This is also what makes the minor-newer path in §3 safe: the keys are visible
  to the user and preserved on disk, not silently swallowed.

  **An unknown *table* is reported as one entry and is not recursed into.** A
  future `[ai]` section reports as `ai`, not as `ai.model`, `ai.summary`,
  `ai.generated_at`. The user can act on "this build does not understand `ai`"
  — upgrade, or ignore it — and cannot act on a list of sub-keys belonging to a
  feature this build does not have. The named case to revisit is `vibe show`:
  if it ever grows a mode that dumps a manifest for a human to audit key by
  key, per-key detail becomes useful there and this decision should be
  reopened rather than rediscovered. Preservation on write is unaffected
  either way — it comes from never rebuilding the document, not from this
  list.
- **Unknown *values* degrade, they do not fail.** A `status = "hibernating"`
  written by a future build parses as `Status::Other("hibernating".into())`,
  displays verbatim, and is never rewritten. Only an explicit `vibe
  archive`/status change replaces it. The same rule applies to unrecognized
  entries in `stack.services` and friends: they are strings, they are kept.

### 6. `archived` is orthogonal to `status`

```toml
[project]
status = "shipped"
archived = true
```

- **`vibe archive` sets `[project] archived = true` and does not touch
  `status`.** Un-archiving is therefore not a guess: `archived = false` restores
  the exact prior state because the prior state was never overwritten.
- **`vibe unarchive <name>` is part of the v1 CLI surface.** The "un-archiving is
  not a guess" property is the entire justification for a separate key, and
  without an inverse command it is a property nobody can reach. It is one
  `FieldEdit::SetArchived(false)`, the same op `archive` uses with the other
  operand, so it costs a clap subcommand and nothing else. `archive` on an
  already-archived project and `unarchive` on a non-archived one are both
  no-ops that produce an empty `WritePlan` (`ManifestDocument::into_op` returns
  `None` when the rendered document is unchanged), not errors.
- **`status` and `archived` are two dimensions and every combination is legal.**
  `dead` narrows to *abandoned before completion*; `archived` means *off my
  desk*. `shipped + archived` (finished, put away) and `dead + archived`
  (abandoned, put away) are different states and both must round-trip through
  `open → parse → render` unchanged.
- **`vibe list` hides archived projects by default; `--all` includes them.**
  `--json` always includes the `archived` field regardless, because a machine
  consumer filters for itself and a field that appears conditionally is a field
  that breaks scripts.
- **Absent `archived` parses as `false`.** Existing manifests are unaffected, so
  this is a minor-version change per §2, not a major one.

`created` is an ISO-8601 date string (`YYYY-MM-DD`), parsed leniently. TOML has a
native date type; we deliberately do not use it, because half these manifests
will be hand-written or agent-written and a quoted string is what everyone
actually types. An unparseable `created` becomes `Detected::Unknown`, not an
error.

### 7. Constants and testing

```rust
pub const SCHEMA_MAJOR: u16 = 1;
pub const SCHEMA_MINOR: u16 = 0;
```

Only the major is compared for the refuse/accept decision; the minor is compared
only to decide whether to warn.

### The rule for testing a preservation property

Written down because intending to get it right has now failed three times, and
been caught by the same mechanism all three times.

**Any test asserting that something is preserved must (a) place its sentinels on
keys the command under test actually writes, and (b) be negative-controlled by
breaking the preservation mechanism and observing the test fail.**

Sentinels only on untouched keys test that the parser does not corrupt the file.
They do not test what the design exists to guarantee — that the *edited* key
keeps its surroundings. Three times a preservation test was written that passed
against an implementation with decor preservation deliberately removed:

- P1, the `toml_edit` round-trip property: every sentinel was on an unedited key.
- P3, `sync` at the command level: the same omission, in a test written
  specifically because of the first one.

In both cases the test was correct-looking, passed, and proved nothing. In both
cases the negative control found it in one run. **The negative control is not a
nice-to-have here; it is the only thing that has ever detected this.**

**A negative control must demonstrate that the sabotaged guard was *reached*.**
A test that fails before the guard executes proves nothing about the guard.
Build the fixture so the operation succeeds to completion when the guard is
removed.

Added after the fourth appearance of this family, and the sharpest one. The
first three were *weak assertions* — sentinels on keys nothing wrote. The fourth
was different and worse: `agents update` refusing a store directory that is not
ours. The assertion was correct, the guard was sabotaged, and the test still
failed — but it failed because the fixture's `git fetch` errored out on its own,
several steps *before* the sabotaged check would have mattered. The test proved
nothing while looking exactly like proof, and would have kept passing if the
guard had been deleted outright.

Rebuilding the fixture so every step downstream of the check succeeds — giving
the victim repository a working `origin` of its own — showed the sabotaged
build running `reset --hard` on a user's repository to completion. That is what
the control is for, and the distinction between the two fixtures is the whole
lesson: **a negative control is an experiment, and an experiment that terminates
early has not been run.**

**A negative control must also fire *deterministically*.** One whose firing
depends on winning a race can stop proving anything without ever failing, which
is the same outcome as the guard never being reached, arrived at from the other
direction — and worse, because it arrives as a green check.

Added after the fifth appearance, found by review rather than by a failure. The
`ext::` control in `agents_store.rs` pointed the remote helper at
`touch <marker>` and asserted the marker existed. But `touch` is a child `git`
spawns and then kills the moment the helper fails to speak the protocol, so the
assertion was really *"`touch` won that race"*. On a loaded runner it need not,
and the test would then report **pass** — silently returning `GitUrl::parse`'s
`::` rejection to a guard against a hazard nobody had demonstrated.

The repair is the general one: assert on something the system under test does
**synchronously and reports itself**, not on a side effect that has to win.
Naming a program that cannot exist (`ext::vibe-nonexistent-helper-probe`) makes
`git` print what it *tried to spawn* — the execution primitive itself — with
nothing needing to run or exist. Where an assertion genuinely cannot be made
deterministic, it must **fail loudly when it could not run**, never pass quietly.

**And a control must be paired: same input, enabling condition removed, hazard
must be *absent*.** A control that only asserts a hazard is present is sensitive
in one direction. It fires when the hazard exists, and it goes **quiet when the
hazard gets worse** — which is the direction that matters.

The same `ext::` case is the worked example. Asserting only that the transport
runs *with* `protocol.ext.allow = always` in the per-user config leaves two
things unestablished:

1. **That the config is the enabling condition at all**, rather than something
   merely present while the hole reproduced. Nothing tested the difference.
2. **That the hole has not widened.** If a future `git` enabled `ext::` without
   any config, the one-sided control would still pass — the hazard it asserts is
   still there, only now reachable by more people. It would report green while
   the thing it exists to watch got strictly worse.

So the control runs the invocation twice, identical but for the enabling
condition, and asserts the hazard appears in one and is **absent** in the other.
That makes the premise a tested claim rather than an assumption, and makes the
test sensitive in both directions: it fails if the hazard disappears *and* if it
spreads. The general form — **assert the mechanism is present under the
condition and absent without it** — applies to any control whose value depends
on a precondition someone wrote down once and nobody has checked since.

**A fixture must not leave anything running.** The two rules above are about the
control failing to prove what it claims. This one is about the harness producing
a *finding* — an observation about itself, reported as an observation about the
subject.

`scan_never_writes` snapshots a repository, scans it, and asserts nothing
changed. It failed intermittently on `macos-latest` with
`scan modified …/.git/objects/maintenance.lock` — and the scan had not touched
it. `git commit` spawns a **detached** `git maintenance run --auto`, which holds
that lock and removes it when it finishes, asynchronously, after `commit` has
returned. The fixture snapshotted while the lock existed; it was gone by the
second snapshot; the assertion named the scan. `vibe scan` was innocent, and
every hypothesis about *it* was chasing the fixture's exhaust.

Two things to take from it:

1. **Disable asynchrony at the source, do not filter it out.** Teaching the
   snapshot to ignore `*.lock` would have removed the detection along with the
   noise, because a lock file appearing under `.git/` is exactly what that test
   exists to notice. The fixture sets `gc.auto=0` and `maintenance.auto=false`
   instead, so the race cannot occur.
2. **The platform was a red herring, and admitting that mattered.** The failure
   was not macOS-specific in kind; `macos-latest` is the slowest runner, so the
   window between the two snapshots was widest there. "macOS is special" would
   have been a premise, and a wrong one, built from a sample of two.

**An uncompiled test is a test that has not run.** The same family again, one
step earlier: the rules above are about a control that executes and proves less
than it appears to. This one is about a control that never executed at all.

A stretch of ten commits — `7b5dac1` through `b71deca`, covering the `ext::`
control's rewrite, the hooks claim becoming a test, and §7's store-age reporting
— was authored on a machine with no Rust toolchain. **Five of them said so**
(`7b5dac1`, `05a8624`, `9706075`, `3caab2e`, `b71deca`): *"Not compiled — no
cargo on this machine. CI is the first run."* Recording the caveat was right.
Leaving it recorded indefinitely would not be, because "probably fine, CI will
tell us" is a claim about work nobody has checked.

**Discharged 2026-08-11.** CI verified each commit individually on push, across
three platforms; a local stable toolchain then verified the cumulative tree at
`b71deca` on Windows 10 — **233 tests passing, `clippy -D warnings` clean,
`rustfmt` clean, and `cargo +1.85.0 check` green**. The distinction is worth
keeping: CI checked every commit, the local run checked the result of all of
them. Neither alone is the whole claim.

The general rule: a caveat of this shape is a debt with a due date, not a
disclaimer. It is discharged by naming what was run, where, and on which
revision — never by the passage of time or by later work being green on top of
it.

**The harness and the subject must agree about what environment is under test.**
A test that constructs its preconditions differently from how the code under
test observes them is measuring something else — and it fails in the direction
that looks like a finding about the subject.

This is the class several earlier rules are instances of, named late because it
took three sightings to see it.

**The instances are named, not numbered, and this list is the only place that
knows how many there are.** Ordinals were tried and are a bad identifier in this
corpus: "the fifth" already means three unrelated things across these documents
— an instance of *this* rule, a member of the negative-control family, and
product constraint 5 — so an ordinal disambiguates nothing on its own. Worse,
every new instance renumbers the ones after it, and the mechanism that would
have to catch a missed renumber is a repository-wide search, which is the thing
that has now failed twice in two days. A name survives insertion, does not
collide across series, and cannot be silently stale.

**What naming fixes and what it does not, in the same three results measured for
`RemoteBlocked::ALL`** — this is the identical hole in prose, and it is recorded
so nobody tries to close it:

1. **Adding an instance to the corpus without adding it here is caught by
   nothing.** Prose has no compiler, so this is strictly *weaker* than `ALL`,
   where a new variant at least fails to build twice before anyone can ignore
   it.
2. **A list missing an instance reads exactly like a complete one.** There is no
   independent enumeration of prose, so any check driven by this list is
   circular for the same reason it is for `ALL`: an instance absent from the
   list is never visited, whatever the check asserts about the ones that are.
3. **Removing an entry is uncaught too** — `ALL` at least has an array length
   the type checker enforces; a bullet list has no arity.

So the claim is bounded: naming removes the *renumber cascade*, which was the
thing repeatedly going stale. It does not make the list complete, and no
arrangement of the list can. Completeness here depends on whoever writes the
next instance remembering — which is the standing cost of every prose index and
is not worth machinery to pretend otherwise.

- **The root-relative path fixture** (P2, `Interest`). The fixture built paths
  one way, the walker resolved them another, and the disagreement read as a
  detector bug.
- **The exit-status-discarding `git()` helper.** It swallowed exit status, so a
  failed `git commit` let the hooks control report "the probes never fired" — a
  false negative accusing a subsystem that was innocent.
- **The ambient-`gitconfig` identity probe** (`new_git_cli`). The test read
  `git config user.email` in *its own* environment; `vibe` runs `git` under
  `env_clear()` plus `GIT_CONFIG_NOSYSTEM=1`. A system-level `gitconfig` is
  visible to one and invisible to the other, so the two disagreed about whether
  an identity existed. It passed on Linux and Windows and failed on macOS, and
  the failure named `vibe`.

The fix is never a better probe. It is to stop reading ambient state: plant the
environment the test intends — a `HOME`, a config directory, a `PATH` — and pass
it to the subject the same way the subject will read it. A probe and a subject
that consult different sources will eventually disagree, and the disagreement
surfaces as a platform-specific red that costs a day.

**The inherited-environment `gh` probe — the first sighting in shipping code
rather than in a test, and that is worse.**
`repo::gh_available()` span `gh --version` with the *inherited* environment,
while the `gh` invocation it gates would have run under `env_clear()` plus an
allowlist. Same two sources, same eventual disagreement — except that no test
observes a product disagreeing with itself, so it does not arrive as a red at
all. It arrives as a user reporting that `vibe` said `gh` was there and then
could not run it. The probe now goes through the same constructed environment
as the operation, and the general form is worth stating: **a capability check
must observe the environment the capability will actually run in.** A test
harness measuring the wrong world costs a day of debugging; a product measuring
the wrong world ships.

**The locale-blind `grep` — the one that shows how wide this is.**
Removing a name from the repository, the search was
`grep -rniE "ens(ō|o)\b"`. It reported **zero matches over a tree containing
three** — and reported it in the tone of a completed removal. The disagreement:
the pattern held a non-ASCII character, `-i` was applied to it, and the shell's
locale was never declared by either side, so the regex engine and the files
disagreed about **what a character is**. Neither the harness nor the subject was
wrong on its own terms; they were reading different worlds, and the environment
that separated them was one nobody had thought of as an environment at all.

Three things make this the useful instance rather than a footnote:

- **The failure direction inverted, and that is the whole point.** Every earlier
  instance produced a *false red* accusing an innocent subject — loud, and
  somebody investigates a red. This produced a **false green** endorsing a
  removal that had never happened, and **nobody investigates a green.** Same
  class, opposite consequence, and the second is the one that survives.
- **It was not in a test.** It was in a verification step — the thing that
  checks the work, where a wrong answer is trusted precisely because it came
  from a check.
- **It arrived before the anticipated one.** ADR-0008 §9 had already named a
  hostile-`gh` fixture as a *deliberate* future instance of this rule. That one
  is still hypothetical; this one landed first, from a locale, from outside
  anywhere anyone was watching. **Anticipating one instance did not stop an
  unanticipated one arriving first**, which is the argument for indexing on the
  failure rather than on the case: a rule about test fixtures would not have
  caught a `grep`, and the next one will not be a `grep` either.

**The two repairs are not peers, and filing them as equals would teach the wrong
lesson.** Within minutes of writing the paragraph above, the same author's next
search was blind again — `"sixth instance"` against `**sixth** instance`, where
markdown emphasis sits between the two words. That second case was **entirely
ASCII**, so the cause-bound repair from the first case would not have caught it.

- **Cause-bound, and it will keep failing:** avoid what made *this* search
  blind — plain ASCII patterns, no `-i` over non-ASCII, no assumption about the
  locale. Necessary, and it only ever closes the causes already met.
- **Cause-indifferent, and it caught both:** the **positive control**. A pattern
  known to be present, run through the identical tooling in the same
  invocation, whose non-zero count proves the instrument was working when it
  reported the zero. **It does not need to know why the tool might be blind**,
  which is exactly why it survives the next cause.

A third failure the same hour looked like the *safe* mode and is worth stating
carefully, because the obvious phrasing of it is wrong. A Python pattern built
with `[/\\]` raised `PatternError` instead of returning empty.

**"A tool that dies is harmless" is false as written. The property is that it
died where someone saw it.** In a shell pipeline without `pipefail`, a stage
that dies mid-chain produces empty output and a zero exit downstream — the exact
false green this rule defends against, manufactured by the failure mode the
naive phrasing files as safe. `cmd | head` reports `head`'s status. So: **loud
only counts where the loudness is observed**, and a crash inside a pipeline is
not.

That has bitten here, and the case is named because a passing mention of it
would be the same burial this section exists to prevent. `cargo test … --exact
definitely_no_such_test 2>&1 | tail -3; echo "exit=$?"` and the same form for a
missing `--test` target both reported `exit=0` — **`tail`'s status, not
`cargo`'s.** Caught immediately and re-run unpiped, which is where the recorded
`0` and `101` in ADR-0008 §9 come from; the piped pair never reached a document.

**Every recorded exit-code finding was then audited, not just that one**, since
one misread makes the class suspect rather than the instance:

| Finding | Reads | Re-measured | Result |
| --- | --- | --- | --- |
| `git diff --quiet` byte-identity (§9) | `$?`, unpiped | yes | `0`, and a positive control on two *different* trees gives `1` |
| `--exact` filter matching nothing | `$?`, unpiped | yes | `0` — matches the record |
| missing `--test` target | `$?`, unpiped | yes | `101` — matches the record |
| `grep -c` positive control on the rename | **stdout**, not `$?` | yes | `3` — matches the record |
| unscoped `--name-only` identity proof | **stdout**, not `$?` | yes | the same two ADR files — matches the record |

**All five were re-run; none was reasoned about.** The column exists because the
last two belong to a different class, and a row that only said "not an exit
code" would be a classification sitting where a reader looks for a verification
status — promoted to *checked* by adjacency to the rows above it. Stating the
class without stating the status is the same ambiguity this audit exists to
remove.

The class distinction is worth keeping for a reason beyond bookkeeping: **for a
stdout finding the pipeline failure mode is the safe one.** A dying producer
yields empty output, so an expected count simply fails to arrive and the check
fails — where a dying producer in an exit-code chain yields somebody else's
zero and the check *passes*. That is why the positive control is expressed as a
count rather than as a status wherever there is a choice.

Nothing recorded was wrong. **Reporting which findings were checked, not only
which changed, is the point**: an audit that names only its hits leaves a reader
unable to tell a clean class from an unexamined one.

**The positive control covers this case too, which is the argument for leaning
on it rather than on failure modes.** A known count that fails to arrive is a
failed control whether the tool crashed, returned empty, or was silently blind
— it does not need to distinguish them. Where a search cannot be made robust,
make it fail where the failure is seen; where that cannot be guaranteed, the
control is what remains.

**A rule that earns its keep on its own author within minutes of being written
has evidence few rules get.** Recorded here rather than smoothed over, because
the temptation is to present the discovery and not the immediate relapse, and
the relapse is the stronger half of the argument.

**The channel between the harness and the subject alters what is sent, and the
result reads as a measurement of the subject.** Measure the channel with a known
input before measuring anything through it.

This is a sibling of the environment rule above, not an instance of it, and the
distinction is worth holding because the repair differs. There, harness and
subject each read their own world and *disagreed*; the inputs arrived intact.
Here the two agree about the world and **the input never arrives as sent** — a
shell, a path translator, an encoder or a variable table rewrites it in transit.
The subject then answers a question nobody asked, correctly, and the answer is
recorded against the question that was intended.

The tell is that the subject is innocent *and* blameless-looking: no error, no
red, a plausible result. Three of the six instances below produced a publishable
false fact about someone else's tool, and one produced the same false fact
twice, byte-identically, which is what a stable measurement looks like.

**The repair is the positive control moved one level up**: send a *known* input
through the identical channel and read back what arrived, before sending the
input whose answer you do not know. `node -e 'console.log(process.argv)'` on the
same invocation shape settles in one run what argument survives, and it does not
need to know which of the channel's many transformations was going to fire —
which is exactly why it survives the next one. Cause-bound fixes ("quote it",
"set the locale") close only the transformation already met; six instances in
one session is the evidence that there is always another.

The instances, named rather than numbered for the reason recorded above, all
from 2026-08-12 and all on Windows:

- **The dropped empty argument.** PowerShell 5.1 discards `""` from a native
  command line, so `--tools ""` reached the subject as a bare `--tools` and the
  run never happened. A harness that then read "the newest transcript" reported
  the *previous* run's bytes. Two different invocations produced identical
  output, which read as a reproducible finding rather than as a stale file.
- **The split quoted argument.** The same shell turned one argv entry into two,
  and the result read as *Claude Code truncating a quoted slash-command
  argument*. It does not; it groups by quotes correctly. The false finding was
  already written down when the control retracted it.
- **The space-split `Start-Process` argument list.** `-ArgumentList` joins its
  elements with spaces and quotes nothing, so a prompt arrived as its first
  word. The subject asked what it was meant to do, and three experiments were
  scored as *"the model declined the task"* — an inference about a model's
  behaviour drawn from a string it never received.
- **The MSYS-rewritten leading slash.** Git Bash converted `/probe2 …` to
  `C:/Program Files/Git/probe2 …`. The one channel whose transformation is
  documented and still unguessable from the failure.
- **The BOM ahead of the slash.** A UTF-8 BOM emitted into the child's stdin
  meant the prompt did not begin with `/`. It read as *"stdin does not expand
  slash commands"* — a clean, quotable, entirely false capability claim.
- **The clobbered case-insensitive variable.** `$s` overwrote `$S`, the scratch
  path, because PowerShell variable names do not distinguish case. The channel
  here is the harness's own name table, which is why the rule is about channels
  and not about shells.

Note what the list does *not* contain: a case where the subject was wrong. Six
for six, the tool under test behaved correctly and the instrument reported
otherwise.

**So the base rate is the operating rule, not a moral at the end of a list: when
the instrument and the subject disagree, suspicion goes to the instrument
first.** Not second, and not "consider both" — first, and the subject is not
touched until the channel has been shown to be intact with a known input.

The evidence is a whole measurement round rather than a selected case. Every
discrepancy observed while measuring Claude Code 2.1.228 on 2026-08-12 was the
instrument; the count of exceptions is zero. That is the number the ordering has
to be set by, and it is not close enough to argue about.

The cost of the other ordering is already paid and worth naming, because it is
not a wasted experiment. Three kill attempts were recorded as *"the model
declined the long task"* — a conclusion about a third party's behaviour, drawn
from a prompt that reached it as its first word. Wrong claims about our own code
are caught by tests; **a wrong claim about someone else's tool has no such
mechanism**, travels in prose, and is exactly what this project has now had to
retract repeatedly. Suspecting the instrument first is cheap — one control run —
and it is the only step that fails safe.

**The repair is structural now, because the rate earned it.** Seven instances,
roughly one per boundary. That rate is not carelessness — Windows plus
PowerShell plus Git Bash plus MSYS plus markdown is a genuinely hostile
measurement environment, and the rate is evidence for that rather than against
the measurer. But a rule remembered at each invocation pays its cost every time,
and this project's own standard is no machinery before the third caller. This is
the seventh.

`scratchpad/probe.js` is a search that **cannot report a zero without a passing
control in the same invocation**: it echoes argv as received, so a channel that
split or rewrote an argument is visible rather than inferred; it exits `2` and
prints no target count when the control matches nothing; and it reads UTF-8 and
matches with JS regex, so no shell locale decides what a character is. It lives
in the scratchpad rather than the crate, because it is an instrument and not a
product.

**It produced a false zero on its first real use, and that is the more valuable
half.** The first version matched **line by line**, so a phrase spanning a
newline could never match — the exact blindness it was built to prevent, inside
the prevention mechanism. Its control passed throughout, because the control was
a single-line string that exercised nothing of the hazard. Only comparing the
result against a prior measurement from a *different* instrument caught it.

**So a control proves only the hazard class it exercises**, and that is the part
which generalises past this tool. A control showing the file was read does not
show that a multi-line pattern could have matched; an ASCII control says nothing
about a non-ASCII target. **The control must be of the same shape as the
target**, or its green covers a narrower claim than the one being made — the
instrument rule above, arriving inside the positive control itself.

**A precondition you did not construct is not a precondition.** Worked example,
because it is genuinely surprising: *"no git identity configured" is not a
deterministic state.* When `user.name`/`user.email` are unset, `git` may
**auto-detect** an identity from the login name and the hostname and commit with
a warning instead of failing. Whether that succeeds depends on whether the
machine has a resolvable hostname — a CI container often does not
(`runner@fv-az123.(none)`) and the commit fails, while a developer machine
usually does and it succeeds. A test asserting "vibe reports a missing identity"
was therefore asserting against a coin flip weighted by platform.

`user.useConfigOnly = true` in the planted config makes the refusal
unconditional, and `git` emits the identical *"Author identity unknown"* it
emits when auto-detection fails — so the condition under test is the real one,
produced rather than hoped for. Anyone writing the next identity-adjacent test
needs this; it is not discoverable from the failure.

**Recorded honestly: that fix was elimination, not diagnosis.** CI logs require
authentication that was not available, so the second attempt removed *both*
remaining platform variables — the auto-detected identity and the detached
`git maintenance` — without confirming which one was the cause. Each is
independently required by a rule above, so the change is right either way, but
neither was individually established as the failure. Written down because "we
removed two things and it went green" reads as a diagnosis six months later
unless someone says it was not one.

**Prove the control ran without needing to read a log.** The `ext::`
verification distinguished "passed" from "returned early at the availability
guard" by a human reading `--nocapture` output on three runners. That works
exactly as long as someone reads it, and it stopped working the moment CI logs
turned out to need a credential nobody had.

The `gh` containment control does better: CI sets `VIBE_REQUIRE_GH=1`, under
which a missing `gh` **panics** instead of skipping. The step passing is
therefore itself the proof that `gh` was present wherever the guard was
consulted — the reached-guard rule built into the mechanism rather than enforced
by discipline. **Prefer this shape for every CI-verified control.**

**And sabotage the reporting mechanism too, because it is also a guard.** This
paragraph first claimed the green step proved the controls had *executed*. That
was tested on 2026-08-11 and was false: forcing each control file in turn to
return before its guard call left the step green on all three runners
(ADR-0008 §9 has the runs). A panic proves one skip point was reached, not
every one.

The corrected statement is worth carrying because the shape recurs:
**`VIBE_REQUIRE_GH` closes an environment-shaped hole, not a code-shaped one.**
It converts "this machine lacks the tool, so the control skipped" — the
`ext::`-era failure, a runner's configuration silently voiding a check — into a
failure. It cannot convert "this control no longer checks anything" into one,
because that hole is inside the test rather than around it and an exit status
has nothing to observe. A proof-carrying result is still the right design; it
just proves a narrower thing than it is tempting to write down, and the way to
find out which is to break it.

Why *that* shape and not a better channel, confirmed on 2026-08-11: ADR-0008 §9
was discharged by reading one step's conclusion from the public Actions API,
with no credential and no log. Job and step *conclusions* turn out to be public
for a public repository even though logs are not — a channel this section
assumed was closed. That discovery is not the lesson and treating it as one
would be the wrong repair, because **a conclusion cannot distinguish a control
that ran from one that skipped**: both are green, which is the failure this rule
exists against, and it would remain true if every log on earth were
world-readable. What made the public channel sufficient is that
`VIBE_REQUIRE_GH` had already moved the information *out* of the log and into
the exit status. A control whose **result** carries the proof does not care
which channels are open; one that needs its output read is hostage to whichever
channel happens to be available that week.

**An instrument's properties are measured with the instrument, and they belong
to a build of it rather than to its category.** Not assumed from how the
instrument is described, not inferred from how it is usually used, and not
carried over from a different version — asked, of the thing itself, before any
result it produces is trusted.

Indexed on the action deliberately. The obvious phrasing of this case is *"a
mechanism's green means only as much as its granularity allows"*, and that is
true, but granularity was merely this instance's topic. The next wrong
assumption will be about what the instrument counts as **caught**, how it scores
an **unviable** result, what its **timeout** does to a slow case, or which of its
operators are **enabled by default** — and a rule indexed on granularity does
not fire for any of them. Indexed on the action, it fires for all of them,
including the two rules above, which are the same failure with the instrument
being a fixture and an exit status:

- **The unreached guard.** The fixture's granularity was *"the command
  failed"*, which cannot separate "the guard fired" from "an earlier step
  errored". The assertion was correct and the mechanism could not carry it.
- **`VIBE_REQUIRE_GH`.** The exit status's granularity is *"every guard call
  that executed found the tool"*, which cannot separate "every control ran"
  from "none did". Two sabotages established that (ADR-0008 §9), and the claim
  was narrowed to what the granularity supports.
- **Mutation testing, where the assumption ran the other way.** This project's
  hand sabotage — delete a guard, require red — is mutation testing done
  manually, so the tool was evaluated as the general form. The evaluation
  asserted that `cargo-mutants` replaces whole function bodies and therefore
  could never express removing one guard *clause*, and concluded that a green
  run would say less than it appears to. **That was read from outside the tool
  and was wrong.** `cargo mutants --list` settles it without running a single
  test, and on 27.1.0 it emits `delete ! in create_remote` at exactly the
  `if !committed` line, alongside `replace reject_dangerous_gh_args -> …
  with Ok(())`, which is character-for-character the sabotage a human had
  already run by hand.

The third instance is the useful one precisely because the error went the
*generous* direction: assuming a mechanism is coarser than it is discards a
proof you already have, and assuming it is finer manufactures one you do not.
Both are the same mistake — a conclusion resting on an unmeasured property of
the instrument — and both are cheap to avoid, because instruments have ways of
being asked. `--list`, a dry run, a deliberate injection.

**Record the version beside the answer**, or the measurement decays into an
assumption of exactly the kind it was made to replace: a claim that omits the
version silently becomes a claim about whatever is installed today.

**A sabotage whose expected result is green proves nothing without a separate
assertion that the edit landed.** Red carries its own proof of application — a
no-op edit leaves the test green, so a test that went red must have seen the
change. Green does not: it is ambiguous between two opposite conclusions, *the
sabotage applied and the control is inert* and *the sabotage never applied*.
Same colour, inverse readings, and the wrong one is the one that flatters the
control.

Found on 2026-08-11 while negative-controlling ADR-0001 §4's key-plus-test
chain. A `perl -0pi -e` substitution silently matched nothing; the test passed;
the run would have been recorded as "the control is insensitive here" when
nothing had been done to it. The repair is not "check it landed that time" but
the same shape as `VIBE_REQUIRE_GH`: make the experiment's result carry its own
proof. In practice that is an assertion in the patch script — `assert old in s`
before writing — so a pattern that stops matching **fails the experiment**
instead of passing it.

This is why the sabotage table above is worth reading in one direction only: a
row that says *red* is self-proving, and a row that says *green* is a claim
about the harness as much as about the subject.

**It generalises past sabotages to any search whose expected result is empty.**
"No occurrences remain", "no call site does this", "nothing else branches on
that" — each is a green whose two readings are *the thing is absent* and *the
search was blind*. The locale-blind `grep` above is exactly that failure, and it
produced a clean-looking confirmation of a removal that had not happened.

**The repair is the `VIBE_REQUIRE_GH` shape, and it belongs beside that rule
rather than filed as a searching tip.** Pair the empty result with a **positive
control in the same invocation**: a pattern known to be present, run through the
identical tooling, whose non-zero count proves the instrument was working when
it reported the zero. That is the same property as a missing `gh` panicking —
**the result carries the proof that the mechanism ran**, so nobody has to
remember to check separately, and no second channel has to be read. An empty
result with no positive control is a skipped test wearing a green tick.

Where output genuinely must escape, `$GITHUB_STEP_SUMMARY` is readable without
authentication. **A credential that reads CI logs is not the answer**: this
project's containment story is built on constructing environments rather than
adding channels to them, which is the same argument that rejected an askpass
helper in ADR-0008 §4, and it does not stop applying because the beneficiary is
the person debugging.

**Retracting a claim is not finished when the retraction is written. It is
finished when the residue is swept — across code and comments, not only where
the retraction is recorded.** A claim that has been disproved and left standing
somewhere a writer reads is not dormant; it is the version that gets copied.

Added 2026-08-13, from a two-sided instance in this project's own corpus. The
practice already existed and was applied intermittently, which is the finding:

- **Swept.** When `VIBE_INJECT_HAZARD` was struck (ADR-0008 §9), a residue
  search followed the strike, and no orphaned mention of the struck design
  survived.
- **Not swept.** When the `VIBE_REQUIRE_GH` claim was *narrowed* by two
  sabotage runs — a green step does not prove a control file executed — the
  retraction went into ADR-0008 §9 and the comment in `ci.yml` that asserted
  the disproved half was left alone.

**The consequence arrived weeks later and arrived by copying.** The diff adding
a `VIBE_REQUIRE_GIT` step reproduced the disproved claim **twice** — once in
`ci.yml` and once in the new control file's guard — because the author read the
neighbouring comment rather than the ADR. Four sites in total, two of them
authored long after the experiment that disproved them.

Three things make this a rule rather than an anecdote:

- **The failure direction is the dangerous one.** A retraction that lives only
  in an ADR leaves a *false green* in place: the wrong claim reads as
  established, is adjacent to working code, and nobody investigates it. Same
  shape as the locale-blind `grep` above — nobody investigates a green.
- **Distance from the retraction is what decides who reads which version.** The
  ADR is where the argument is settled; the comment is where the next author
  actually stands. **Correctness must be placed at the copy site**, not merely
  recorded at the decision site. Where several call sites will be written from
  a template, the corrected claim belongs *once, above the group* — a per-site
  restatement is a per-site opportunity to restate it wrongly.
- **The sweep is a search, so it takes a positive control** like any other, per
  the rule immediately above. "No other site asserts this" is an empty result,
  and an empty result with no control is a skipped test wearing a green tick.

The mechanical form: when a claim is retracted or narrowed, search the whole
tree for the *claim*, not for the identifier — the wrong sentence travels in
prose and paraphrase, so a search for `VIBE_REQUIRE_GH` finds the variable and
misses the assertion made about it three lines down.

The corollary, from `W_SCHEMA_MINOR_NEWER`: a warning defined and never emitted
is a policy that exists only in this document. A test asserting a diagnostic is
reported must check that something *produces* it, not merely that a consumer
would render it if given one. **A retraction left unswept is the same failure
with the polarity reversed** — a claim that exists everywhere *except* this
document, which is where it was withdrawn.

The regression test that matters, and that must exist before the first write
feature ships: a corpus of manifests in `crates/vibe-core/tests/corpus/` — each
with comments, blank lines, mixed array styles, and keys from a hypothetical
future minor — round-tripped through `open → apply(every FieldEdit variant) →
render`, snapshotted with `insta`. A dropped comment or a vanished unknown key
becomes a failing snapshot diff, which is the only way this invariant stays true
after the tenth feature. The corpus must include a `shipped + archived` and a
`dead + archived` manifest, per §6.

## Consequences

**Easier:** `vibe sync` is safe to run on any manifest, including ones from
builds that do not exist yet. Users can hand-edit freely and add their own keys;
the tool becomes a good citizen in a file it does not exclusively own.
Multi-machine drift resolves by upgrading `vibectl`, never by losing data.
Additive schema growth — which is the overwhelmingly common case — no longer
breaks any deployed binary.

**Harder:** Every new manifest field requires a `FieldEdit` variant and a
corresponding arm in the document writer — more ceremony than `serde`
round-tripping. The `Manifest`/`ManifestDocument` split means two types where a
naive design has one, and contributors will reach for the wrong one until the API
stops them (it will: `Manifest` cannot produce TOML). Two version components mean
every schema change now requires a judgement about which one moves, and getting
that judgement wrong is what breaks compatibility rather than the change itself.

**Trade-off accepted #1:** *a major mismatch makes a project unreadable rather
than partially readable.* We no longer attempt a best-effort read across a major
boundary. This is a deliberate reversal of the earlier draft: a best-effort read
of a file whose key meanings changed produces confidently-wrong output, which for
a tool whose fifth constraint is "detection is honest" is the worse failure. The
mitigation is that the project still appears in `vibe list` as an error row, so
the registry never silently loses an entry.

**Trade-off accepted #2:** *a `vibe sync` on an old manifest upgrades it in place,
after which an older `vibectl` on another machine can read it but no longer write
it.* Migration is one-way and there is no downgrade path. Migrations are rare by
construction (§2), and we mitigate this by making the migration a visible line in
`--dry-run` output rather than an invisible side effect of an unrelated command.

**Trade-off accepted #3:** *two orthogonal fields mean two things to keep
consistent.* `status = "dead"` with `archived = false` is a legal state that reads
oddly ("abandoned but still on my desk"). We accept it rather than adding
validation, because the alternative — inferring one field from the other — is
exactly the information-destroying coupling §6 exists to avoid.
