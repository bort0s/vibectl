# ADR-0008: `git init`, Repository Creation, and the Credential That Is Not Used

## Status

Proposed (2026-08-11). Design for P5.

**Amends [ADR-0005](0005-core-api-amendments-for-desktop-consumption.md) §10
rule 3a**, whose open question this answers.

**§6 verified 2026-08-11 (`4de79c0`).** A per-user `gh` config does **not**
redirect `gh repo create`, on `ubuntu-latest`, `macos-latest` and
`windows-latest`. The `gh` path is unblocked.

**§2 implemented 2026-08-11.** `GhOp`, the `gh` half of ADR-0005 §10 rule 1.
§3 is amended and §3a added: the remote is a second opt-in, so `--git` alone
stays local. §9 records the controls and the one caveat still open.

## Context

`vibe new` scaffolds a directory and a manifest. P5 adds the two things a person
does next: initialise a git repository, and create the remote.

ADR-0005 §10 rule 3a left one question open and named it a P5 decision:

> **Open question, deliberately not answered here:** *which* of those ops can
> actually consume a `GITHUB_TOKEN` is a P5 decision. […] `git push` does **not**
> — git has no concept of `GITHUB_TOKEN`, and the usual bridges (a
> `credential.helper`, a token embedded in the remote URL) are respectively
> blocked by rule 2 and unacceptable because the URL gets written into
> `.git/config`. The honest possibilities are that the `gh`-absent fallback
> creates the repo via the API but does not push, or that it uses an askpass
> helper.

Both listed possibilities were worse than a third, and the answer turns out to
remove the question rather than settle it.

## Decision

### 1. `git init` lands here, and the closed enum gains four ops — not five

ADR-0005 §10 rule 1's v1 set was `{Init, Add, Commit, RemoteAdd, Push}`. P5
implements **four of them**:

```rust
GitOp::Init      { cwd }                          // git init
GitOp::Add       { cwd, paths }                   // git add -- <paths>
GitOp::Commit    { cwd, message }                 // git commit -m <msg>
GitOp::RemoteAdd { cwd, name, url: GitUrl }       // git remote add <name> -- <url>
GitOp::CurrentBranch { cwd }                      // git symbolic-ref --short HEAD
```

**`Push` is not implemented, and its absence is the point.** The only path that
pushes is the one where `gh` owns authentication and does the push itself. A
`GitOp::Push` would therefore be an operation with no legitimate caller — and an
op with no caller is an execution primitive kept warm for whoever finds it next.
It stays unrepresentable until something needs it, exactly as `FileOp::Delete`
did until `RemoveOwnedAgent` gave it a bounded reason to exist.

`RemoteAdd` takes a [`GitUrl`], not a `String`. Rule 4's validator was put in
`vibe_core::url` rather than inside the agent store precisely so its second
caller would find it already there; this is that caller.

### 2. `gh` present: `gh` does the whole remote flow

`gh repo create <name> --source=. --push [--private|--public]`. `gh` owns the
credential, which is the original architecture and the reason `gh` is preferred
at all. Nothing about the token reaches this crate.

**Implemented 2026-08-11**, once §6 was green. `GhOp` is the `gh` half of rule
1: one variant, one `(program, subcommand)` pair, argv constructed rather than
passed through. `alias set` and `extension install` — the two invocations
ADR-0005 §10 names as arbitrary-execution-by-design — are **unexpressible**
rather than filtered, the same technique as the missing `FileOp::Delete` and the
missing `GitOp::Push`.

Four things about the shape are decisions rather than details:

- **`--source=.` is a literal and the directory is the process's cwd.** The
  source repository is therefore named without any path reaching argv, which
  leaves the repository name as the only value from outside this crate in the
  whole vector — last, after a `--`.
- **The argument check is an allowlist, and it stops at the `--`.** Every
  element before the separator must be the allowlisted pair or one of four
  named flags; everything after it is data. This *diverges* from `GitOp`'s
  deliberately un-`--`-aware check, and the divergence is the point: there the
  narrowing costs nothing because `Add` only carries paths this crate builds,
  where here it would refuse to create a repository named `alias`. Rule 2 exists
  to catch a value landing in a slot that turns out to be flag-parsed, and a
  slot after `--` in a `cobra` command is not one. Refusing there would be a
  validation rule wearing a containment rule's clothes.
- **A separate two-variant `RepoVisibility`, not the manifest's `Visibility`.**
  The manifest type carries `Other(String)` so a value from a future build
  round-trips, which is right for a field being read and wrong for one being
  turned into a flag: `Other(s)` is a user string one `format!` from argv.
- **The remote is a second opt-in: `--git --private` or `--git --public`.**
  See §3a.

`gh`'s environment is constructed like `git`'s, with three additions and one
refusal. `XDG_CONFIG_HOME` is forwarded on the ground rule 3 already admits
`HOME` — a program cannot find its own configuration without being told where it
is — and it grants nothing `HOME` does not, which is only acceptable *because*
§6 verified that a per-user `gh` config cannot redirect `gh repo create`.
`GH_PAGER` is set **blank**, because `gh` pipes its output through a pager named
in that same per-user config: a command reachable through a config file is the
`ext::` shape in a new place, and blank means there is no pager to name.
`SSH_AUTH_SOCK` is forwarded only because this op pushes, the same scoping
`GitOp::Clone` gets. And no token, per §5.

### 3. `gh` absent: everything local, and say exactly what is left

No remote is created and nothing is pushed. `vibe` initialises the repository,
commits the scaffold, and reports the precise commands that finish the job.

```
Created project and initialised a git repository on branch trunk.
gh not found - vibe cannot create the remote repository or push without it.
Run:
  gh repo create you/project --source=. --push
  # or, if you create the repo on github.com first:
  git remote add origin git@github.com:you/project.git
  git push -u origin trunk
```

**This is the honest-detection rule turned on the tool's own capability.** "vibe
cannot push here" is a fact about *this machine's tooling*, not a property of
the project — the same distinction `SyncNotes::not_attempted` draws when `git`
is missing, and the same reason `NotAttempted` must never collapse into
`NoEvidence`. The tool does not guess, does not half-perform the step, and does
not invent a credential path in order to avoid admitting a limit.

**Amended 2026-08-11: this message is printed when a remote was asked for, not
whenever `gh` is missing.** The message above assumed `--git` implied a remote.
Under §3a it does not, and printing "vibe cannot create the remote" to someone
who asked for a local repository reports a limitation nobody reached — the same
nagging `archive`-on-an-already-archived-project refuses to do. The wording,
the honesty rule and the paste-ready commands are unchanged; only the condition
moved.

`RemoteBlocked` names three reasons, and the renderer says a different sentence
for each, because they need different actions from the user:

| Reason | What the user does |
| --- | --- |
| `GhMissing` | install `gh`, or run the printed commands by hand |
| `NotAuthenticated` | `gh auth login` |
| `NothingToPush` | get a commit first |

They are checked **in the order the user has to fix them**, not in the order
that is cheapest to check. A fresh machine often has neither an identity nor
`gh`, and telling that person to install `gh` sends them after a tool that would
not have run anyway.

`NothingToPush` is checked before `gh` is invoked at all, and that ordering is
the whole reason it exists: `gh repo create --push` against a repository with no
commits creates the remote and *then* fails at the push, leaving an empty
repository on the user's account as the side effect of a command that reported
failure.

Anything `gh` says that is not one of those is a real failure and surfaces as
`ToolFailed` carrying `gh`'s own stderr. A catch-all fourth variant would turn
every `gh` problem into a silent "no remote", which is the swallow
`classify_commit_failure` already refuses.

### 3a. The remote is a second opt-in, and `--git` alone stays local

`vibe new --git` initialises a repository and commits. It does **not** create a
remote, on any machine, however well `gh` is set up. Creating one needs
`--private` or `--public` as well.

Two independent reasons, either of which would be sufficient:

- **`gh` requires the visibility to be stated, and so must we.** There is no
  default we could pick that is not this tool deciding whether someone's code is
  published. That is the same class of act as writing a plausible value into a
  manifest field nothing detected, with a worse blast radius, because publishing
  is not undoable by us.
- **`--git` says "initialise a git repository".** Reading it as "and publish
  this to github.com" would make a flag whose help text describes a local action
  perform an outward-facing one — precisely the objection §8 makes to `git init`
  happening unasked, one step further out.

The cost is that a user who wants the whole flow types one more flag. The
alternative cost is a repository appearing on someone's GitHub account because
they had `gh` installed.

`--private` and `--public` conflict with each other and both require `--git`, so
the argument parser refuses every reading that would need a guess, before
anything is scaffolded.

### 4. The `GITHUB_TOKEN` API fallback is designed and not built

The obvious middle path — call the GitHub API to create the repository, wire
`origin`, and leave only the push — is **deliberately not implemented**.

**The reason is dependency cost, measured rather than asserted.** `vibe-core`
has 8 direct and 49 transitive dependencies. The smallest viable HTTP client
with TLS (`ureq`, `--no-default-features --features rustls`) adds **18 net new
crates**, including `ring` and a full `rustls` stack: a 37% increase in
supply-chain surface, for the fallback path of an optional feature.

This is the `blake3` decision run in the opposite direction and reaching the
opposite answer for the same reason. `blake3` was taken despite not being on the
approved list because it bought a property nothing else could provide: telling
"we wrote this" from "the user edited it" needs a real content hash. This buys
the automation of one command the user can run.

Two costs are specific to this project:

- **A dependency in a library is a dependency in everything downstream.**
  `vibe-core` is a library a Tauri frontend will link, and that frontend already
  has an HTTP stack. Baking one in duplicates it — the same imposition ADR-0005
  refused when it declined to force an async runtime on every embedder.
- **Without the API call the owner is unknowable**, so the path needs
  `--repo owner/name` from the user regardless. The automation was one
  `gh repo create` sitting behind a required argument.

**Revisit trigger, named so this is a decision rather than an omission:** if
this workspace takes an HTTP dependency for any other reason, the API create
becomes nearly free and should be reconsidered on its merits. Absent that, it
stays unbuilt. Same shape as ADR-0005's `PlanId` + `apply_by_id` path, which is
designed, justified, and deliberately not built.

### 5. Therefore `GITHUB_TOKEN` is used nowhere in P5

`GitOp::needs_credential()` remains uniformly `false`, now for a second
independent reason: the store had no op that could consume a token, and P5 adds
no op that wants one.

**Rule 3a's open question is answered by removal, not by scoping.** The safest
handling of a credential is not a narrow environment — it is not needing the
credential. Scoping is what you do when the need is real.

**The cost this has, stated rather than discovered later.** `gh` reads
`GH_TOKEN`/`GITHUB_TOKEN` from its environment, and the environment `vibe` hands
it is constructed, so a machine authenticated *only* by an exported token — a
CI shell, or a developer who never ran `gh auth login` — finds `gh`
unauthenticated here even though `gh auth status` in their terminal is green.

That is a real degradation and it is reported as one:
`RemoteBlocked::NotAuthenticated`, with `gh auth login` as the first line of the
advice and a sentence saying *why* it happened, because "it works in my shell"
is otherwise an unexplainable failure. Forwarding the token would fix it in one
line and would put a credential into a subprocess environment for the first time
in this codebase, which is what rule 3a exists to prevent. **If this is ever
revisited, the argument to beat is not convenience — it is that `gh auth login`
already solves it, on the user's side, once.**

### 6. Verified: a per-user `gh` config does not redirect `gh repo create`

**Not assumed either way. The `gh` path did not ship until this was green.**

ADR-0005 §10 says `gh` is *worse* than `git` because `gh alias set` and
`gh extension install` execute arbitrary binaries by design. Rule 1 means this
crate never invokes those. That is not the question.

The question is the `ext::` finding's shape exactly. Rule 1 closes what we
*invoke*; the `ext::` hole was never in what we invoked. `HOME` and `APPDATA`
must be forwarded — `gh` cannot find its own configuration without them — and
`gh` reads a per-user config through them. `GIT_CONFIG_NOSYSTEM=1` turning out
not to cover `~/.gitconfig` is the standing precedent for assuming nothing about
what a per-user config can reach.

So: **can an alias, an extension, or a config key in a per-user `gh` config
change what `gh repo create` does?**

Verified as a CI test on a runner that ships `gh`, not as a paragraph — the same
treatment the local-clone-hooks claim got, and for the same reason: a test goes
red when a `gh` release changes the answer, where a paragraph goes quietly
wrong.

**The control is paired** (ADR-0002 §7): the hostile config present must
redirect *or demonstrably not*, and the same invocation with no such config must
not. A one-sided control goes quiet the day a `gh` release widens what aliases
can intercept, which is the failure the paired `ext::` control exists to catch.

If an alias *can* redirect `gh repo create`, the containment answer is not a
longer argument filter — it is the same answer rule 4 gave: neutralise the
input. Most likely `GH_CONFIG_DIR` pointed at an empty directory we construct,
which is a constructed constant rather than passthrough and so raises no new
tension with rules 2–3 (`GIT_CONFIG_NOSYSTEM=1` is the precedent).

#### Result (2026-08-11, `4de79c0`)

**A per-user alias does not redirect `gh repo create`.** Green on
`ubuntu-latest`, `macos-latest` and `windows-latest`.

Two things make that result mean what it says, and both were built in
deliberately:

- **A skip cannot masquerade as a pass.** `gh` is not installed on the
  development machine, so these tests pass locally by returning early — the
  exact shape of a control that never reached its guard. CI runs them by name
  with `VIBE_REQUIRE_GH=1`, under which a missing `gh` *panics* rather than
  skips. The step passing is therefore proof `gh` was present and the control
  ran, without anyone having to read a log to check. The `ext::` verification
  needed `--nocapture` output read by a human to draw the same distinction;
  this does not.
- **The negative result is not vacuous.** A `gh` that failed for an unrelated
  reason would also leave no marker, so the clean run must reach `gh`'s own
  `repo create` help or the test fails on that instead.

**This stays a test rather than becoming a sentence here**, for the reason the
local-clone-hooks claim did: a `gh` release that widens what aliases can
intercept turns this red, where a paragraph would go quietly wrong.

### 7. The branch name is read back, never assumed

`git init` honours the user's `init.defaultBranch`. Printing
`git push -u origin main` would therefore be a **plausible-looking guess in the
one output whose entire purpose is being correct enough to paste**.

`GitOp::CurrentBranch` reads it (`git symbolic-ref --short HEAD`) and the report
names what is actually there. `vibe` does not pin `-b main` either: overriding a
user's configured default to make our own output easier is the tool deciding
something that is not its business.

### 8. Git initialisation is opt-in

Behind a flag on `vibe new`, not the default. `vibe new` is a registry
scaffolder; a person who wanted a repository in a directory knows how to make
one, and creating one unasked is the tool doing something that was not
requested. When `git` itself is missing, the flag fails cleanly and says so —
a fact about this machine, reported as one.

### 9. The controls the `gh` path ships with, and the one caveat still open

Four guards were added with the `gh`-present path, and each was **sabotaged and
observed to fail** before being committed (ADR-0002 §7). Recorded because the
rule's whole history is that intending to get this right has not been enough:

| Guard | Sabotage | Observed |
| --- | --- | --- |
| the `gh` argument allowlist | `reject_dangerous_gh_args` returns `Ok(())` | 2 red in `gh::tests` |
| the `NothingToPush` pre-flight | delete the check | red: the runner panicked because `gh repo create` was actually invoked |
| the probe's `--help` placement | append instead of insert | red, naming the argv that would have been a live create |
| the message's remote gating | gate on `false` | red at both unit and CLI level |

The second is the one worth keeping in mind. Its fixture is a runner that
**panics if `gh` is run at all**, so the sabotaged build demonstrably reached
the subprocess rather than failing earlier — the reached-guard rule built into
the fixture. And a third test asserts the op *is* run when both preconditions
hold, so "never calls `gh`" cannot pass by accident.

Two CI-verified controls now run under `VIBE_REQUIRE_GH=1`, in one step:

- `gh_containment.rs` — §6's question, green since `4de79c0`.
- `gh_argv.rs` — **does this `gh` still accept the argv `GhOp` constructs?**
  The unit tests assert the enum against strings and cannot answer that; only
  `gh` can. It runs the real argv with `--help` inserted after the subcommand
  and before the flags, so `cobra` parses every flag and then prints help
  instead of doing anything, and it is paired: the same invocation with
  `--source-tree=.` must be rejected, or the control cannot detect a rename and
  its green half proves nothing. Three independent things stop it creating
  anything — `--help` short-circuits, `GH_CONFIG_DIR` is an empty directory with
  no credential, and the working directory is not a repository.

Both files take the shape §6 established: `VIBE_REQUIRE_GH=1` turns a missing
`gh` into a panic, so **the step passing is itself the proof the controls ran**,
with no log to read and no credential needed to read one.

**Caveat, with a due date rather than a disclaimer.** The `gh`-requiring
assertions in `gh_argv.rs` have not run anywhere yet: `gh` is not installed on
the development machine, and every other check *was* run locally — 303 tests,
`clippy -D warnings`, `rustfmt`, and `cargo +1.85.0 check`, all green on Windows
10. What is unverified is narrow and specific: whether a real `gh` accepts
`--source=.`, `--push` and `--private` in the positions `GhOp::argv` puts them,
and whether it rejects `--source-tree=.`. The `VIBE_REQUIRE_GH` guard itself was
verified locally by running it with `gh` absent and observing the panic. **CI is
the first run of the rest; discharge this by naming the run and the revision,
not by the passage of time.**

**Not tested anywhere, deliberately:** the path where `gh` *succeeds*. It
creates a repository on github.com under whoever is logged in, and a suite that
can do that on a developer's machine or a CI runner is one that will eventually
do it by accident.

## Consequences

**Easier:** the credential question disappears. There is no token in any
subprocess environment, no askpass binary, no `credential.helper`, and no token
written into `.git/config` — because nothing in P5 needs any of them.

**Harder:** the `gh`-absent path leaves the user with work to do, and the
quality of that experience is entirely the quality of one message. A vague
message makes this the worst path in the tool.

**Trade-off accepted #1:** *without `gh`, `vibe new --git` does not produce a
remote repository.* This is a real reduction against the spec's ambition, and it
is the graceful degradation ADR-0005 §10 rule 3a already permitted in writing.

**Trade-off accepted #1a:** *with `gh`, `vibe new --git` still does not produce
one* — the user types `--private` or `--public` too (§3a). One more flag, in
exchange for never publishing a repository nobody asked to publish.

**Trade-off accepted #1b:** *a user authenticated only by an exported
`GH_TOKEN` is told to run `gh auth login`* (§5), because the environment handed
to `gh` is constructed and the token is not forwarded. The alternative is the
first credential in a subprocess environment in this codebase.

**Trade-off accepted #2:** *the API-create fallback is designed and not built,
so a user with `GITHUB_TOKEN` and no `gh` gets no more than a user with
neither.* Both are told exactly what to run. Revisit if the workspace takes an
HTTP dependency for another reason.
