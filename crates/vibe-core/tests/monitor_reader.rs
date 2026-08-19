//! Controls for the sink reader — ADR-0011 §7a's ordering contract.
//!
//! The governing requirement: **the reader's contract is a partial order, not a
//! sequence** — and where two records are unordered by both the payload and the
//! stamps it must *say so rather than present one*. §9 requires the control to
//! be paired, because *a build that always emits a sequence satisfies the second
//! half perfectly, and the failure is a plausible history, which is the one
//! nobody investigates.*
//!
//! # FORBIDDEN here too
//!
//! **Do not write a control asserting that an open `tool_use_id` means work is
//! in flight.** §5's retraction is total, and the subagent round added a fourth
//! member to the disjunction: a parent's `Agent` tool_use_id stays open for the
//! entire subagent lifetime — 21 seconds in one measured run. An open id means
//! *executing*, *waiting for approval*, *finished after a denial*, or *a
//! subagent is running*, with no common consequence.

use std::fs;
use std::path::Path;

use vibe_core::monitor::{
    Attribution, OrderBasis, RecordOrder, Sequencing, TailState, order, read_sink,
};

/// Build a sink file by hand. The reader is the subject, so the writer is not
/// in the loop — a fixture that went through the writer could only ever produce
/// records the writer happens to emit.
fn plant(dir: &Path, name: &str, records: &[&str]) {
    let mut body = String::new();
    for r in records {
        body.push_str(r);
        body.push('\n');
    }
    fs::write(dir.join(name), body).expect("plant");
}

/// One record line, in the writer's shape: hoisted fields plus the payload
/// verbatim as a JSON string.
fn rec(session: &str, agent: Option<&str>, event: &str, stamp: &str, extra: &str) -> String {
    let payload = format!(
        r#"{{"session_id":"{session}","hook_event_name":"{event}"{}{extra}}}"#,
        agent.map_or(String::new(), |a| format!(r#","agent_id":"{a}""#))
    );
    let escaped = serde_json::to_string(&payload).expect("escape");
    let agent_field = agent.map_or(String::new(), |a| format!(r#","agent":"{a}""#));
    format!(
        r#"{{"v":"1","identity":"alpha","session":"{session}"{agent_field},"stamp_ns":"{stamp}","payload":{escaped}}}"#
    )
}

// ---------------------------------------------------------------------------
// The ordering contract, paired
// ---------------------------------------------------------------------------

/// **Paired half one: a payload relation present must render ORDERED.**
#[test]
fn a_tool_pair_and_the_lifecycle_are_ordered_by_the_payload() {
    let dir = tempfile::tempdir().expect("tempdir");
    plant(
        dir.path(),
        "sess__alpha.jsonl",
        &[
            &rec("sess", None, "SessionStart", "1000", ""),
            &rec(
                "sess",
                None,
                "PreToolUse",
                "2000",
                r#","tool_use_id":"toolu_1""#,
            ),
            &rec(
                "sess",
                None,
                "PostToolUse",
                "3000",
                r#","tool_use_id":"toolu_1""#,
            ),
            &rec("sess", None, "SessionEnd", "4000", ""),
        ],
    );
    let (listing, unreadable) = read_sink(dir.path()).expect("sink readable");
    assert!(unreadable.is_empty());
    let e = &listing.files[0].entries;
    assert_eq!(e.len(), 4);

    assert_eq!(
        order(&e[1], &e[2]),
        RecordOrder::Before {
            basis: OrderBasis::ToolPair
        },
        "PreToolUse precedes its own PostToolUse"
    );
    assert_eq!(
        order(&e[2], &e[1]),
        RecordOrder::After {
            basis: OrderBasis::ToolPair
        },
        "and the relation is antisymmetric"
    );
    assert_eq!(
        order(&e[0], &e[2]),
        RecordOrder::Before {
            basis: OrderBasis::Lifecycle
        }
    );
    assert_eq!(
        order(&e[1], &e[3]),
        RecordOrder::Before {
            basis: OrderBasis::Lifecycle
        }
    );
}

/// **Paired half two: no payload relation must render UNORDERED, even when the
/// stamps look decisive.**
///
/// This is the half a build that always emits a sequence fails. The two records
/// are in different turns, share no `tool_use_id`, and their stamps are 1000
/// apart — every temptation to sort them is present and none of it is evidence.
#[test]
fn two_records_with_no_payload_relation_are_unordered_however_the_stamps_read() {
    let dir = tempfile::tempdir().expect("tempdir");
    plant(
        dir.path(),
        "sess__alpha.jsonl",
        &[
            &rec(
                "sess",
                None,
                "PreToolUse",
                "5000",
                r#","tool_use_id":"toolu_a","prompt_id":"p1""#,
            ),
            &rec(
                "sess",
                None,
                "PreToolUse",
                "6000",
                r#","tool_use_id":"toolu_b","prompt_id":"p2""#,
            ),
        ],
    );
    let (listing, _) = read_sink(dir.path()).expect("readable");
    let e = &listing.files[0].entries;

    assert_eq!(
        order(&e[0], &e[1]),
        RecordOrder::Unordered,
        "ascending stamps are not a payload relation, and presenting them as \
         one is a plausible history — the failure nobody investigates"
    );
    // Inverted stamps must not flip it either: the answer does not depend on
    // the stamps at all.
    assert_eq!(order(&e[1], &e[0]), RecordOrder::Unordered);
}

/// Records from different sessions are never comparable, whatever else matches.
#[test]
fn records_from_different_sessions_are_unordered() {
    let dir = tempfile::tempdir().expect("tempdir");
    plant(
        dir.path(),
        "one__alpha.jsonl",
        &[&rec("one", None, "SessionStart", "1000", "")],
    );
    plant(
        dir.path(),
        "two__alpha.jsonl",
        &[&rec("two", None, "SessionEnd", "2000", "")],
    );
    let (listing, _) = read_sink(dir.path()).expect("readable");
    let a = &listing.files[0].entries[0];
    let b = &listing.files[1].entries[0];
    assert_eq!(
        order(a, b),
        RecordOrder::Unordered,
        "a SessionStart in one session says nothing about a SessionEnd in another"
    );
}

/// A parent and a subagent in the SAME session are ordered only where the
/// payload orders them — sharing a session is grouping, not sequencing.
#[test]
fn a_subagent_and_its_parent_are_grouped_but_not_thereby_ordered() {
    let dir = tempfile::tempdir().expect("tempdir");
    plant(
        dir.path(),
        "sess__alpha.jsonl",
        &[&rec(
            "sess",
            None,
            "PreToolUse",
            "1000",
            r#","tool_use_id":"toolu_parent""#,
        )],
    );
    plant(
        dir.path(),
        "sess__ab8b50189992e6091__alpha.jsonl",
        &[&rec(
            "sess",
            Some("ab8b50189992e6091"),
            "PreToolUse",
            "2000",
            r#","tool_use_id":"toolu_child""#,
        )],
    );
    let (listing, _) = read_sink(dir.path()).expect("readable");

    // Selected by what the file IS, not by where it sorted. Path order put the
    // subagent first here — `sess__ab8b…` precedes `sess__alpha` — and a
    // fixture that indexes by position is asserting about the sort.
    let pick = |want_agent: bool| {
        listing
            .files
            .iter()
            .find(|f| {
                matches!(&f.attribution, Attribution::Attributed { agent, .. }
                    if agent.is_some() == want_agent)
            })
            .map(|f| &f.entries[0])
            .expect("both files are attributed")
    };
    let parent = pick(false);
    let child = pick(true);

    assert_eq!(
        parent.session, child.session,
        "same session — they are grouped"
    );
    assert_eq!(parent.agent, None);
    assert_eq!(child.agent.as_deref(), Some("ab8b50189992e6091"));
    assert_eq!(
        order(parent, child),
        RecordOrder::Unordered,
        "the subagent's work is nested inside the parent's open Agent call in \
         real traces, but nothing in THESE two payloads says so"
    );
}

// ---------------------------------------------------------------------------
// Attribution — boundary question 1
// ---------------------------------------------------------------------------

/// A file the reader cannot attribute is **read anyway** and named as
/// unattributed.
///
/// ADR-0011 §7a: discarding it would lose real events, guessing a source would
/// invent one, and calling it an error would say something failed when nothing
/// did. §7 permits hand-installed hooks, so this is the ordinary case.
#[test]
fn an_unattributable_file_is_named_and_its_records_are_still_read() {
    let dir = tempfile::tempdir().expect("tempdir");
    // No separator at all: nothing to parse into components.
    plant(
        dir.path(),
        "handrolled.jsonl",
        &[
            &rec("sess", None, "SessionStart", "1000", ""),
            &rec("sess", None, "SessionEnd", "2000", ""),
        ],
    );
    let (listing, _) = read_sink(dir.path()).expect("readable");
    assert_eq!(listing.unattributed, 1);
    assert_eq!(listing.files[0].attribution.key(), "unattributed");
    assert_eq!(
        listing.files[0].entries.len(),
        2,
        "the records are real and the session is named in every payload; only \
         the SOURCE is unknown"
    );
    assert_eq!(
        listing.files[0].entries[0].session.as_deref(),
        Some("sess"),
        "and the session is readable from the payload, not from the name"
    );

    // Paired: a well-named file IS attributed, or the above is satisfied by a
    // build that attributes nothing.
    plant(
        dir.path(),
        "sess__alpha.jsonl",
        &[&rec("sess", None, "SessionStart", "1000", "")],
    );
    let (listing, _) = read_sink(dir.path()).expect("readable");
    let attributed: Vec<&str> = listing.files.iter().map(|f| f.attribution.key()).collect();
    assert!(attributed.contains(&"attributed"));
    assert!(attributed.contains(&"unattributed"));
}

/// A name that parses into components but whose components are not valid path
/// components is unattributed too — not silently accepted.
#[test]
fn a_name_with_an_invalid_component_is_unattributed_rather_than_accepted() {
    let dir = tempfile::tempdir().expect("tempdir");
    plant(dir.path(), "sess__.jsonl", &[]);
    let (listing, _) = read_sink(dir.path()).expect("readable");
    assert_eq!(listing.files[0].attribution.key(), "unattributed");
    assert!(matches!(
        &listing.files[0].attribution,
        Attribution::Unattributed { reason } if reason.contains("identity")
    ));
}

// ---------------------------------------------------------------------------
// Disagreement — boundary question 2
// ---------------------------------------------------------------------------

/// When the filename and the records disagree about the session, the reader
/// **reports it and resolves nothing**.
///
/// One of them is wrong and nothing here can tell which; picking either would
/// be inventing the answer that decides which session a record belongs to.
#[test]
fn a_filename_that_disagrees_with_its_records_is_reported_not_resolved() {
    let dir = tempfile::tempdir().expect("tempdir");
    plant(
        dir.path(),
        "claimed__alpha.jsonl",
        &[
            &rec("actual", None, "SessionStart", "1000", ""),
            &rec("actual", None, "SessionEnd", "2000", ""),
        ],
    );
    let (listing, _) = read_sink(dir.path()).expect("readable");

    assert_eq!(
        listing.disagreements.len(),
        1,
        "{:#?}",
        listing.disagreements
    );
    let d = &listing.disagreements[0];
    assert_eq!(d.field, "session");
    assert_eq!(d.filename_says, "claimed");
    assert_eq!(d.record_says, "actual");
    assert_eq!(d.records, 2, "both records disagree, and the count says so");

    // Nothing was rewritten in either direction.
    assert!(matches!(
        &listing.files[0].attribution,
        Attribution::Attributed { session, .. } if session.as_str() == "claimed"
    ));
    assert_eq!(
        listing.files[0].entries[0].session.as_deref(),
        Some("actual"),
        "the entry keeps what the PAYLOAD said — the payload is the source and \
         the filename is the claim"
    );

    // Paired: an agreeing file produces no disagreement, or this is satisfied
    // by a build that reports one for every file.
    let dir2 = tempfile::tempdir().expect("tempdir");
    plant(
        dir2.path(),
        "sess__alpha.jsonl",
        &[&rec("sess", None, "SessionStart", "1000", "")],
    );
    let (clean, _) = read_sink(dir2.path()).expect("readable");
    assert!(clean.disagreements.is_empty(), "{:#?}", clean.disagreements);
}

/// **The payload beats the writer's hoist**, and this control exists because a
/// sabotage found it missing.
///
/// Every other fixture here builds records whose hoisted `session` equals the
/// payload's `session_id` — they are written in one operation from one parse, so
/// they cannot differ at write time. That made the preference unobservable:
/// sabotaging `Entry` to read the hoist instead of the payload left every test
/// green. A hand-edited file is what separates them, and it is the case the
/// preference exists for.
#[test]
fn where_the_hoist_and_the_payload_disagree_the_payload_wins() {
    let dir = tempfile::tempdir().expect("tempdir");
    // Hoisted session says `stale`; the payload says `real`. Only a hand edit
    // produces this, which is exactly when the hoist is the stale copy.
    let payload = r#"{"session_id":"real","hook_event_name":"SessionStart"}"#;
    let escaped = serde_json::to_string(payload).expect("escape");
    fs::write(
        dir.path().join("real__alpha.jsonl"),
        format!(
            r#"{{"v":"1","identity":"alpha","session":"stale","stamp_ns":"1000","payload":{escaped}}}"#
        ) + "\n",
    )
    .expect("plant");

    let (listing, _) = read_sink(dir.path()).expect("readable");
    assert_eq!(
        listing.files[0].entries[0].session.as_deref(),
        Some("real"),
        "the payload is the source of truth and the hoist is a reading of it; \
         trusting the hoist here would take the stale copy"
    );
    // And the disagreement is between the FILENAME and the payload — which
    // agree — so nothing is reported. The hoist being wrong is not itself a
    // session disagreement.
    assert!(
        listing.disagreements.is_empty(),
        "{:#?}",
        listing.disagreements
    );
}

/// The agent half of the same question: a file named for one agent holding
/// another agent's records.
#[test]
fn an_agent_disagreement_is_reported_too() {
    let dir = tempfile::tempdir().expect("tempdir");
    plant(
        dir.path(),
        "sess__aaaaaaaaaaaaaaaaa__alpha.jsonl",
        &[&rec(
            "sess",
            Some("bbbbbbbbbbbbbbbbb"),
            "PreToolUse",
            "1000",
            "",
        )],
    );
    let (listing, _) = read_sink(dir.path()).expect("readable");
    assert_eq!(listing.disagreements.len(), 1);
    assert_eq!(listing.disagreements[0].field, "agent");
}

// ---------------------------------------------------------------------------
// The stamp, and what it is allowed to do
// ---------------------------------------------------------------------------

/// A decreasing stamp inside one file is **reported** and changes no ordering.
#[test]
fn a_backwards_stamp_inside_one_file_is_reported_and_reorders_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    plant(
        dir.path(),
        "sess__alpha.jsonl",
        &[
            &rec("sess", None, "PreToolUse", "9000", r#","tool_use_id":"t1""#),
            &rec(
                "sess",
                None,
                "PostToolUse",
                "1000",
                r#","tool_use_id":"t1""#,
            ),
        ],
    );
    let (listing, _) = read_sink(dir.path()).expect("readable");
    assert_eq!(listing.clock_steps.len(), 1);
    assert_eq!(listing.clock_steps[0].from, "9000");
    assert_eq!(listing.clock_steps[0].to, "1000");

    // The payload still orders them, and the backwards stamp does not win.
    let e = &listing.files[0].entries;
    assert_eq!(
        order(&e[0], &e[1]),
        RecordOrder::Before {
            basis: OrderBasis::ToolPair
        },
        "the tool pair orders these; a stepped clock cannot invert it"
    );

    // Paired: non-decreasing stamps produce no observation.
    let dir2 = tempfile::tempdir().expect("tempdir");
    plant(
        dir2.path(),
        "sess__alpha.jsonl",
        &[
            &rec("sess", None, "PreToolUse", "1000", ""),
            &rec("sess", None, "PostToolUse", "2000", ""),
        ],
    );
    let (clean, _) = read_sink(dir2.path()).expect("readable");
    assert!(clean.clock_steps.is_empty());
}

// ---------------------------------------------------------------------------
// Statelessness, tails and unreadability
// ---------------------------------------------------------------------------

/// Reading twice gives the identical answer and leaves nothing behind.
///
/// ADR-0011 §7a: the reader is stateless — no watermark, no consumption
/// tracking. A second read that differed would mean state was being kept
/// somewhere.
#[test]
fn reading_twice_gives_the_same_answer_and_writes_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    plant(
        dir.path(),
        "sess__alpha.jsonl",
        &[&rec("sess", None, "SessionStart", "1000", "")],
    );
    let before: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .collect();

    let (first, _) = read_sink(dir.path()).expect("readable");
    let (second, _) = read_sink(dir.path()).expect("readable");
    assert_eq!(first, second, "the reader holds no state between reads");

    let after: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .collect();
    assert_eq!(before, after, "reading the sink must not write to it");
}

/// A torn tail survives the merge and is still reported per file.
#[test]
fn a_torn_tail_is_reported_by_the_listing() {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(
        dir.path().join("sess__alpha.jsonl"),
        format!(
            "{}\n{{\"v\":\"1\",\"ses",
            rec("sess", None, "SessionStart", "1000", "")
        ),
    )
    .expect("plant");
    let (listing, _) = read_sink(dir.path()).expect("readable");
    assert!(matches!(listing.files[0].tail, TailState::Partial { .. }));
    assert_eq!(listing.files[0].entries.len(), 1);
}

/// **One damaged file must not darken the sink**, and this control exists
/// because a decision rests on it that predates it.
///
/// ADR-0011 §2 measured that Claude Code's `timeout` kills a hook. §7a admits
/// exactly one corruption under one-writer-per-file — truncation — so a kill
/// can in principle leave a torn trailing record. The question that decides
/// whether a short `timeout` is admissible at all is what happens to the
/// **rest** of the sink when it does.
///
/// The existing torn-tail control uses a sink of one file, so it cannot see the
/// difference between *"this file reports a partial tail"* and *"a partial tail
/// costs you everything else"*. That difference is the whole of the concern: if
/// one torn line darkened the read, any kill, ever, would permanently blank the
/// monitor — silent non-delivery produced by the reader rather than by the
/// channel.
///
/// Four files in one sink, three of them damaged in different ways, and the
/// assertion is that **every whole record in every file is still read**.
#[test]
fn one_damaged_file_does_not_cost_the_sink_its_other_records() {
    let dir = tempfile::tempdir().expect("tempdir");

    // Undamaged.
    plant(
        dir.path(),
        "good__alpha.jsonl",
        &[
            &rec("good", None, "SessionStart", "1000", ""),
            &rec("good", None, "SessionEnd", "2000", ""),
        ],
    );

    // A whole record followed by a torn one — the kill case.
    fs::write(
        dir.path().join("torn__alpha.jsonl"),
        format!(
            "{}\n{{\"v\":\"1\",\"ses",
            rec("torn", None, "SessionStart", "1000", "")
        ),
    )
    .expect("plant");

    // A whole line that does not parse, between two that do — not a tail
    // problem at all, and a different arm of the reader.
    fs::write(
        dir.path().join("junk__alpha.jsonl"),
        format!(
            "{}\nthis line is not json at all\n{}\n",
            rec("junk", None, "SessionStart", "1000", ""),
            rec("junk", None, "SessionEnd", "2000", "")
        ),
    )
    .expect("plant");

    // A name the reader cannot attribute, whose records are still real.
    fs::write(
        dir.path().join("not-a-sink-name.jsonl"),
        format!("{}\n", rec("orphan", None, "SessionStart", "1000", "")),
    )
    .expect("plant");

    let (listing, unreadable) = read_sink(dir.path()).expect(
        "a sink containing damaged files is still readable — the error case is a \
         directory that cannot be enumerated, which is a different fact",
    );
    assert!(unreadable.is_empty(), "{unreadable:?}");
    assert_eq!(listing.files.len(), 4);

    let records_in = |name: &str| -> usize {
        listing
            .files
            .iter()
            .find(|f| f.path.file_name().and_then(|n| n.to_str()) == Some(name))
            .unwrap_or_else(|| panic!("{name} is missing from the listing"))
            .entries
            .len()
    };

    assert_eq!(records_in("good__alpha.jsonl"), 2, "the undamaged file");
    assert_eq!(
        records_in("torn__alpha.jsonl"),
        1,
        "the whole record BEFORE a torn tail is still a record"
    );
    assert_eq!(
        records_in("junk__alpha.jsonl"),
        2,
        "a whole line that does not parse must not take its neighbours with it"
    );
    assert_eq!(
        records_in("not-a-sink-name.jsonl"),
        1,
        "an unattributable name is a state, not a reason to drop records"
    );

    // And the damage is REPORTED rather than absorbed, or the assertions above
    // are satisfied by a reader that silently discards what it cannot read.
    let torn = listing
        .files
        .iter()
        .find(|f| f.path.file_name().and_then(|n| n.to_str()) == Some("torn__alpha.jsonl"))
        .expect("present");
    assert!(
        matches!(torn.tail, TailState::Partial { .. }),
        "a torn tail must be reported, not merely survived"
    );
    let junk = listing
        .files
        .iter()
        .find(|f| f.path.file_name().and_then(|n| n.to_str()) == Some("junk__alpha.jsonl"))
        .expect("present");
    assert_eq!(junk.unparseable, 1, "the unparseable line must be counted");
    assert!(
        matches!(junk.tail, TailState::Complete),
        "a bad line in the middle is not a tail problem, and the two must not \
         share an observable"
    );
    assert_eq!(listing.unattributed, 1);

    // Paired: the same sink with nothing damaged reports no damage, or every
    // assertion above is satisfied by a reader that reports damage always.
    let clean = tempfile::tempdir().expect("tempdir");
    plant(
        clean.path(),
        "good__alpha.jsonl",
        &[&rec("good", None, "SessionStart", "1000", "")],
    );
    let (clean_listing, _) = read_sink(clean.path()).expect("readable");
    assert_eq!(clean_listing.unattributed, 0);
    assert!(matches!(clean_listing.files[0].tail, TailState::Complete));
    assert_eq!(clean_listing.files[0].unparseable, 0);
}

/// An empty sink and an unreadable sink are different facts.
#[test]
fn an_empty_sink_and_a_missing_sink_do_not_render_the_same() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (empty, unreadable) = read_sink(dir.path()).expect("an empty directory is readable");
    assert!(empty.files.is_empty());
    assert!(unreadable.is_empty());

    let missing = dir.path().join("no-such-sink");
    assert!(
        read_sink(&missing).is_err(),
        "a sink that cannot be enumerated is an error, not an empty listing — \
         those are different facts and must not render the same"
    );
}

/// Two declared identities that fold to one filename key are reported by the
/// reader as well, even though §7a puts the real check on the config.
#[test]
fn identities_colliding_under_the_filename_key_are_reported() {
    let dir = tempfile::tempdir().expect("tempdir");
    plant(
        dir.path(),
        "sess__alpha.jsonl",
        &[&rec("sess", None, "SessionStart", "1000", "")],
    );
    plant(
        dir.path(),
        "other__Alpha.jsonl",
        &[&rec("other", None, "SessionStart", "1000", "")],
    );
    let (listing, _) = read_sink(dir.path()).expect("readable");
    assert_eq!(
        listing.identity_collisions,
        vec!["alpha".to_owned()],
        "`alpha` and `Alpha` are one file on NTFS — found late here, which is \
         why the enforcing check is on the config"
    );

    // Paired: genuinely distinct identities produce no collision.
    let dir2 = tempfile::tempdir().expect("tempdir");
    plant(
        dir2.path(),
        "sess__alpha.jsonl",
        &[&rec("sess", None, "SessionStart", "1000", "")],
    );
    plant(
        dir2.path(),
        "sess__beta.jsonl",
        &[&rec("sess", None, "SessionStart", "1000", "")],
    );
    let (clean, _) = read_sink(dir2.path()).expect("readable");
    assert!(clean.identity_collisions.is_empty());
}

// ---------------------------------------------------------------------------
// The listing-level verdict — boundary question 3, decided as two layers
// ---------------------------------------------------------------------------

/// **A listing containing unordered pairs says so.**
///
/// The per-pair verdict is the contract and it is complete, but a listing that
/// contains unordered pairs and does not say so **reads as a sequence** —
/// records come out in some order, and an order that is an artifact of
/// iteration is indistinguishable from one that was established.
///
/// ADR-0010 §5's argument: the state is shown by default because the failure is
/// *"I didn't notice"*, and a property available on request does not guard
/// against not noticing.
#[test]
fn a_listing_with_unordered_pairs_reports_that_it_is_not_a_sequence() {
    let dir = tempfile::tempdir().expect("tempdir");
    plant(
        dir.path(),
        "sess__alpha.jsonl",
        &[
            &rec("sess", None, "PreToolUse", "1000", r#","tool_use_id":"a""#),
            &rec("sess", None, "PreToolUse", "2000", r#","tool_use_id":"b""#),
        ],
    );
    let (listing, _) = read_sink(dir.path()).expect("readable");
    assert_eq!(
        listing.sequencing,
        Sequencing::PartlyUnordered { unordered_pairs: 1 }
    );
    assert!(
        !listing.sequencing.may_be_presented_as_a_sequence(),
        "presenting this as a sequence would be presenting a plausible history"
    );
}

/// **Paired: a fully ordered listing says that too**, or the assertion above is
/// satisfied by a build that calls everything unordered — which would be honest
/// and useless, and would make the flag carry no information.
#[test]
fn a_fully_ordered_listing_may_be_presented_as_a_sequence() {
    let dir = tempfile::tempdir().expect("tempdir");
    plant(
        dir.path(),
        "sess__alpha.jsonl",
        &[
            &rec("sess", None, "SessionStart", "1000", ""),
            &rec("sess", None, "SessionEnd", "2000", ""),
        ],
    );
    let (listing, _) = read_sink(dir.path()).expect("readable");
    assert_eq!(listing.sequencing, Sequencing::FullyOrdered);
    assert!(listing.sequencing.may_be_presented_as_a_sequence());
}

/// Zero or one record is `Trivial`, and `Trivial` does **not** license a
/// sequence.
///
/// An empty listing that answered "yes, present me as a sequence" would let the
/// most common case in a fresh sink hand out the permission the flag exists to
/// withhold.
#[test]
fn an_empty_or_single_record_listing_is_trivial_and_licenses_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (empty, _) = read_sink(dir.path()).expect("readable");
    assert_eq!(empty.sequencing, Sequencing::Trivial);
    assert!(!empty.sequencing.may_be_presented_as_a_sequence());

    plant(
        dir.path(),
        "sess__alpha.jsonl",
        &[&rec("sess", None, "SessionStart", "1000", "")],
    );
    let (one, _) = read_sink(dir.path()).expect("readable");
    assert_eq!(one.sequencing, Sequencing::Trivial);
    assert!(!one.sequencing.may_be_presented_as_a_sequence());
}

/// The listing-level flag is **derived from the primitive**, not maintained
/// beside it: every pair the flag counts is a pair `order` calls unordered.
///
/// Asserted rather than assumed, because a second field kept equal to the
/// primitive is the shape ADR-0010 §3 rejected and its staleness would be
/// silent in the dangerous direction.
#[test]
fn the_listing_flag_agrees_with_the_primitive_pair_by_pair() {
    let dir = tempfile::tempdir().expect("tempdir");
    plant(
        dir.path(),
        "sess__alpha.jsonl",
        &[
            &rec("sess", None, "SessionStart", "1000", ""),
            &rec("sess", None, "PreToolUse", "2000", r#","tool_use_id":"a""#),
            &rec("sess", None, "PreToolUse", "3000", r#","tool_use_id":"b""#),
            &rec("sess", None, "SessionEnd", "4000", ""),
        ],
    );
    let (listing, _) = read_sink(dir.path()).expect("readable");
    let e = &listing.files[0].entries;

    let mut counted = 0usize;
    for (i, a) in e.iter().enumerate() {
        for b in &e[i + 1..] {
            if !order(a, b).is_ordered() {
                counted += 1;
            }
        }
    }
    assert_eq!(
        listing.sequencing,
        Sequencing::PartlyUnordered {
            unordered_pairs: counted
        },
        "the flag must be exactly what the primitive says, recomputed"
    );
    assert_eq!(
        counted, 1,
        "only the two independent tool calls are unordered"
    );
}
