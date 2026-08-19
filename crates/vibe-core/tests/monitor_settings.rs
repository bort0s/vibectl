//! Controls for the `settings.json` edit — ADR-0011 §7b.
//!
//! **The fixture is a re-install, not a first install.** Every control here
//! starts from a 49-line file that already carries user keys, a user's own hook
//! under a `matcher`, and a vibe hook. That is the file install actually meets:
//! ADR-0011 §7 measured three real settings files on one machine and **none of
//! them declared `hooks` at all**, so a clean file is the case that proves
//! least.
//!
//! The formatting numbers quoted below are measured, not chosen — see §7b's
//! table.

use vibe_core::monitor::{
    HOOK_TIMEOUT_SECS, HookSpec, INSTALLED_EVENTS, InstallOutcome, MATCH_ALL, SettingsDocument,
    WriterIdentity, install, read_document,
};

fn spec(identity: &str) -> HookSpec {
    HookSpec {
        command: std::path::PathBuf::from("C:/Users/x/.local/bin/vibe.exe"),
        sink: std::path::PathBuf::from("C:/Users/x/AppData/Local/vibe/data/monitor"),
        identity: WriterIdentity::parse(identity).expect("fixture identity is valid"),
    }
}

/// The file install meets on its second run: user keys, a user's own hook under
/// a matcher, and a vibe group already present.
fn realistic(indent: &str, crlf: bool) -> String {
    let body = r#"{
  "model": "opus[1m]",
  "effortLevel": "xhigh",
  "permissions": {
    "allow": [
      "Bash(cargo test:*)",
      "Read(*)"
    ]
  },
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Bash",
        "hooks": [
          {
            "type": "command",
            "command": "C:/tools/my-audit.exe",
            "timeout": 10
          }
        ]
      }
    ],
    "SessionStart": [
      {
        "matcher": "*",
        "hooks": [
          {
            "type": "command",
            "command": "C:/Users/x/.local/bin/vibe.exe",
            "args": [
              "monitor",
              "hook",
              "--identity",
              "user",
              "--sink",
              "C:/old/sink",
              "--contract",
              "1"
            ],
            "once": false,
            "async": false,
            "asyncRewake": false,
            "timeout": 5
          }
        ]
      }
    ]
  },
  "env": {
    "MY_VAR": "1"
  }
}
"#;
    let reindented = if indent == "  " {
        body.to_owned()
    } else {
        body.lines()
            .map(|l| {
                let depth = (l.len() - l.trim_start().len()) / 2;
                format!("{}{}", indent.repeat(depth), l.trim_start())
            })
            .collect::<Vec<_>>()
            .join("\n")
            + "\n"
    };
    if crlf {
        reindented.replace('\n', "\r\n")
    } else {
        reindented
    }
}

// ---------------------------------------------------------------------------
// Formatting: sniffed, not imposed
// ---------------------------------------------------------------------------

/// **A document survives a round trip byte-for-byte, in the style it arrived
/// in** — and the naive path is asserted to have damaged it, or this control
/// passes against a build that got lucky on a two-space file.
#[test]
fn a_round_trip_preserves_indent_and_line_ending_and_the_naive_path_would_not() {
    for (name, indent, crlf, naive_damage) in [
        ("2-space LF", "  ", false, false),
        ("4-space LF", "    ", false, true),
        ("tab LF", "\t", false, true),
        ("2-space CRLF", "  ", true, true),
    ] {
        let text = realistic(indent, crlf);
        let doc = SettingsDocument::parse(&text).expect("strict json");
        assert_eq!(
            doc.render(),
            text,
            "{name}: the round trip changed a file vibe does not own"
        );

        // The premise, per row: the fixed-format path really would have
        // damaged this one. Without it, three of these four rows prove nothing
        // beyond "two-space LF is the default".
        let value: serde_json::Value = serde_json::from_str(&text).expect("json");
        let naive = serde_json::to_string_pretty(&value).expect("pretty") + "\n";
        assert_eq!(
            naive != text,
            naive_damage,
            "{name}: the fixture's premise is wrong about what the naive path does"
        );
    }
}

/// **Unknown keys and key order both survive**, because this build's idea of
/// the schema is a snapshot of somebody else's.
#[test]
fn an_unknown_key_and_the_key_order_both_survive() {
    let text = "{\n  \"zzz_from_a_newer_build\": {\n    \"nested\": true\n  },\n  \"model\": \"opus\"\n}\n";
    let doc = SettingsDocument::parse(text).expect("strict json");
    let out = doc.render();
    assert_eq!(out, text);
    assert!(
        out.find("zzz_from_a_newer_build") < out.find("model"),
        "key order was not preserved, so `serde_json/preserve_order` is off"
    );
}

/// The residuals are what the module says they are, asserted rather than
/// described: nothing to sniff falls back to two spaces, and a file that
/// disagrees with itself takes whichever came first.
#[test]
fn the_declared_sniffing_residuals_behave_as_declared() {
    let minified = "{\"a\":1}\n";
    assert_eq!(
        SettingsDocument::parse(minified).expect("json").indent(),
        "  ",
        "nothing to sniff must fall back to the declared default"
    );

    let mixed = "{\n\t\"a\": 1,\n  \"b\": 2\n}\n";
    assert_eq!(
        SettingsDocument::parse(mixed).expect("json").indent(),
        "\t",
        "mixed indentation takes the FIRST indented line, which is the declared \
         residual — a file disagreeing with itself is normalised and the rest \
         rewritten"
    );

    let mixed_endings = "{\r\n  \"a\": 1,\n  \"b\": 2\r\n}\n";
    assert_eq!(
        SettingsDocument::parse(mixed_endings)
            .expect("json")
            .newline(),
        "\r\n",
        "mixed line endings carry the same residual as mixed indentation"
    );
}

// ---------------------------------------------------------------------------
// What the emitted hook contains — and what it must not
// ---------------------------------------------------------------------------

/// **The emitted hook uses the `args` exec form**, and this is the control that
/// keeps `shell` closed.
///
/// `shell` has no value meaning *no shell* — it is a string enum of `bash` and
/// `powershell`, and the loader refuses `false` (ADR-0011 §2, round 3d). So
/// install cannot delete that dependency by writing the field; what keeps a
/// shell out of the channel is `args`. A later move to the string `command`
/// form would make `shell` live again, and without this nothing would say so.
///
/// **Paired**: the string form is what the absence of `args` means, so the
/// control also asserts the emitted hook is not that.
#[test]
fn the_emitted_hook_uses_the_args_exec_form() {
    let hook = spec("user").hook_object();
    let args = hook
        .get("args")
        .and_then(serde_json::Value::as_array)
        .expect(
            "no `args`: the hook is in the STRING command form, which puts a shell in the \
             channel — ADR-0011 §7a chose the exec form precisely to keep one out",
        );
    assert!(!args.is_empty());
    assert_eq!(args[0].as_str(), Some("monitor"));
    assert_eq!(args[1].as_str(), Some("hook"));
    assert!(
        hook.get("command")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|c| !c.contains(' ') || !c.contains("--identity")),
        "`command` must name an executable, not carry the arguments — with \
         `args` present it is resolved as a program and spawned directly"
    );
}

/// Every field install depends on is written, and the two it must not write are
/// absent for stated reasons.
#[test]
fn the_emitted_hook_writes_its_dependencies_and_omits_the_two_it_cannot() {
    let hook = spec("user").hook_object();
    let obj = hook.as_object().expect("object");

    for (key, expected) in [
        ("once", serde_json::Value::Bool(false)),
        ("async", serde_json::Value::Bool(false)),
        ("asyncRewake", serde_json::Value::Bool(false)),
        ("timeout", serde_json::Value::from(HOOK_TIMEOUT_SECS)),
    ] {
        assert_eq!(
            obj.get(key),
            Some(&expected),
            "`{key}` must be written, or install depends on an upstream default \
             with nothing watching it"
        );
    }

    assert!(
        !obj.contains_key("shell"),
        "`shell` must NOT be written: it is a string enum with no value meaning \
         `no shell`, and the loader refuses `false`. What closes it is `args`."
    );
    assert!(
        !obj.contains_key("if"),
        "`if` must NOT be written: `if: \"*\"` is accepted by the loader and \
         fires nothing, so there is no value meaning `do not suppress`. It is \
         the one residual omission-dependency."
    );

    // The group carries the matcher, not the hook — measured on the lifecycle
    // five, which is the class install writes.
    let group = spec("user").group();
    assert_eq!(
        group.get("matcher").and_then(|m| m.as_str()),
        Some(MATCH_ALL)
    );
}

// ---------------------------------------------------------------------------
// Idempotency, which is the axis
// ---------------------------------------------------------------------------

/// **Install into the realistic file: vibe gets its OWN group, and the user's
/// hook is untouched.**
#[test]
fn install_adds_its_own_group_and_leaves_the_users_hook_alone() {
    let text = realistic("  ", false);
    let mut doc = SettingsDocument::parse(&text).expect("json");
    let outcome = install(&mut doc, &spec("user")).expect("installable");

    // The existing SessionStart group declared the same identity with an old
    // sink, so this is an upgrade rather than a first install.
    assert!(
        matches!(outcome, InstallOutcome::Upgraded { .. }),
        "{outcome:?}"
    );

    let out = doc.render();
    // The user's own hook, under their own matcher, verbatim.
    assert!(out.contains("C:/tools/my-audit.exe"));
    assert!(out.contains("\"matcher\": \"Bash\""));
    // Their other keys, in their order.
    assert!(out.find("\"model\"") < out.find("\"effortLevel\""));
    assert!(out.contains("MY_VAR"));
    // The stale sink is gone and the new one is there.
    assert!(!out.contains("C:/old/sink"));
    assert!(out.contains("AppData/Local/vibe/data/monitor"));

    // All five events now carry exactly one vibe group.
    let value: serde_json::Value = serde_json::from_str(&out).expect("still json");
    for event in INSTALLED_EVENTS {
        let groups = value
            .pointer(&format!("/hooks/{event}"))
            .and_then(serde_json::Value::as_array)
            .unwrap_or_else(|| panic!("{event} missing"));
        let ours = groups
            .iter()
            .filter(|g| g.to_string().contains("--identity"))
            .count();
        assert_eq!(ours, 1, "{event} should carry exactly one vibe group");
    }
}

/// **Re-install changes nothing.** The second run reports `Unchanged` and the
/// bytes are identical — which is what makes re-install the normal path rather
/// than a thing to be feared.
#[test]
fn re_install_is_idempotent_down_to_the_bytes() {
    let text = realistic("  ", false);
    let mut doc = SettingsDocument::parse(&text).expect("json");
    install(&mut doc, &spec("user")).expect("first");
    let after_first = doc.render();

    let mut doc2 = SettingsDocument::parse(&after_first).expect("json");
    let outcome = install(&mut doc2, &spec("user")).expect("second");
    assert_eq!(outcome, InstallOutcome::Unchanged);
    assert_eq!(doc2.render(), after_first, "a re-install rewrote the file");

    // Paired: a DIFFERENT identity is a different writer and must be added, or
    // "unchanged" is satisfied by a build that never writes anything.
    let mut doc3 = SettingsDocument::parse(&after_first).expect("json");
    let other = install(&mut doc3, &spec("second")).expect("third");
    assert!(
        matches!(other, InstallOutcome::Installed { .. }),
        "{other:?}"
    );
    assert_ne!(doc3.render(), after_first);
}

// ---------------------------------------------------------------------------
// Report, never repair
// ---------------------------------------------------------------------------

/// **A vibe hook sitting inside somebody else's group is refused**, because
/// neither repair is vibe's to choose: replacing the group deletes their
/// config, and replacing the hook alone leaves vibe's delivery under a
/// `matcher` vibe never wrote.
///
/// **Paired** against the same document with the hook in its own group, which
/// installs normally — or a build that refuses everything satisfies this.
#[test]
fn a_vibe_hook_inside_someone_elses_group_is_reported_not_repaired() {
    let shared = r#"{
  "hooks": {
    "SessionStart": [
      {
        "matcher": "Bash",
        "hooks": [
          { "type": "command", "command": "C:/tools/theirs.exe" },
          { "type": "command", "command": "C:/vibe.exe",
            "args": ["monitor", "hook", "--identity", "user", "--sink", "s", "--contract", "1"] }
        ]
      }
    ]
  }
}
"#;
    let mut doc = SettingsDocument::parse(shared).expect("json");
    let before = doc.render();
    let err = install(&mut doc, &spec("user")).expect_err("must refuse");
    assert_eq!(err.key(), "identity_shares_a_group");
    assert_eq!(
        doc.render(),
        before,
        "a refusal must leave the document exactly as it was"
    );

    let alone = shared.replace(
        r#"          { "type": "command", "command": "C:/tools/theirs.exe" },
"#,
        "",
    );
    let mut doc2 = SettingsDocument::parse(&alone).expect("json");
    assert!(
        install(&mut doc2, &spec("user")).is_ok(),
        "the same hook in a group of its own must install normally"
    );
}

/// One identity declared in two groups for one event is §7a's uniqueness fault,
/// seen where §7a puts the check.
#[test]
fn one_identity_in_two_groups_for_one_event_is_refused() {
    let twice = r#"{
  "hooks": {
    "SessionStart": [
      { "matcher": "*", "hooks": [ { "type": "command", "command": "a",
        "args": ["monitor", "hook", "--identity", "user", "--sink", "s", "--contract", "1"] } ] },
      { "matcher": "*", "hooks": [ { "type": "command", "command": "b",
        "args": ["monitor", "hook", "--identity", "user", "--sink", "t", "--contract", "1"] } ] }
    ]
  }
}
"#;
    let mut doc = SettingsDocument::parse(twice).expect("json");
    let err = install(&mut doc, &spec("user")).expect_err("must refuse");
    assert_eq!(err.key(), "identity_declared_twice");
}

/// A shape this module does not recognise is refused rather than coerced, and
/// the refusal names where.
#[test]
fn an_unrecognised_shape_is_refused_and_says_where() {
    for (text, key) in [
        ("{\"hooks\": 3}\n", "hooks_not_an_object"),
        ("{\"hooks\": {\"SessionStart\": 3}}\n", "event_not_an_array"),
        (
            "{\"hooks\": {\"SessionStart\": [3]}}\n",
            "group_not_an_object",
        ),
        (
            "{\"hooks\": {\"SessionStart\": [{\"hooks\": 3}]}}\n",
            "group_hooks_not_an_array",
        ),
    ] {
        let mut doc = SettingsDocument::parse(text).expect("json");
        let err = install(&mut doc, &spec("user")).expect_err("must refuse");
        assert_eq!(err.key(), key, "for {text}");
    }

    // Not JSON at all, and not an object, are separate facts.
    assert_eq!(
        SettingsDocument::parse("{ // hi\n}")
            .expect_err("refuse")
            .key(),
        "not_json",
        "a comment is not strict JSON — measured, Claude Code refuses it too"
    );
    assert_eq!(
        SettingsDocument::parse("[1,2]").expect_err("refuse").key(),
        "not_an_object"
    );
}

/// **A refusal on a later event leaves the earlier ones alone.** `apply` is
/// all-or-nothing at the decision level (ADR-0001 §3); this is that rule one
/// layer in, and without it a document could come back half-written.
#[test]
fn a_refusal_on_the_last_event_leaves_the_first_four_untouched() {
    let text = r#"{
  "hooks": {
    "Stop": [
      {
        "hooks": [
          { "type": "command", "command": "theirs" },
          { "type": "command", "command": "ours",
            "args": ["monitor", "hook", "--identity", "user", "--sink", "s", "--contract", "1"] }
        ]
      }
    ]
  }
}
"#;
    let mut doc = SettingsDocument::parse(text).expect("json");
    let before = doc.render();
    let err = install(&mut doc, &spec("user")).expect_err("must refuse on Stop");
    assert_eq!(err.key(), "identity_shares_a_group");
    assert_eq!(
        doc.render(),
        before,
        "SessionStart comes first in INSTALLED_EVENTS, so a build that wrote as \
         it went would have added four groups before reaching the refusal"
    );
}

/// An absent settings file is a first install, not an error and not an empty
/// file — and the document it produces is well-formed.
#[test]
fn an_absent_settings_file_installs_from_empty() {
    let mut doc = SettingsDocument::empty();
    let outcome = install(&mut doc, &spec("user")).expect("installable");
    assert!(matches!(outcome, InstallOutcome::Installed { .. }));
    let out = doc.render();
    let value: serde_json::Value = serde_json::from_str(&out).expect("valid json");
    assert!(value.pointer("/hooks/SessionStart").is_some());
    assert!(out.ends_with('\n'));
}

// ---------------------------------------------------------------------------
// The file on disk, and the write that never happens on a refusal
// ---------------------------------------------------------------------------

/// **A settings file that is not valid JSON is refused, and the bytes on disk
/// are untouched.**
///
/// The three report-never-repair cases already controlled are all *semantic* —
/// a shape this module does not recognise. The **syntactic** one was missing,
/// and it is the likelier one: users edit this file by hand, and ADR-0011 §7
/// measured Claude Code refusing a `//` comment, a `/* */` comment and a
/// trailing comma. Each of those is a file that exists, parses nowhere, and
/// must not be opened in truncate mode to find that out.
///
/// This asserts against the **bytes on disk**, through the whole read-parse
/// route, rather than against a `Result` — because *"parse returned an error"*
/// and *"nothing was written"* are different claims and only the second is the
/// one that matters to somebody's config.
///
/// **Paired**: the same route over a valid file writes, so *"the file is
/// unchanged"* is not satisfied by a build that never writes anything.
#[test]
fn an_unparseable_settings_file_is_refused_with_its_bytes_untouched() {
    let dir = tempfile::tempdir().expect("tempdir");

    for (name, text) in [
        (
            "line-comment",
            "{\n  // a user's note\n  \"model\": \"opus\"\n}\n",
        ),
        (
            "block-comment",
            "{\n  /* a note */\n  \"model\": \"opus\"\n}\n",
        ),
        ("trailing-comma", "{\n  \"model\": \"opus\",\n}\n"),
        ("truncated", "{\n  \"model\": \"op"),
    ] {
        let path = dir.path().join(format!("{name}.json"));
        std::fs::write(&path, text).expect("plant");
        let before = std::fs::read(&path).expect("read");

        let on_disk = read_document(&path).expect("readable").expect("present");
        let refusal = SettingsDocument::parse(&on_disk).expect_err("must refuse");
        assert_eq!(refusal.key(), "not_json", "for {name}");

        assert_eq!(
            std::fs::read(&path).expect("read"),
            before,
            "{name}: the file changed while being refused"
        );
    }

    // Paired: a valid file does go through, or nothing above is a measurement
    // of restraint.
    let ok = dir.path().join("ok.json");
    std::fs::write(&ok, "{\n  \"model\": \"opus\"\n}\n").expect("plant");
    let text = read_document(&ok).expect("readable").expect("present");
    let mut doc = SettingsDocument::parse(&text).expect("valid");
    install(&mut doc, &spec("user")).expect("installs");
    vibe_core::write_atomically(&ok, &doc.render()).expect("write");
    assert!(
        std::fs::read_to_string(&ok)
            .expect("read")
            .contains("--identity"),
        "the valid file must actually have been written"
    );
}

/// **An absent settings file is not an unreadable one.**
#[test]
fn an_absent_file_and_an_unreadable_one_are_different_facts() {
    let dir = tempfile::tempdir().expect("tempdir");
    assert_eq!(
        read_document(&dir.path().join("nope.json")).expect("absent is Ok"),
        None,
        "a missing settings.json is a first install, not an error"
    );

    // A directory where a file should be: present, and not readable as one.
    let as_dir = dir.path().join("settings.json");
    std::fs::create_dir(&as_dir).expect("mkdir");
    assert!(
        read_document(&as_dir).is_err(),
        "something that exists and cannot be read must not report as absent — \
         that would install over it"
    );
}

/// **The generated group is byte-identical to the group that was validated.**
///
/// ADR-0011 §2 round 3d ran a hook group end to end — accepted by the loader,
/// firing on all five lifecycle events, 1:1 against a bare control. That group
/// was **hand-written**. The editor generates one, and *"the hand-written one
/// works"* says nothing about the generated one: they are different artifacts.
///
/// So the validated shape is pinned here as a literal, and the generator is
/// asserted equal to it. The end-to-end run against the **editor's own output**
/// is the stronger check and it was done — but it needs a real Claude Code
/// binary and a live session, so it cannot run in CI, and this can.
#[test]
fn the_generated_group_matches_the_group_that_was_validated() {
    let validated: serde_json::Value = serde_json::from_str(
        r#"{
          "matcher": "*",
          "hooks": [
            {
              "type": "command",
              "command": "C:/Users/x/.local/bin/vibe.exe",
              "args": ["monitor", "hook",
                       "--identity", "user",
                       "--sink", "C:/Users/x/AppData/Local/vibe/data/monitor",
                       "--contract", "1"],
              "once": false,
              "async": false,
              "asyncRewake": false,
              "timeout": 5
            }
          ]
        }"#,
    )
    .expect("the validated group is json");

    assert_eq!(
        spec("user").group(),
        validated,
        "the editor now emits a different group from the one that was run \
         against a real agent. Either re-run it end to end or bring this \
         literal back to what was validated — a generator that drifted from \
         its validation is a control asserting the wrong artifact."
    );
}
