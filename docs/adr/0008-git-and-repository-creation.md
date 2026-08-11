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

**Caveat as recorded, and discharged the same day.** When the `gh` path was
committed, the `gh`-requiring assertions in `gh_argv.rs` had run nowhere: `gh`
is not installed on the development machine, though every other check *was* run
locally — 303 tests, `clippy -D warnings`, `rustfmt`, and `cargo +1.85.0 check`,
all green on Windows 10. What was unverified was narrow and specific: whether a
real `gh` accepts `--source=.`, `--push` and `--private` in the positions
`GhOp::argv` puts them, and whether it rejects `--source-tree=.`. The
`VIBE_REQUIRE_GH` guard itself was verified locally, by running it with `gh`
absent and observing the panic — the guard the other proofs rest on, checked
first.

> **Discharged 2026-08-11.** Run
> [31477784150](https://github.com/bort0s/vibectl/actions/runs/31477784150) at
> `0c4720e`, green on `ubuntu-latest`, `macos-latest` and `windows-latest`. The
> **`Verify gh containment and argv` step succeeded on all three**, and under
> `VIBE_REQUIRE_GH=1` that is more than "the tests passed": a missing `gh`
> panics, so the step's success is evidence that `gh` was present for every
> guard call that executed. **How much more is bounded below, by experiment
> rather than by argument** — the first draft of this paragraph claimed the step
> also proved both control files executed, and that claim was false.
>
> Established by that run, in the runners' own `gh`: `gh repo create` accepts
> the argv `GhOp::argv` constructs, rejects `--source-tree=.` — so the control
> can detect a renamed flag rather than only an outage — and a per-user config
> still cannot redirect the command (§6, re-established on this revision rather
> than inherited from `4de79c0`).
>
> Run [31478052772](https://github.com/bort0s/vibectl/actions/runs/31478052772)
> at `03cb963` is green too, and it is worth being exact about what that adds:
> **nothing to coverage.** It ran the same code tree — `03cb963` changes only
> ADR text — on fresh runner instances. What it establishes is *determinism*,
> which §7 requires of a control separately from correctness: the same input on
> different machines produced the same answer, so neither control is sensitive
> to runner state the way the `ext::` control's `touch` race was. A second green
> on an unchanged tree is anti-flake evidence, and reading it as a second
> confirmation of the `gh` behaviour would be counting one observation twice.
>
> **What that run covers, in the project's own two-facts form.** `b21b6b7` and
> `0c4720e` were pushed together, so Actions raised one run, for the tip. The
> *project property* — **this code tree passes CI on three platforms** — is
> established for `b21b6b7`, because the code tree at `b21b6b7` and the code
> tree the run tested are the same bytes. The *per-machine fact* — a run
> labelled `b21b6b7` — does not exist and will not. Those are different claims
> and only the second is missing.
>
> The identity is proved rather than asserted, and here is the invocation so a
> later reader can re-run it instead of trusting this sentence:
>
> ```
> $ git diff --name-only b21b6b7 0c4720e
> docs/adr/0005-core-api-amendments-for-desktop-consumption.md
> docs/adr/0008-git-and-repository-creation.md
> ```
>
> **Unscoped, deliberately.** That is the entire delta between the two commits:
> two ADR files, and nothing else in the repository. It needs no judgement about
> which paths matter, which is exactly what makes it the proof.
>
> The path-scoped form — `git diff --quiet b21b6b7 0c4720e -- crates/
> Cargo.toml Cargo.lock .github/`, exit `0` — was written first and is kept only
> as this footnote, because **it is an allowlist and allowlists go stale.**
> `rust-toolchain.toml`, `.cargo/config.toml`, `rustfmt.toml`, `clippy.toml` and
> `deny.toml` all change what CI does and all sit outside it. If one of them
> lands later, that command still exits `0` and still reads like proof. The
> unscoped listing has no such failure mode and proves strictly more, so it
> leads. Same argument as rule 4's closed scheme allowlist, pointed the other
> way: a denylist of paths-that-do-not-matter would have been the right shape
> here, and enumerating what *does* matter was the wrong one.
>
> This is the same shape as the uncompiled-commit discharge above, which
> distinguished "CI checked every commit" from "a local run checked the result
> of all of them" and kept both. **Neither alone is the whole claim**, and
> collapsing them in either direction — "unverified" when the bytes are covered,
> or "verified per commit" when no such run exists — records something that is
> not true.
>
> The inference this licenses — *a documentation-only commit inherits the
> previous verdict, so its own run adds nothing* — is a **condition, not a
> conclusion**: it holds only while no CI job consumes documentation. Today none
> does; `fmt`, `clippy`, `test` and `msrv` read Rust sources, manifests and the
> lockfile, and the `ci` job reads only other jobs' results. A markdown lint, a
> link checker, or a job that renders these ADRs would void it, and would do so
> without touching this paragraph. State the condition when using the shortcut,
> or the shortcut outlives the thing that made it true.

#### What the green step proves, established by sabotaging it

The claim above was narrowed after being tested. ADR-0002 §7 says a control's
value must be demonstrated by breaking it, and **that applies to the mechanism
that reports the control just as much as to the control** — a proof-carrying
exit status is itself a guard, and an unsabotaged guard is a claim.

The original claim was that a green step proves `gh` was present *and* that both
control files executed rather than returning early. The panic establishes the
first. Nothing established the second: a panic proves one skip point was
reached, not two, and a control file whose tests return before calling the guard
never reaches it at all.

Two sabotages, one control file each, same CI input, run against the baseline of
`31477784150` (both files live, step green):

| Branch | Sabotage | Run | `Run tests` | `Verify gh …` step |
| --- | --- | --- | --- | --- |
| `exp/skip-gh-argv` | `gh_argv`'s `gh` test returns before `gh_available()` | [31480318558](https://github.com/bort0s/vibectl/actions/runs/31480318558) | success ×3 | **success ×3** |
| `exp/skip-gh-containment` | all three `gh_containment` tests return before `gh_available()` | [31480321845](https://github.com/bort0s/vibectl/actions/runs/31480321845) | success ×3 | **success ×3** |

**Both sabotages left the step green on every runner.** `Run tests` staying
green in both is what makes the result conclusive rather than a compile failure
wearing the same colour — had the tree failed to build, both steps would have
gone red and the experiment would have proved nothing.

**The diffs, verbatim, because the table alone asserts the one thing that would
invalidate the experiment.** A `return` placed *after* `gh_available()` would
have produced the same two rows, the same two green runs, and the opposite
conclusion — the reached-guard rule turned on the sabotage itself. Note in each
hunk that the inserted block is immediately followed by the `if !gh_available()`
line: the context lines are what prove the ordering, which is why they are kept
rather than trimmed.

**The commits are kept as annotated tags, so this paste stays checkable.** The
branches were deleted; the commits would then have survived only until the
reflog expired, after which the one mechanism that can detect a corrupted paste
— comparing it against the real diff — becomes unrunnable, and a reader cannot
distinguish that from a correct record. `exp-skip-gh-argv` and
`exp-skip-gh-containment` are tags rather than branches: lighter, off every
branch's history, and excluded from CI by the workflow's `branches:` filter. The
check is therefore re-runnable indefinitely:

```
git diff 03cb963 exp-skip-gh-argv        -- crates/vibe-core/tests/gh_argv.rs
git diff 03cb963 exp-skip-gh-containment -- crates/vibe-core/tests/gh_containment.rs
```

The blocks below were produced that way and compared line-for-line against the
command's output rather than trusted — which found one transcription error, a
dropped `#[test]`, before it shipped. **A verbatim claim that nothing checks is
just a claim**, and this one is now checkable by anyone with the repository.

`exp/skip-gh-argv` (`03cb963..7f218f2`):

```diff
--- a/crates/vibe-core/tests/gh_argv.rs
+++ b/crates/vibe-core/tests/gh_argv.rs
@@ -172,6 +172,9 @@ fn the_parse_error_detector_is_sensitive_in_both_directions() {

 #[test]
 fn the_argv_the_enum_constructs_is_accepted_by_this_gh() {
+    if true {
+        return;
+    }
     if !gh_available() {
         eprintln!("skipping: gh is not on PATH (this is the CI-verified check)");
         return;
```

`exp/skip-gh-containment` (`03cb963..0ce2eab`):

```diff
--- a/crates/vibe-core/tests/gh_containment.rs
+++ b/crates/vibe-core/tests/gh_containment.rs
@@ -141,6 +141,9 @@ fn hostile_config(dir: &Path, marker: &Path) {
 /// `gh repo create` still run `gh`'s own command?
 #[test]
 fn a_per_user_alias_does_not_redirect_gh_repo_create() {
+    if true {
+        return;
+    }
     if !gh_available() {
         eprintln!("skipping: gh is not on PATH (this is the CI-verified check)");
         return;
@@ -189,6 +192,9 @@ fn a_per_user_alias_does_not_redirect_gh_repo_create() {
 /// under test and not something incidental.
 #[test]
 fn the_same_invocation_without_the_config_behaves_the_same() {
+    if true {
+        return;
+    }
     if !gh_available() {
         eprintln!("skipping: gh is not on PATH");
         return;
@@ -229,6 +235,9 @@ fn the_same_invocation_without_the_config_behaves_the_same() {
 /// on. Recorded so the CI log says which.
 #[test]
 fn report_whether_gh_permits_aliasing_a_builtin() {
+    if true {
+        return;
+    }
     if !gh_available() {
         eprintln!("skipping: gh is not on PATH");
         return;
```

All three of `gh_containment`'s tests were made inert, not just the asserting
two, so the sabotage models the file going silently dead rather than one test
being disabled.

So the claim is now this, and no more:

- **What the step proves.** Both control *targets* exist, compile, and pass —
  `cargo test --test gh_containment --test gh_argv` exits `101` if a named
  target is missing, so deleting or renaming a control file turns the step red.
  And every `gh_available()` call that executed found `gh` on `PATH`, because a
  missing one panics.
- **What it does not prove.** That any assertion inside those files ran. Zero
  guard calls and one hundred are the same green.

> **Precondition on the first half: one control, one integration-test target.**
> Under `tests/*.rs` the target name *is* the filename, which is the only reason
> `--test gh_argv` can fail when the control disappears. **Fold a control into a
> module of a shared target and that half of the proof dies silently** — the
> target still exists, `--test <shared>` still exits `0`, and the module can be
> renamed or deleted with the step staying green. Nothing in such a diff would
> touch this section, which is the same half-death the sabotage above found one
> level up. If the layout ever changes, this claim is void until something else
> establishes it.

The distinction generalises, and it is the useful part: **`VIBE_REQUIRE_GH`
closes an environment-shaped hole, not a code-shaped one.** It converts "this
machine has no `gh`, so the control skipped" — the `ext::`-era failure, where a
runner's configuration silently voided the check — into a failure. It cannot
convert "this control no longer checks anything" into one, because that hole is
in the test rather than around it, and an exit status has nothing to observe.

**Closing the second hole needs proof that each control's assertions executed,
and only one thing gives that: the control must go red when the hazard is
present.** A skipped test cannot fail, so a control that goes red under an
injected hazard has necessarily run. No exit status can substitute, because an
exit status has nothing to observe.

The obvious lever does not work in the *forward* direction: `cargo test` exits
`0` when a filter matches nothing (`-- --exact no_such_test` → `0`, verified on
cargo 1.97.1), so naming each control in the verify step would not fail when a
name stops existing.

##### Designed and not built: hazard injection at the input layer

Recorded in full because the shape that is *wrong* here is instructive, and
because the version that is merely expensive should not be re-derived from
scratch.

**The rejected shape: a build-configuration flag.** Inject the hazard into the
library behind `--cfg vibe_hazard_injection`, and have a dedicated job require
the controls to fail. The strongest guard for such a flag — `cfg(all(test,
…))` — **is unavailable here**, and the reason is structural rather than
incidental: the containment lives in `vibe-core`'s library, the controls are
integration tests under `tests/`, and a library linked by an integration test is
compiled *without* `cfg(test)`. The hazard would therefore have to compile into
an ordinary library build. What remains — `compile_error!` on
`not(debug_assertions)`, `check-cfg` for typos, a release job asserting the flag
was unset — guards our builds and not anyone else's.

**And that is the argument that decides it, by symmetry with §6.** `RUSTFLAGS`
and `.cargo/config.toml` are ambient and inherited from parent directories, so
a downstream consumer — the Tauri frontend that links this library — could have
containment removed from *their* build of `vibe-core` by configuration they did
not write and would not see. That is precisely the class of hazard §6 spent a
CI control proving does **not** exist for `gh`: a per-user configuration file
silently redirecting what a program does. It would be perverse to establish that
the subprocess boundary has no such redirect and then introduce one *inside the
library*, reachable by exactly the same kind of ambient file. **The flag is
rejected on that ground, not on cost.**

**The variant that keeps the value: inject the hazard as the control's own
input.** `VIBE_INJECT_HAZARD=1` makes a control build its argv, or its
environment, from the hazardous variant instead of the real one — `gh_argv`
using `--source-tree=.`, say — with its assertions unchanged, so it must fail. A
dedicated job runs the controls with the variable set and **requires a non-zero
exit**. An inert control stays green under injection and is caught. Nothing
enters the library, nothing reaches a shipped artifact, no `cfg` exists to be
set by anyone downstream, and the worst failure mode is a broken test.

Four requirements, three of them learned from the way this section's own claims
failed:

1. **The injected job and the ordinary `Test` job must be required as a pair on
   the same commit, not as independent checks.** A requires-red job has three
   distinct false-pass modes and the pairing closes all three at once: a
   compile error is red (reads as success), a **missing target is red** —
   `cargo test --test <name>` exits `101`, which a requires-red job scores as a
   pass — and an already-failing test is red for a reason that has nothing to
   do with injection. If the same tree is green uninjected, none of the three
   can explain red under injection. This is the same discriminator the sabotage
   experiment above relied on when `Run tests` stayed green.
2. **One `--exact` invocation per control, not one run for all of them.** A
   single injected run going red proves *some* assertion fired, not that each
   control's did — the original hole with an extra job in front of it. Worse, a
   hazard that flips shared setup rather than one control's own input would take
   the whole file red while proving nothing about any individual assertion. Each
   control gets its own invocation, and the injection must be reachable only
   through that control's own input path.
3. **`--exact` is fail-safe in this direction, which is the useful inversion of
   the defect measured above.** A filter matching nothing exits `0`; a job
   requiring non-zero therefore *fails* when a control is renamed or deleted.
   The property that made enumeration useless in the verify step makes it
   load-bearing here.
4. **Every control needs its injection point or the job silently covers less** —
   the same drift this section documents, now with two places to keep in step.
   That cost is real and is the reason this is not built for two files.

**The revisit trigger has to be observable, and the obvious one is not.** "When
the controls outgrow what a reviewer checks by eye" cannot fire: the reviewer
who can no longer check by eye is precisely the one not noticing that they
can't. That is the `ext::` control's failure shape — a condition that goes quiet
exactly when it starts mattering. So the trigger is an **event that produces a
diff someone reviews**, whichever comes first:

- **A seventh control file** under `crates/*/tests/` gated on a `VIBE_REQUIRE_*`
  variable. Crude, but it is a number, and adding one is a diff.
- **The first control modified by someone who did not write it.** The proof by
  eye rests on the reader holding the whole argument; the first hand-off is
  where that stops being true, and it arrives as a reviewable change rather than
  as a gradual loss of attention.

Until one of those fires, the honest position is the narrowed claim above rather
than a mechanism nobody has verified.

##### Two claims that must not be merged, and the gap that remains

A first draft of this section closed with: *"injection is only needed where a
guard's fired state is unobservable — where the guard returns its decision, a
test asserts the decision directly."* That is two claims welded together, one
true and one false.

- **False:** that an observable decision removes the need for injection. It does
  not. `assert_eq!(outcome.blocked, Some(RemoteBlocked::NothingToPush))` in a
  test that returns early is green in exactly the same way as an assertion about
  something unobservable. **The code-shaped hole does not care how convenient
  the assertion was to write** — it is about whether the assertion ran, and
  nothing about the guard's return type bears on that.
- **True:** that making guards report their decision is cheaper than making
  hazards injectable, and is better design regardless. A guard returning
  `Err(..)` or `Some(RemoteBlocked::..)` can be tested by an ordinary paired
  assertion; a guard whose success is silence needs a fixture built around its
  absence. That is a claim about the cost of *writing* the control, not about
  what the control proves once written.

The consequence, stated plainly rather than dissolved by the principle:
**`reject_dangerous_gh_args` and the `NothingToPush` pre-flight have no
continuous proof that their controls execute.** What they have is the one-time
local sabotage recorded in the table at the top of this section — a human ran
it, observed red, and wrote down the result. That is worth more than nothing and
is less than CI. If those controls go inert tomorrow, no job notices.

This is a gap in the argument, recorded as one. It is the gap the injected job
above would close for the two `gh` controls, and the gap that a mutation-testing
pass would close more broadly, since removing a guard's body and requiring the
suite to go red is the same experiment generalised — and it reaches the
library-internal guards that input injection cannot, because there the hazard
*is* the guard's absence.

That last claim is measured rather than assumed. `cargo mutants --list`
(27.1.0) over the five containment modules enumerates **111 candidate mutants**,
and both of the guards named above are among them:

```
crates/vibe-core/src/exec.rs:162:5: replace reject_dangerous_gh_args -> Result<(), DetectError> with Ok(())
crates/vibe-core/src/repo.rs:333:8: delete ! in create_remote          # the `if !committed` pre-flight
```

The first is character-for-character the sabotage a human ran by hand. The
second reaches a single guard *clause*, which an earlier evaluation of this tool
asserted it could not do — see ADR-0002 §7's granularity rule, which that error
produced. Nothing is run in CI; this establishes only that the mechanism can
express the experiment, which is the precondition for the decision, not the
decision.

The two sabotage branches were deleted after the runs. They exist as this table,
not as code — a one-time demonstration in §7's sense, not a permanent job.

#### The `gh`-succeeds path is UNTESTED, deliberately and permanently

Recorded the way `scan_bench`'s cold-cache measurement was: named in capitals,
with the reason and the boundary, so nobody reads a green suite as covering it.

**What is not tested anywhere:** a `gh repo create` that authenticates and
returns success. No unit test, no integration test, no CI job, on any platform.

**Why it stays that way.** That path creates a repository on github.com under
whoever the machine is logged in as. A suite that *can* do that is a suite that
eventually does it by accident — on a developer's laptop, or on a runner with a
token in scope, at which point the side effect is on a real account and no test
teardown can un-publish it. This is not a cost/benefit judgement that could go
the other way with more effort; the thing that would make the test meaningful is
exactly the thing that makes it unacceptable.

**Where verification does stop, precisely.** The seam is checked from both sides
right up to the subprocess boundary:

- `GhOp::argv` produces the vector — asserted against the enum's own output, not
  a retyped copy.
- `gh_argv.rs` puts that vector through a real `gh` and confirms it parses,
  paired against a flag `gh` does not have.
- `gh_containment.rs` confirms a per-user config cannot redirect the command.
- `repo.rs` covers everything downstream of the call: both pre-flight checks,
  the classification of what `gh` says, and the refusal to flatten an
  unrecognised failure into "no remote".

What is unverified is one hop: whether GitHub, having received a well-formed and
authenticated request, does what its documentation says. That is a dependency's
contract, not this crate's behaviour, and the honest place to stop is the last
thing we control.

**The revisit trigger, so this is a decision and not an omission:** if a
disposable test account with a scoped token and automatic repository cleanup
ever exists as project infrastructure, this becomes testable without a side
effect on anyone's real account, and should be reconsidered then. Absent that,
it stays untested and the boundary above is the claim.

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
