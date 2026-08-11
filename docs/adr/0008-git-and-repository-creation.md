# ADR-0008: `git init`, Repository Creation, and the Credential That Is Not Used

## Status

Proposed (2026-08-11). Design for P5.

**Amends [ADR-0005](0005-core-api-amendments-for-desktop-consumption.md) §10
rule 3a**, whose open question this answers.

**§6 verified 2026-08-11 (`4de79c0`).** A per-user `gh` config does **not**
redirect `gh repo create`, on `ubuntu-latest`, `macos-latest` and
`windows-latest`. The `gh` path is unblocked.

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

**Trade-off accepted #2:** *the API-create fallback is designed and not built,
so a user with `GITHUB_TOKEN` and no `gh` gets no more than a user with
neither.* Both are told exactly what to run. Revisit if the workspace takes an
HTTP dependency for another reason.
