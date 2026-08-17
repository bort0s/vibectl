# ADR-0011: Live agent monitoring — agents report, and the last event is never terminal

## Status

**Scoped (2026-08-12), design open, no code.** P6's second feature. This records
what was measured, the one architectural decision taken in principle, and the
constraints that hold *whatever* the design turns out to be — the shape ADR-0009
used for the frontend, and for the same reason: "nothing recorded" and "no
constraints" are different things.

**Re-pinned to Claude Code 2.1.233 (2026-08-17), and the previous corpus is
deliberately not re-measured.** The build every fact below was first established
on — **2.1.228** — no longer exists on this machine; 2.1.229 and 2.1.233 are
installed in its place. A second round was taken on 2.1.233 and is recorded as
**deltas against the 2.1.228 record**, not as a replacement for it. Re-measuring
the whole corpus was considered and declined: it is expensive and it produces
another corpus that goes stale by the same mechanism on the next release. **The
value is in the delta**, which is what a reader of a versioned measurement
actually needs.

**That round retracted a claim rather than adding one, and the retraction is the
headline.** §5's sentence that an unmatched `tool_use_id` means a tool is in
flight is **false on 2.1.233 and nothing replaces it.** The gap §5 claimed to
have narrowed is open. §8's open items are revised accordingly.

**And transport — missing from §8 entirely, which read as complete — is decided
in §7a, by a decision that was taken and then reversed.** `http` was chosen on a
property no other variant has, costed, and the costing showed the property to be
information about the receiver rather than about the subject. The transport is a
file. **Both halves are kept**, because the reversal is the more useful record: a
real and unique property is not the same as a decisive one, and this one did not
survive being priced.

**This round is also the first measured instance of a staleness channel this
project had only predicted.** ADR-0005 §10 rule 4a records that upstream releases
are the one channel with no trigger — no diff on our side, nothing to review. It
happened: a vendor bumped a version and an ADR claim became false with no change
to this repository. Recorded there rather than here, because the next person
relying on a measured property of someone else's tool will be reading the rule.

## Context

The feature is seeing which Claude Code instances are running, what they are
doing, and whether they have stopped. It shares a measurement round with
[ADR-0010](0010-per-project-prompts.md) and none of its decisions.

Versions, because a property belongs to a build:

| | |
| --- | --- |
| Claude Code, round 1 (2026-08-12) | **2.1.228**, native binary bundled in the VSCode extension |
| Claude Code, round 2 (2026-08-17) | **2.1.233**, commit `f8d57569aaf3`, same bundling |
| OS | Windows 10 Pro 19045; node v24.16.0 |
| Windows session | **non-elevated**, which §5's start-time fixture depends on |
| Codex | **not installed — nothing measured**, per ADR-0010 §1 |

**Every claim below carries its round.** Where round 2 contradicts round 1 the
contradiction is recorded as such rather than resolved by overwriting, because
which build a behaviour belongs to is the fact, and a document that silently
adopts the newest answer has thrown away the only thing a version-pinned
measurement was for.

## Decision

### 1. Agents report their own state; nothing is inferred from silence

Taken in principle before the measurement and confirmed by it. State is read
from **hooks**, never from process liveness or transcript mtime.

The argument is constraint 5. Thinking, running a long tool, waiting for
approval, having finished, and having crashed **all look identical from the
outside** — they are all silence. Inferring "stopped" from silence is inventing
a plausible value, and it is the same invention whether the source is a quiet
process or an unchanged file. Hooks replace it with reported facts.

### 2. Hooks fire on Windows, and both settings files load as a union

Measured by installing a node hook that appends its stdin verbatim, then
invoking the agent headlessly.

**Six event types observed in one run (round 1, 2.1.228):** `SessionStart`,
`UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `Stop`, `SessionEnd`.

**`settings.json` and `settings.local.json` both load and both fire — a union,
not an override.** The two events declared in both produced **two entries each**,
distinguished only by a label the hook itself supplied.

That is a design constraint, not a curiosity: **an event can be reported more
than once, and the duplicate is not distinguishable by content.** Anything that
counts events, or that treats an event as a state transition, must deduplicate
on something the payload carries — `tool_use_id` for the tool pair, `prompt_id`
for the turn, `session_id` plus event name otherwise. A design that assumes one
delivery per event is wrong on any machine where both files declare hooks, which
is the ordinary case once a user has local overrides.

**Not observed, and recorded as unobserved rather than absent:** `Notification`
never fired, because no notification occurred — the positive control is that
seven other entries from the same config were written in the same run.
`PermissionRequest` was never exercised at all.

**Round 2 (2.1.233): nine event types, and the union re-established rather than
inherited.** *Added 2026-08-17.* Three runs against a fixture whose two settings
files both declare hooks — so the union is a **constructed** precondition, which
is what §9's dedupe obligation requires and what a fixture with hooks in one file
only would have failed to provide. Every event fired **twice**, once per file,
distinguishable only by a label the hook itself supplied. Round 1's finding holds
on this build.

Two types round 1 does not record: **`MessageDisplay`** and **`PostToolBatch`**.
So the observed set is nine and the earlier six was never a closed enumeration —
it was the list of what one run happened to produce, which is the ordinary rate
at which these tables turn out to be enumerations of what was tried (ADR-0010 §5).

**The build enumerates its own valid event names, and that is a different kind of
fact from an observation.** `claude doctor` reports an unknown hook event by name
and prints the valid set — **31 names on 2.1.233**, including `PermissionRequest`,
`PermissionDenied`, `Notification`, `SubagentStart`, `Elicitation` and
`TeammateIdle`. That enumeration says which names the build *accepts*; it says
nothing about which ones *fire*. The two are kept apart deliberately, because
collapsing them is how a schema becomes a behavioural claim.

**It does close one hole, and that is why it was worth running.** A negative
result about an event that never fired is worthless if the name was never
registered — the failure would be in this project's config and would arrive
labelled as a fact about the subject. Paired both ways: a deliberately bogus name
(`VibeNonexistentEventProbe`) is reported as *"Unknown hook event … was
ignored"*, and a file declaring only the real names produces no such report at
all. So §5's negatives below are about **firing**, not about a typo here.

**The hook *transport* is not one thing on this build, and §7 now depends on
that.** Five variants: `command` (with an `args` exec form that spawns directly,
**no shell in the channel** — used by every fixture in round 2), `prompt`,
`agent`, `http` (POSTs the hook JSON to a URL), and `mcp_tool`. Each carries
`timeout`, and `command` additionally carries `async` / `asyncRewake`.

### 3. Attribution comes from the payload, never from the environment

Every payload carries `session_id`, `transcript_path` (absolute), `cwd`
(absolute), and `hook_event_name`; from `UserPromptSubmit` onward also
`prompt_id` and `permission_mode`. `PreToolUse` adds `tool_name`, the complete
`tool_input`, and `tool_use_id`; `PostToolUse` adds `tool_response` and
`duration_ms`; `Stop` adds `last_assistant_message`, `background_tasks`,
`session_crons` and `stop_hook_active`; `SessionEnd` adds `reason`.

**So attribution to a registered project needs no guessing and no environment
read**: `cwd` is the project directory, and `session_id` plus `transcript_path`
identify the run. **`PreToolUse` does say what the agent is doing** — the tool
name plus its full input, correlatable to its `PostToolUse` by `tool_use_id`.

**The environment is deliberately not used, and that is what dissolves a
caveat.** These payloads were captured from a fixture **nested inside a Claude
Code session**, and the child's environment was measured to be contaminated by
the parent's: `CLAUDE_CODE_EXECPATH` named 2.1.227, the version the outer session
ran, not the 2.1.228 binary under test. Because all three attribution routes are
inside the payload JSON, a design that reads the payload and never the
environment does not inherit that caveat at all.

**`CLAUDE_PID` is the exception, and it was earned rather than assumed.** It is
the agent process's own PID, cross-checked **five times** against the PID the
launcher reported — 320, 9204, 1732, 15152, 12068, each matching its run.
`CLAUDE_PROJECT_DIR` is dropped: never cross-checked, and unnecessary because
`cwd` is in the payload.

**Anything added later that depends on how the agent was launched is still
holding the nested-fixture caveat** and must be labelled as such until
re-established from a plain launch.

### 4. The last event is never terminal, so a third state is mandatory

Measured twice, by killing a running agent and reading what was on disk.

**Killed mid-turn**, with a `UserPromptSubmit` hook still in flight:

```
SessionStart
UserPromptSubmit          <- last written state
```

**Killed mid-tool**, with a `PreToolUse` hook still in flight:

```
SessionStart
UserPromptSubmit
PreToolUse   tool=Read    <- last written state
```

In both cases: **no `Stop`, no `SessionEnd`**, process confirmed gone. For
contrast, a normal completion writes `Stop` and then `SessionEnd` about 150 ms
apart.

The window was made deterministic by a hook that writes its line and then blocks,
because three attempts at a naturally long turn completed too fast — recorded
because it is how the state was produced, and because the mechanism of the block
does not change what was written.

**The consequence is the whole design constraint:** *last reported state* is not
*current state*. An agent whose last event is `PreToolUse` may be running a slow
tool, waiting on approval, or dead, and **the hook record cannot tell those
apart** — the last event of a crashed agent is indistinguishable from the last
event of a busy one. Rendering the last event as the current state re-invents
exactly the value hooks were adopted to stop inventing; it is constraint 5
arriving through the mechanism chosen to avoid it, which is why it is stated here
rather than left to the display.

**So a third state is not a nicety, it is required by the measurement**, and it
must be distinguishable from both "working" and "stopped".

**Round 2 leaves this argument untouched and makes it stronger, and the
distinction matters because §5 below changes a great deal.** *Added 2026-08-17.*
What §5 retracts is a claim about what an unmatched `tool_use_id` *means*; it
does not touch the reason a third state exists, which is that the last written
event is not the current state. **The third state's composition is unchanged.
What changes is how much lives inside it** — round 1 read "alive and quiet" as
one case plus a hedge, and it is three cases with no discriminator. A third state
is required by the same measurement and now has more to hold.

### 5. Liveness is the only measured discriminator, and it is not sufficient alone

`CLAUDE_PID` gives a check the hook record cannot: whether the process that
reported the last event still exists. That is the only measured way to separate
*busy* from *gone*.

**It does not restore certainty, and reading it as though it did would repeat the
error one level down.** A live PID says a process exists, not that the agent is
healthy or making progress; and PIDs are reused by the operating system, so a
live PID is not proof it is *the same* process.

**Pairing the PID with `session_id` does not fix that, and an earlier draft of
this section said it did.** `session_id` comes from the payload — it is static
data written to a file by a process that may no longer exist. It identifies the
*run*; it says nothing about the process currently holding that PID. On reuse,
"PID 320 is alive" is a true statement about something else entirely, and no
amount of payload data touches it. **So "process alive" is itself a potentially
invented value, sitting inside the fact that is supposed to keep the rest
honest.**

**The fix is composite identity — `(pid, start_time)` — where a mismatched start
time means the PID was recycled.** Its cost is genuinely cross-platform work and
is stated rather than waved at:

- **Linux:** field 22 of `/proc/<pid>/stat`. No dependency.
- **macOS:** `sysctl` `KERN_PROC_PID` → `kinfo_proc`. `libc` plus `unsafe`.
- **Windows:** `GetProcessTimes`. Measured available on this machine at 100 ns
  resolution and stable across reads — three identical reads of one process —
  and **not universally readable**, because the datum is access-controlled.
  *"Start time unavailable"* is therefore a third outcome and must not collapse
  into either alive or gone. That is the same `NotAttempted` line one level down.

  **The observable was wrong, and it was the instrument's.** *Corrected
  2026-08-17.* This bullet read that *"one process in a four-row sample returned
  an empty start time"*, and an empty start time is not what the syscall does.
  Re-measured through raw `OpenProcess` + `GetProcessTimes` with a positive
  control on our own process:

  | PID | class | outcome | Win32 |
  | --- | --- | --- | --- |
  | own | **positive control** | OK, 100 ns FILETIME | 0 |
  | a user-launched `node` | **the subject class** | OK | 0 |
  | 4 | `System`, protected | `OpenProcess` returns NULL | **5** `ACCESS_DENIED` |
  | 0 | `Idle` | `OpenProcess` returns NULL | 87 |
  | 999999 | nonexistent | `OpenProcess` returns NULL | **87** |

  The empty string came from **.NET's `Process.StartTime` accessor**, which on a
  protected process yields an empty value **and raises nothing** — a blank cell
  in a table, no error to notice. The syscall is loud. Suspicion goes to the
  instrument first, and it was the instrument again; ADR-0002 §7 carries the
  general form and explicitly does not fold this into the six-for-six tally,
  which is a different population.

  **So the arm is reachable, and PID 4 is a real fixture — with the claim it
  supports stated narrowly.** It exercises the real syscall path to a real
  unavailable outcome, so no synthesised value is needed on Windows. It does
  **not** establish that the subject can ever be in that state: the subject is
  always a user-launched `claude`/`node` process, measured `OK`. The fixture also
  depends on two facts about the environment rather than about the code — that
  PID 4 is the protected `System` process on build 19045, and that the session is
  **non-elevated**. An elevated run holding `SeDebugPrivilege` may not reproduce
  it; that was not tested. **Both belong in a comment beside the fixture**, or
  the day it stops failing nobody will know why.

  **And the real hazard is one this bullet did not have: `OpenProcess` returning
  NULL is two outcomes wearing one observable.** `ACCESS_DENIED` means *the
  process exists and cannot be read*; `ERROR_INVALID_PARAMETER` means *there is
  no such process*. Code that branches on the null handle collapses them, and it
  collapses them in the direction that reads as reassurance: **a live agent whose
  start time is unreadable renders as stopped.** That is ADR-0010 §5's
  *"nonzero means not-ignored"* one platform down — an error read as a clean
  answer — and §9 carries it as a requirement on the diff with its paired
  control. Branch on `GetLastError`, never on the handle.

- **Linux and macOS: unmeasured, and the asymmetry is therefore unknown.**
  Round 2 ran on Windows only. Whether either can produce a real *unavailable*
  through its own route — `/proc/<pid>/stat` field 22, `KERN_PROC_PID` — is not
  established, and if they cannot, the platform limit inverts ADR-0010 §10's,
  where Unix had the reachable fixture and Windows had the synthesised one. That
  inversion is worth recording **if it is measured**; it is not inferred here.
- **Or one crate covering all three**, which is a dependency added to a library
  every embedder links — the cost ADR-0008 §4 weighed when it declined 18 crates
  for a TLS stack. Not weighed here, because that is a decision for the diff.

**Without something of that shape, liveness is an inference wearing the costume
of a fact**, and it belongs in the same category as the silence this whole design
refuses to read.

**How much is actually unresolved while a process is alive and quiet — the
narrowing is RETRACTED, and nothing replaces it.** *Amended 2026-08-17, from the
2.1.233 round. The retracted sentence is quoted rather than deleted, because a
retraction a reader cannot see is one they will re-derive.*

It read: *"an unmatched `tool_use_id` with the reporter alive is a tool in flight
— a reported fact, not an inference from silence, and it is not thinking."* It
also conceded one open distinction, conditional on something recorded as
unmeasured: whether approval is requested inside the tool window.

**Both halves were measured. The condition resolves to the bad branch, and the
sentence is false for a reason that was not on the list at all.**

Three fixtures, each with its precondition constructed rather than hoped for —
round 1's *"headless auto-denies"* did not reproduce, because the child inherited
a permission state that simply allowed the tool and nothing ever asked. That run
proved nothing and is recorded as the unreached guard it was. `permissions.ask`
in the fixture's own settings is what produced a real decision.

1. **Approval sits INSIDE the window.** A blocking MCP `--permission-prompt-tool`
   held a real approval for **9 s**: `PreToolUse` opened `toolu_01JtAH…` at
   3387 ms, the prompt was asked at ~3400 ms and answered at ~12400 ms, and
   `PostToolUse` closed the same id at 13548 ms. Two snapshots taken *inside* the
   wait each report exactly one open `tool_use_id` — the one being waited on —
   and no other events. **In the hook record a 9 s approval wait is byte-identical
   to a 9 s slow tool.**

2. **A denied tool leaves its `tool_use_id` open permanently.** Same fixture, the
   decision refused: `PreToolUse` opened `toolu_01KJH…` at 3046 ms, **no
   `PostToolUse` ever**, `PostToolBatch` at 3143 ms, then `Stop` at 8170 ms and
   `SessionEnd` at 8305 ms. The subject's own report confirms a real denial
   (`permission_denials` non-empty, `is_error=true`). **The id is not open until
   it finishes. It is open by definition, in a session that has stopped.**

3. **`PermissionRequest`, `PermissionDenied` and `Notification` did not fire** in
   any of it, with the names demonstrably registered (§2) and the mechanism
   demonstrably run. Positive control: 16 and 18 hook entries from the same
   config in the same runs, including `PreToolUse` for the very tool whose
   permission was prompted.

**The retraction is total rather than partial, and that is a decision rather than
an oversight.** The narrowed form available here is *"either a tool is executing,
or the agent is waiting for approval, or a tool was denied and nothing is
running"* — a three-way disjunction over states with no common consequence.
**That is not a reported fact; it is an absence of information with a list
inside it**, and writing it as a narrowing would leave this section claiming to
have bounded something it has not. So: **an unmatched `tool_use_id` with the
reporter alive supports nothing about the present.** The gap §5 opened with is
open, and it is wider than when it was written.

**Case 2 is the worst of the three, and the general shape it exposes is worth
more than the case.** §7 records that absence of events does not prove absence of
work — an empty monitor looks like a quiet machine. Case 2 is the converse:
**presence of an unclosed event does not prove presence of work.** An
unmatched id survives `Stop` and `SessionEnd`, so it is not even evidence the
session is running.

**The channel is ambiguous in both directions, not one.** That is the sentence to
carry forward, because every mechanism in this document so far was built against
the silent direction — the wiring proof, the liveness check, the third state —
and a design that guards only silence is guarding half of the problem. Silence
overstates stoppage; an open event overstates activity. Constraint 5 has two
faces here and they invent in opposite directions.

**`PostToolBatch` is a candidate closer, filed with its bound and not as a
replacement.** It fired in both runs — 97 ms after the denial that produced no
`PostToolUse`, and 113 ms after the `PostToolUse` that did — so it closed the
tool phase in the case where `PostToolUse` never came. **Two observations, both
single-tool batches, semantics under parallel or nested tool calls unmeasured.**
It does not rescue the retracted sentence and must not be written up as though it
did; what it is, is the first thing to measure when this is picked up.

**Where the negatives in point 3 stop.** They are headless, through the MCP
permission route. Whether the **interactive TUI** path fires them is unmeasured:
no pty is constructible in this environment, which is the identical one-sided
limit ADR-0005 §10 records for the pager, from the same cause. So §8's first item
is answered on one path and open on the other, and the honest form of the result
is *"not on this path"* rather than *"not used"*.

### 6. What must be true of the display, whatever it looks like

- **The three-way distinction survives into the UI.** ADR-0009 §3c already
  requires that a fact we did not establish renders differently from a fact we
  established as absent. Here that is: *reported working and reporter alive*,
  *reported working and reporter gone*, and *reported stopped* — with the middle
  one never borrowing the appearance of either neighbour.
- **A timestamp is part of the fact, not decoration.** A reported state is a
  fact *as of* an instant, and one without its age is a claim about now.
- **Absence of events is not a state.** An agent that never ran and an agent
  whose hooks were never installed both produce an empty record, and neither is
  "stopped". This is the same `NotAttempted`-versus-`NoEvidence` line ADR-0003
  draws and ADR-0010 §6 applies to shadowing.

### 7. Vibe may write hook config, as one explicit act — and the wiring carries its own proof

**Decided.** An earlier draft left this open on the ground that hooks create a
consistency relationship between two artifacts, which is the shape ADR-0010 §3
rejected for prompts, and doubted whether the reasoning transferred *because the
second artifact is configuration rather than content*. **That was the wrong
distinction.** ADR-0010 §3 did not reject writing; it rejected **sync** — a
standing obligation to keep two things equal, discharged silently and repeatedly.

So the permitted shape is the one ADR-0010 §3 already permitted itself: **an explicit
`vibe monitor install` — singular, user-initiated, never automatic.** What must
not exist is silent repair of drift.

**And drift is not answered by syncing; it is answered by versioning the
contract.** The installed hook declares which contract version it implements,
vibe reads that declaration, and **a mismatch is reported, never repaired.** A
repair would be sync under another name, and it would do it at the moment the
user is least able to see it.

**The larger hazard is not the write, and it outranks it: silent
non-delivery.** If the hook is missing, misconfigured, broken, or removed, vibe
receives nothing — and **"no events" is indistinguishable from "the agent is not
running"**. That is constraint 5 at the centre of this feature, in its worst
form, because the default reading is the reassuring one: an empty monitor looks
like a quiet machine.

**Therefore the governing constraint is not about writing at all: the wiring must
carry its own proof, per session.** A session that has delivered a `SessionStart`
has demonstrated its wiring **live, for that session** — the hook ran, the
transport worked, the payload arrived. A session with no such marker is
**unknown**, never idle, and no amount of subsequent silence upgrades it.

This is `VIBE_REQUIRE_GH`'s shape (ADR-0002 §7) applied to the channel rather
than to a control: the *result* carries the evidence that the mechanism ran, so
nothing has to be remembered or checked on the side.

**The proof is stronger than `SessionStart`, and the strengthening does not help
where it matters. Both halves are recorded, because either alone misleads.**

- **Stronger:** *every* delivered event proves the channel live **at its own
  timestamp**, not merely at session start. A hook removed mid-session stops
  refreshing the proof, so the uncovered window is **"since the last received
  event"** rather than "since session start" — a much smaller and
  self-narrowing hole, which any traffic at all closes again.
- **And useless exactly where it is needed:** that window is precisely the
  *silent* one, which is when the question is being asked. Traffic refreshes the
  proof; **absence of traffic is simultaneously the question and the hole.** An
  agent that has said nothing for an hour is the case a monitor exists for, and
  it is the one case where the freshness of the wiring proof is at its weakest.

So the limit stands in its sharpened form: **a delivered event proves the wiring
worked when it was delivered, and nothing about now.** That is the
environment-shaped hole closed and the code-shaped one left open, exactly as
ADR-0002 §7 records for the original — and it is why §5's liveness check is not
redundant with this one. They fail in different directions, which is the only
reason to have both.

It follows that **monitoring is opt-in per project** — hooks fire only where
installed — and that an uninstrumented project renders as unknown rather than
idle, per §6.

### 7a. Transport: a file — and `http` chosen, costed, and reversed

*Added 2026-08-17. This was missing from §8's list of open decisions, which read
as complete and was not.* The omission had a reason and the reason was wrong:
transport looked like a decision for the diff, by analogy with §5's
cross-platform crate. **It is not analogous.** A crate choice is a dependency;
transport decides **whether non-delivery is detectable at all**, and
non-delivery is §7's central hazard. A file append and a socket fail
differently. That is §7's own question, not an adjacent one.

**Decided: a file, appended to by a `command` hook in the `args` exec form.**
`prompt` and `agent` cost a model turn and are refused on the ground ADR-0010 §6
refuses enrichment as a base layer. `mcp_tool` needs a server per session and
reports failure to the agent rather than to us. `http` was **chosen first, and
the choice did not survive being costed** — the reversal is recorded below in
full, because it is more useful than the conclusion.

#### Why `http` was chosen, and why that property did not hold up

The argument was real and no other variant has it: **`http` is the only variant
where the receiving side holds independent evidence of its own liveness**, so
*"no events"* splits into *"the receiver was up and heard nothing"* and *"the
receiver was down"*. A file cannot make that split — a file that stops growing is
indistinguishable from a quiet agent, which is §7's central hazard exactly.

Costing it produced a narrowing, and following the narrowing reversed it.

**1. The narrowing.** The receiver's evidence covers only the interval it has
been **continuously up**. Across a restart it cannot separate *"nothing happened
while I was down"* from *"things happened while I was down"*, because **a process
that is down cannot record its own downtime.** The repair is for the receiver to
record its own coverage intervals durably, so *"I have no coverage for this
window"* is a reported fact — §7's proof-carrying shape applied to the receiver.
That needs durable state regardless, so `http` does not replace the file; it puts
a listener in front of one.

**2. Give both transports that coverage log, and the residual is identical.**
Nothing arrived because the agent was quiet, or because the hook is broken.
Neither transport touches that, and **the per-session wiring proof of §7 handles
it identically in both.** So the coverage log equalises them on the only question
either was being chosen to answer.

**3. And then the asymmetry runs the other way.** *"Receiver down"* is a failure
mode **that exists only because there is a receiver.** A file has no up or down
state, so the question never arises. During receiver downtime the hook still
appends to a file; with `http` those events are **lost**. Sleep, reboot and crash
are precisely when vibe is not running — so `http` loses events exactly on the
**long-silence case the monitor exists for**, which is the case §7 already
identifies as the one where every other proof here is weakest.

**4. So the deciding property was information about the receiver, not about the
subject.** Its only use is accounting for a loss the file transport does not
incur. **Information whose sole purpose is explaining your own failure mode is
not a reason to adopt the failure mode**, and that sentence is the transferable
part of this reversal.

**The reversal was falsification-tested rather than argued to a stop**, because
the property being discarded was genuine. The question put was: *name a case
where `http`'s receiver holds information a file cannot, once both have a
coverage log.* Seven candidates were tried and each is symmetric:

| Candidate | Why it is not an asymmetry |
| --- | --- |
| sink unwritable | file-unwritable ↔ receiver-unreachable; both lose silently |
| arrival timestamp | the hook stamps its own time in both, and it is one machine, so no skew |
| synchronous acknowledgment | informs the *agent*, not vibe; `asyncRewake` and exit `2` are open to a `command` hook too |
| session discovery | the same set, learned at read time rather than at arrival |
| malformed payload | lands in the file exactly as it lands at the receiver |
| record deleted or truncated | the receiver's coverage log is itself a file with the same exposure |
| real-time push | latency, not information; a watcher exists on all three platforms |

**None survived, and the failed falsification is what licenses the reversal.**
Had one survived, the decision would have stood. Recorded as a list rather than
as a conclusion so the next reader can attack the same question with a candidate
nobody here thought of, which is the only way this gets overturned again.

**What `http` would still have cost, kept because the costs are what made the
narrowing worth chasing:** vibe grows a long-running mode it has never had —
every command today is one-shot; an inbound socket is a channel *added* to an
environment, which is the opposite of the move ADR-0005 §10 and ADR-0008 §4 are
built on, and a local port accepting hook payloads is one **anything on the
machine can post false events to**, an integrity problem for a tool whose whole
product is reported facts; and `headers` plus `allowedEnvVars` would make it a
secret-management problem in a tool that deliberately has none (ADR-0008 §5).
Those costs did not decide it. **The reversal is on the deciding property
failing, not on the costs outweighing it** — which matters, because a decision
reversed on cost is re-opened by cheaper hardware and this one is not.

#### What the file transport is therefore required to carry

These are **parts of the design, not cost lines**, and they are written as
requirements because the reversal moved them from *"a price `http` avoids"* to
*"work this transport must do"*.

- **One file per writer — `(session_id, settings source)` — so concurrent append
  does not exist.** *Decided 2026-08-17.* Multiple sessions, and the duplicate
  delivery §2 measures, would otherwise write one sink at once.

  **Two shapes were rejected and the first was inadmissible rather than worse.**
  A shared file relying on append atomicity has a failure — a split or
  interleaved record — that **cannot be induced on demand**: it depends on
  filesystem, OS and timing, and may never reproduce on local NTFS or ext4. A
  failure that cannot be produced deliberately **cannot have a paired control**,
  and a guard without one has never been accepted here, so it is out before cost
  is discussed. A shared file with framed records is admissible — framing makes a
  torn record certainly detectable rather than heuristically so, and its fixture
  is fully constructible because the hazard moves into the *reader* — but it
  **detects rather than prevents**, and the torn record's contents are still
  lost.

  **What decides it is neither of those.** Both shared-file shapes rest on an
  atomicity guarantee **this project cannot measure on two of its three
  platforms** from the machine it develops on. That is the same shape as the
  `PIPE_BUF` assumption that died on contact with measurement (ADR-0002 §7): a
  cross-platform property taken on authority. **One writer per file does not
  handle that dependency, it removes it** — the guarantee stops being load-bearing
  because nothing concurrent happens. Same technique as the missing
  `FileOp::Delete` and ADR-0005 §10 rule 1's closed enum.

  **The positional bound is the bonus, not the reason.** With one writer,
  interleaving is impossible and only truncation remains — a crashed hook, a full
  disk, a killed process — so **only the last record in a file can be partial.**
  The reader therefore validates a *tail* rather than scanning for corruption
  anywhere, which is the whole of the framing that survives from the rejected
  shape.

- **Ordering across files is authored, not observed, and the reader must be able
  to refuse.** *This is the largest cost D carries and it is recorded before it
  becomes an implementation detail.*

  **Measured 2026-08-17: no event type carries a timestamp.** Across all eight
  observed types the payload offers `session_id`, `prompt_id`, `turn_id`,
  `message_id`, `tool_use_id`, `index` and `duration_ms` — and nothing naming a
  point in time. Every timestamp in this round's data was added by the
  measurement harness. So under D, where history is reassembled from several
  files, **the merge key is a value the writer invents.**

  That is constraint 5 pointed at *ordering* rather than at a value, and it is
  the more dangerous target: a wrong value is a wrong field, while a wrong order
  is a **plausible history**. Clock skew between hook processes, or a wall clock
  stepping backwards under NTP, silently reorders events into a sequence that
  reads perfectly.

  **What the payload does support, at no cost and with no clock, is a partial
  order**: `session_id` groups, `prompt_id` groups a turn, `tool_use_id` pairs a
  `PreToolUse` with its `PostToolUse`, `index` sequences `MessageDisplay` deltas
  within a message, and the lifecycle constrains the ends. **That is a payload
  fact and it is the primary ordering.** Authored stamps are the fallback, used
  only where the payload orders nothing.

  **So the reader's contract is a partial order, not a sequence** — and where two
  records are unordered by both the payload and the stamps, it must **say so
  rather than present one**. That cost propagates: it reaches §6's display and
  ADR-0009's constraints as a third state one level down, *ordered* versus
  *unordered with respect to each other*, with the second never borrowing the
  appearance of the first.

  **One check D makes available and a shared file would muddy:** within a single
  file there is exactly one writer, so stamps must be non-decreasing. **A
  decreasing stamp inside one file is direct evidence the clock stepped** — a
  reported fact rather than an inference, detectable with no cross-file
  reasoning at all.

  **Deliberately not taken yet: a monotonic stamp.** A boot-relative monotonic
  clock does not step backwards and is understood to be comparable across
  processes on one boot, which would give a total order within a session — and
  *"understood to be"* is exactly the standing this section is not going to build
  on again. It is **unmeasured on all three platforms**, and the last
  cross-platform property accepted on that standing was `PIPE_BUF`. It becomes
  available when measured, and not before.
- **The contract version pins the hook's execution properties, not just its
  payload shape.** §7 requires the installed hook to declare which contract
  version it implements. Hooks carry `timeout`, `async` and `asyncRewake`
  (§2), and each changes *whether and when* a record arrives — an `async` hook
  that is killed at session end delivers nothing, and a `timeout` that fires
  truncates a record. **A contract that pins only the payload leaves the
  delivery semantics unpinned**, which is the half that decides whether absence
  means anything. `http` would have inherited this identically; the reversal
  does not reduce it.
- **Writer identity is declared by the hook, so `unattributed` is a state rather
  than an error.** *Measured: the payload does not name the settings source it
  was delivered through* — §2's two deliveries are distinguishable only by
  something the hook itself supplies. Under D that identity is half the filename,
  so it moves into §7's contract declaration alongside the version.

  **A hand-installed hook that omits it writes a file the reader cannot
  attribute**, and this is the ordinary case rather than the exotic one: §7
  permits hooks installed by hand, so the field will be missing somewhere. The
  reader therefore **lists such a file as `unattributed` and reads its records**
  — the events are real, the session is named in every payload, and only the
  *source* is unknown. Discarding it would lose real events; guessing a source
  would invent one; calling it an error would say something failed when nothing
  did. It is `NotAttempted`'s neighbour: a fact vibe does not have, named as
  missing, with everything that does not depend on it still usable.

  **The one thing it costs is dedupe.** §2's duplicate delivery is identified by
  source, so an unattributed file's records cannot be matched against their twins
  with certainty. That is a bounded, statable degradation — *this session's
  duplicates may not be collapsed* — and it must render as such rather than as a
  session that emitted twice as many events.

- **The sink lives where vibe manages it**, not in a directory a user cleans up,
  since an append into a deleted inode succeeds silently on POSIX. One file per
  writer makes retention a **new and required** piece of work, and it is a
  deletion story, which this project treats carefully.

**And the residual is stated rather than dissolved:** a file that stops growing
is indistinguishable from a quiet agent. Neither transport ever fixed that. It is
§7's hazard, it is answered by the per-session wiring proof and by §5's liveness
check failing in different directions, and **the transport decision was never
capable of touching it** — which is the thing the first version of this section
got wrong.

### 8. Open, and not mine to decide

*Revised 2026-08-17. The previous list had two items and read as complete; it was
missing transport, which is now §7a. What follows is the state after round 2.*

1. **~~Whether `PermissionRequest` and `Notification` are used~~ — answered on one
   path, open on the other.** They do **not** fire headlessly through the MCP
   permission route, with the names registered and the mechanism demonstrably run
   (§5). Whether the interactive TUI path fires them is **unmeasured and not
   measurable here**, for want of a constructible pty. What closes it is a fixture
   with a real terminal, not another headless run.
2. **The display shape and the state names.** Unchanged as a question and
   **harder as a problem**, now for two reasons: §5's retraction means the facts
   available are fewer than round 1 recorded, so a display cannot lean on *"a
   tool is running"*; and §7a's transport makes history a **partial order**, so
   the display inherits an *unordered with respect to each other* state it cannot
   render as a sequence. §6 constrains this and does not settle it.
3. **~~Transport~~ — decided: a file, §7a.** `http` was chosen first on a real
   and unique property and **reversed** when costing showed that property to be
   information about the receiver rather than about the subject. The reversal is
   recorded in full, with the falsification list that licenses it, because the
   next reader overturns it by finding a candidate that list missed.
4. **New: whether `PostToolBatch` is the closer `PostToolUse` is not.** Two
   observations, single-tool batches only; parallel and nested semantics
   unmeasured (§5). This is the first thing to measure when the feature is picked
   up, and it is blocked on measurement rather than on preference.

### 9. Negative-control obligations for whatever gets built

- **The third state needs a paired control**, per ADR-0002 §7's rule against
  one-sided ones: an agent killed mid-tool must render as the third state, **and
  a live agent with the same last event must render as working**. A build that
  reports the third state unconditionally satisfies the first half perfectly.
- **The dedupe needs a control that both files are declaring hooks**, or it tests
  nothing — the union in §2 is the precondition, and a fixture with hooks in one
  file only would pass while the defect ships.
- **The wiring proof needs its own paired control**, because it is the one thing
  everything else rests on: a session with hooks installed must render as
  *observed*, **and the same session with the hook config removed must render as
  unknown rather than idle**. A build that renders unknown only when it has no
  events at all satisfies neither half meaningfully, and the failure is silent by
  construction.
- **Any further measurement of Claude Code needs a channel control first** —
  ADR-0002 §7's channel rule, whose base rate came from this feature's own
  measurement round, where six of six discrepancies were the instrument.
  **Round 2 honoured this**: `probe.js` was rebuilt with its known line-by-line
  blindness fixed and demonstrated in both directions — whole-text matching finds
  a multi-line target, the line-by-line sabotage does not, and the control is
  identical in both, which is exactly why the original blindness survived — plus
  a dead-control run proving a zero cannot be reported without a live control.

- **The `(pid, start_time)` liveness check needs a paired control on the
  `OpenProcess` failure, not on the handle.** *Added 2026-08-17.* NULL is two
  outcomes sharing one observable (§5), so: **PID 4 must render as
  *unavailable*, and a nonexistent PID must render as *gone*, and the two must
  render differently.** A build that maps every NULL to *gone* satisfies the
  second half perfectly while reporting a live agent as stopped. The fixture's
  dependence on running **non-elevated**, and on PID 4 being the protected
  `System` process, belongs in a comment beside it — those are facts about the
  environment, and when one changes the test stops testing without going red.

- **FORBIDDEN: do not write a control asserting that an open `tool_use_id`
  renders as working.** *Added 2026-08-17.* It is the control §5's retracted
  sentence would have justified, it will pass, and **passing is the defect** — an
  open id also belongs to a denied tool in a session that has stopped (§5). A
  build that renders an open id as *working* is wrong on 2.1.233, and this
  control would certify it as right.

  **The general rule is in ADR-0002 §7, not here** — *a retraction removes a
  control's subject, and nothing inherits the removal; record what must not be
  built beside what must.* It is filed there for the reason this project has now
  paid for twice: the next person writing a control will be reading the rules,
  and will not open the monitoring ADR.

- **The one-writer-per-file property needs a control, and "several simultaneous
  writers" is not it.** *Rewritten 2026-08-17, once §7a settled on D. The first
  version of this bullet asked for simultaneous writers appending to one sink and
  per-record parseability, and it would have **passed while proving nothing**: the
  failure it aimed at is size-dependent, not concurrency-dependent, and small
  records from several writers reproduce nothing.*

  Two controls, because the design has two claims:

  **a. Distinct paths, paired.** Two sessions, and §2's two settings sources
  within one session, must produce **four distinct files**. Sabotage by
  collapsing the naming to a constant and observing the collision — which is what
  makes the assertion about the *naming* rather than about a fixture that happened
  to use one session. Without the sabotage half, a build that writes one file per
  *machine* satisfies the "records are all present" reading perfectly.

  **b. A cut-mid-record tail.** Write a file whose final record is truncated
  part-way and assert the reader **reports a partial tail** — not dropping it
  silently, and not parsing a prefix that happens to be valid. Sabotage by
  deleting the tail check and observing the truncated record either vanish or be
  accepted as complete. This is constructible by hand precisely because D bounds
  the hazard positionally; it needs no race, which is the property that made the
  rejected shared-file shape inadmissible.

  **Note what is deliberately not controlled: append atomicity.** Under D nothing
  concurrent happens, so there is no atomicity property to test. A control
  asserting it would be testing an OS guarantee this design no longer depends on
  — and it would go green on the one platform it could run on, which is how the
  dependency got accepted in the first place.

- **Ordering needs a control that the reader refuses rather than guesses.** Two
  records with no ordering relation between them in the payload — different turns,
  no shared `tool_use_id`, no `index` — and stamps that are equal or inverted must
  render as **unordered**, not as a sequence. Paired: the same two records with a
  payload relation present must render **ordered**. A build that always emits a
  sequence satisfies the second half perfectly, and the failure is a plausible
  history, which is the one nobody investigates.

- **The contract version needs a control on the delivery properties, not only on
  the payload.** `timeout`, `async` and `asyncRewake` each change whether and
  when a record arrives (§2, §7a). A control that pins only the payload shape
  passes against a hook whose `async` variant delivers nothing at session end,
  which is silent non-delivery arriving through the mechanism installed to
  prevent it.

## Consequences

**Easier:** attribution is solved and needs nothing invented — three independent
routes, all inside the payload. The nested-fixture caveat costs nothing because
the design does not read the environment.

**Harder:** the honest answer is two facts rather than one, and the interesting
distinctions inside "alive and quiet" are not available from anything measured
here. A monitor that says less than a user wants is the price of not inventing
the rest.

**Harder again, after round 2, and this is a widening rather than a detail.**
*Added 2026-08-17.* "Alive and quiet" holds **three** cases with no
discriminator between them — executing, waiting for approval, and finished after
a denial — where round 1 recorded one plus a hedge. And the ambiguity runs in
both directions: silence overstates stoppage, an unclosed event overstates
activity. **Everything built here so far guards the silent direction only.** The
product consequence is that a monitor built on this record can report *that* an
agent reported something and *when*, and cannot report what it is doing now.

**Trade-off accepted:** hooks require installation, so coverage is opt-in and
partial by construction. The alternative — inferring state from liveness or file
timestamps — covers everything and is wrong in a way the user cannot see, which
is the trade this project has consistently refused.

**Not decided here, deliberately:** the open questions in §8, revised after round
2. Two are blocked on measurement that cannot be taken from this environment or
has not been taken — the interactive-TUI path, and `PostToolBatch`'s semantics
under parallel tool calls — and one is display design, which §6 constrains and
does not settle. **Transport is no longer among them**, and its absence from the
earlier list is recorded in §7a rather than quietly repaired, because a list of
open questions that reads as complete and is not is the same failure this
document is about.
