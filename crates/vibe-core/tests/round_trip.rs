//! The invariant this project is built on: **editing a manifest must not
//! disturb anything the edit did not address.**
//!
//! Two properties, each with a negative control against the naive
//! `toml`-crate path. The negative controls are the point. A test that only
//! asserts our implementation passes proves nothing about whether the property
//! is hard to satisfy — if the obvious serde round-trip also passed, the whole
//! `toml_edit` requirement would be unjustified ceremony. So each property is
//! run twice: once through [`ManifestDocument`], which must preserve, and once
//! through `toml::from_str` -> `toml::to_string`, which must lose.
//!
//! Property 1 — **comments and formatting survive an edit.**
//! Property 2 — **keys this build has never heard of survive an edit.**
//!
//! Property 2 is the sharper one. ADR-0002 §3 says a manifest from a newer
//! *minor* schema is read, its unrecognised keys ignored, and written back. That
//! is only safe if no write path drops them — the moment `sync` reserialises
//! from a serde struct, every forward-compatible field silently vanishes from
//! the exact file the policy exists to protect.

use proptest::prelude::*;
use serde::{Deserialize, Serialize};

use vibe_core::manifest::FieldEdit;
use vibe_core::{ManifestDocument, Status, Visibility};

const PATH: &str = "/tmp/proj/.vibe/project.toml";

// --- sentinel comments -------------------------------------------------
//
// Placed on keys and sections that no generated `FieldEdit` can address, so
// "did the edit disturb something it had no business touching?" is a plain
// substring check rather than an argument about which comment belongs to which
// key. An edit is allowed to reformat the value it edits; it is allowed to
// touch nothing else.

const C_HEADER: &str = "# managed by vibe — hand edits are preserved";
const C_BEFORE_PROJECT: &str = "# who and what";
const C_ON_NAME: &str = "# the slug, not the title";
const C_ON_CREATED: &str = "# first commit, roughly";
const C_BEFORE_REPO: &str = "# where it lives";
const C_BEFORE_DEPLOY: &str = "# where it runs";

// These two sit on keys the generated edits DO address, which is what makes
// the property non-vacuous. Without them, every sentinel would be on an
// untouched key, and an implementation that discarded the decor of the key it
// edits would pass — `status = "active"  # confirmed with the team` would
// quietly become `status = "shipped"` and no test would notice.
const C_ON_STATUS: &str = "# confirmed with the team";
const C_BEFORE_FRAMEWORKS: &str = "# pinned deliberately";

const SENTINELS: &[&str] = &[
    C_HEADER,
    C_BEFORE_PROJECT,
    C_ON_NAME,
    C_ON_CREATED,
    C_BEFORE_REPO,
    C_BEFORE_DEPLOY,
    C_ON_STATUS,
    C_BEFORE_FRAMEWORKS,
];

/// Plausible keys a future minor version might add. None collide with a key
/// this build knows, so they must survive untouched.
const FUTURE_KEYS: &[&str] = &["regions", "risks", "owner", "tags", "budget", "runbook"];

#[derive(Debug, Clone)]
struct ManifestSpec {
    name: String,
    description: String,
    status: String,
    created: String,
    frameworks: Vec<String>,
    services: Vec<String>,
    /// Render `frameworks` across multiple lines instead of inline.
    frameworks_multiline: bool,
    /// Extra blank lines between sections.
    gap: usize,
    /// `(key, value)` pairs this build does not know, at the top level.
    unknown_top: Vec<(String, String)>,
    /// The same, inside the known `[deploy]` table.
    unknown_nested: Vec<(String, String)>,
}

impl ManifestSpec {
    fn render(&self) -> String {
        let nl = "\n".repeat(1 + self.gap);
        let mut s = String::new();

        s.push_str(C_HEADER);
        s.push('\n');
        s.push_str("schema_version = \"1.0\"\n");
        s.push_str(&nl);

        s.push_str(C_BEFORE_PROJECT);
        s.push('\n');
        s.push_str("[project]\n");
        s.push_str(&format!("name = \"{}\"  {C_ON_NAME}\n", self.name));
        s.push_str(&format!("description = \"{}\"\n", self.description));
        s.push_str(&format!("status = \"{}\"  {C_ON_STATUS}\n", self.status));
        s.push_str(&format!("created = \"{}\"  {C_ON_CREATED}\n", self.created));
        for (k, v) in &self.unknown_nested {
            // Nested unknowns live in `[project]` as well as `[deploy]`, so the
            // property covers unknown keys inside a table the edit *does* touch.
            s.push_str(&format!("{k} = \"{v}\"\n"));
        }
        s.push_str(&nl);

        s.push_str("[stack]\n");
        s.push_str("runtime = \"node@22\"\n");
        s.push_str(C_BEFORE_FRAMEWORKS);
        s.push('\n');
        if self.frameworks_multiline {
            s.push_str("frameworks = [\n");
            for f in &self.frameworks {
                s.push_str(&format!("  \"{f}\",\n"));
            }
            s.push_str("]\n");
        } else {
            let inner = self
                .frameworks
                .iter()
                .map(|f| format!("\"{f}\""))
                .collect::<Vec<_>>()
                .join(", ");
            s.push_str(&format!("frameworks = [{inner}]\n"));
        }
        let services = self
            .services
            .iter()
            .map(|f| format!("\"{f}\""))
            .collect::<Vec<_>>()
            .join(", ");
        s.push_str(&format!("services = [{services}]\n"));
        s.push_str(&nl);

        s.push_str(C_BEFORE_REPO);
        s.push('\n');
        s.push_str("[repo]\n");
        s.push_str("remote = \"github.com/bort0s/thing\"\n");
        s.push_str("visibility = \"private\"\n");
        s.push_str(&nl);

        s.push_str(C_BEFORE_DEPLOY);
        s.push('\n');
        s.push_str("[deploy]\n");
        s.push_str("url = \"https://thing.example\"\n");
        s.push_str("env_required = [\"A_KEY\"]\n");
        s.push_str(&nl);

        // Top-level unknown keys go in their own table so they cannot be
        // confused with a known one.
        if !self.unknown_top.is_empty() {
            s.push_str("[future]\n");
            for (k, v) in &self.unknown_top {
                s.push_str(&format!("{k} = \"{v}\"\n"));
            }
        }
        s
    }
}

// Charsets are restricted to what needs no TOML escaping. The generator's job
// is to vary structure and formatting, not to fuzz the TOML lexer — `toml_edit`
// has its own test suite for that.
fn safe_word() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9_-]{0,10}".prop_map(|s| s)
}

fn spec_strategy() -> impl Strategy<Value = ManifestSpec> {
    (
        safe_word(),
        safe_word(),
        prop::sample::select(vec![
            "idea",
            "active",
            "paused",
            "shipped",
            "dead",
            "hibernating",
        ]),
        safe_word(),
        prop::collection::vec(safe_word(), 0..4),
        prop::collection::vec(safe_word(), 0..3),
        any::<bool>(),
        0usize..3,
        prop::collection::vec((prop::sample::select(FUTURE_KEYS), safe_word()), 0..3),
        prop::collection::vec((prop::sample::select(FUTURE_KEYS), safe_word()), 0..2),
    )
        .prop_map(
            |(
                name,
                description,
                status,
                created,
                frameworks,
                services,
                frameworks_multiline,
                gap,
                unknown_top,
                unknown_nested,
            )| {
                ManifestSpec {
                    name,
                    description,
                    status: status.to_owned(),
                    created,
                    frameworks,
                    services,
                    frameworks_multiline,
                    gap,
                    // Duplicate keys are invalid TOML; dedupe by key.
                    unknown_top: dedupe(unknown_top),
                    unknown_nested: dedupe(unknown_nested),
                }
            },
        )
}

fn dedupe(pairs: Vec<(&'static str, String)>) -> Vec<(String, String)> {
    let mut seen = std::collections::BTreeSet::new();
    pairs
        .into_iter()
        .filter(|(k, _)| seen.insert(*k))
        .map(|(k, v)| (k.to_owned(), v))
        .collect()
}

fn edit_strategy() -> impl Strategy<Value = FieldEdit> {
    prop_oneof![
        safe_word().prop_map(|s| FieldEdit::SetDescription(Some(s))),
        Just(FieldEdit::SetDescription(None)),
        prop::sample::select(vec!["idea", "active", "paused", "shipped", "dead"])
            .prop_map(|s| FieldEdit::SetProjectStatus(s.parse::<Status>().unwrap())),
        any::<bool>().prop_map(FieldEdit::SetArchived),
        Just(FieldEdit::SetRepoVisibility(Visibility::Public)),
        prop::collection::vec(safe_word(), 0..4).prop_map(FieldEdit::ReplaceStackFrameworks),
        prop::collection::vec(safe_word(), 0..3).prop_map(FieldEdit::ReplaceStackServices),
        prop::collection::vec(safe_word(), 0..3).prop_map(FieldEdit::ReplaceDeployEnvRequired),
        prop::collection::vec(safe_word(), 0..3).prop_map(FieldEdit::ReplaceContextNext),
    ]
}

// --- the naive path, for contrast --------------------------------------

/// What this project would look like if it used `toml` + `serde` instead.
///
/// Deliberately shaped the obvious way: derive `Serialize`/`Deserialize` on the
/// domain struct and round-trip through it. `#[serde(default)]` everywhere, so
/// it is as forgiving as such a design gets — and it still loses everything.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct NaiveManifest {
    schema_version: String,
    project: NaiveProject,
    stack: NaiveStack,
    repo: NaiveRepo,
    deploy: NaiveDeploy,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct NaiveProject {
    name: String,
    description: String,
    status: String,
    created: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct NaiveStack {
    runtime: String,
    frameworks: Vec<String>,
    services: Vec<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct NaiveRepo {
    remote: String,
    visibility: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct NaiveDeploy {
    url: String,
    env_required: Vec<String>,
}

/// Parse and re-emit through the serde path, returning `None` if it cannot even
/// read the input (which would make it a vacuous comparison).
fn naive_round_trip(text: &str) -> Option<String> {
    let parsed: NaiveManifest = toml::from_str(text).ok()?;
    toml::to_string(&parsed).ok()
}

proptest! {
    /// Reading and re-rendering without editing anything must be byte-identical.
    #[test]
    fn unedited_round_trip_is_byte_identical(spec in spec_strategy()) {
        let text = spec.render();
        let doc = ManifestDocument::from_text(PATH, &text).expect("generated manifest is valid TOML");
        prop_assert_eq!(doc.render(), text);
    }

    /// Property 1: an edit disturbs nothing it did not address.
    #[test]
    fn edits_preserve_comments(spec in spec_strategy(), edit in edit_strategy()) {
        let text = spec.render();
        let mut doc = ManifestDocument::from_text(PATH, &text).unwrap();
        doc.apply(edit.clone()).expect("edit applies");
        let after = doc.render();

        for sentinel in SENTINELS {
            prop_assert!(
                after.contains(sentinel),
                "edit {:?} destroyed the comment {:?}\n--- after ---\n{}",
                edit, sentinel, after
            );
        }
    }

    /// Property 2: keys this build has never heard of survive an edit.
    #[test]
    fn edits_preserve_unknown_keys(spec in spec_strategy(), edit in edit_strategy()) {
        let text = spec.render();
        let mut doc = ManifestDocument::from_text(PATH, &text).unwrap();
        doc.apply(edit.clone()).expect("edit applies");
        let after = doc.render();

        for (key, value) in spec.unknown_top.iter().chain(spec.unknown_nested.iter()) {
            prop_assert!(
                after.contains(key.as_str()),
                "edit {:?} dropped unknown key `{}`\n--- after ---\n{}",
                edit, key, after
            );
            prop_assert!(
                after.contains(value.as_str()),
                "edit {:?} dropped the value of unknown key `{}`\n--- after ---\n{}",
                edit, key, after
            );
        }
    }

    /// The parsed projection reports the unknown keys rather than hiding them,
    /// so `vibe show` can tell the user what this build did not understand.
    #[test]
    fn unknown_keys_are_reported_not_swallowed(spec in spec_strategy()) {
        let text = spec.render();
        let doc = ManifestDocument::from_text(PATH, &text).unwrap();
        let manifest = doc.parse().expect("parses");

        for (key, _) in &spec.unknown_nested {
            let dotted = format!("project.{key}");
            prop_assert!(
                manifest.unknown.iter().any(|u| u.dotted_path == dotted),
                "`{}` should be reported as unknown, got {:?}",
                dotted, manifest.unknown
            );
        }
        if !spec.unknown_top.is_empty() {
            prop_assert!(
                manifest.unknown.iter().any(|u| u.dotted_path == "future"),
                "the unknown `[future]` table should be reported, got {:?}",
                manifest.unknown
            );
        }
    }
}

// --- negative controls -------------------------------------------------
//
// These assert the *naive* implementation fails. If one of them ever starts
// failing, it means the serde path stopped losing data — at which point the
// case for `toml_edit` needs re-examining, and this test failing is how we
// would find out.

proptest! {
    #[test]
    fn negative_control_naive_toml_destroys_comments(spec in spec_strategy()) {
        let text = spec.render();
        let Some(naive) = naive_round_trip(&text) else {
            return Ok(());
        };

        for sentinel in SENTINELS {
            prop_assert!(
                !naive.contains(sentinel),
                "the naive serde path was expected to lose {:?} but kept it:\n{}",
                sentinel, naive
            );
        }

        // And ours keeps every one of them, on the same input.
        let ours = ManifestDocument::from_text(PATH, &text).unwrap().render();
        for sentinel in SENTINELS {
            prop_assert!(ours.contains(sentinel), "ours lost {sentinel:?}");
        }
    }

    #[test]
    fn negative_control_naive_toml_destroys_unknown_keys(spec in spec_strategy()) {
        prop_assume!(!spec.unknown_top.is_empty() || !spec.unknown_nested.is_empty());

        let text = spec.render();
        let Some(naive) = naive_round_trip(&text) else {
            return Ok(());
        };

        for (key, _) in spec.unknown_top.iter().chain(spec.unknown_nested.iter()) {
            prop_assert!(
                !naive.contains(key.as_str()),
                "the naive serde path was expected to drop `{}` but kept it:\n{}",
                key, naive
            );
        }

        let ours = ManifestDocument::from_text(PATH, &text).unwrap().render();
        for (key, _) in spec.unknown_top.iter().chain(spec.unknown_nested.iter()) {
            prop_assert!(ours.contains(key.as_str()), "ours dropped `{key}`");
        }
    }
}

/// A worked example, fixed rather than generated, so a reader can see exactly
/// what the two paths do to the same file without decoding a proptest failure.
#[test]
fn worked_example_of_the_difference() {
    let original = r#"# my notes survive
schema_version = "1.0"

[project]
name = "macroring"      # the slug
status = "active"
created = "2026-03-12"

# a key from a build that does not exist yet
[deploy]
url = "https://macroring.vercel.app"
regions = ["fra1", "iad1"]
"#;

    let mut doc = ManifestDocument::from_text(PATH, original).unwrap();
    doc.apply(FieldEdit::SetProjectStatus(Status::Shipped))
        .unwrap();
    let ours = doc.render();

    assert!(ours.contains("# my notes survive"));
    assert!(ours.contains("# the slug"));
    assert!(ours.contains("# a key from a build that does not exist yet"));
    assert!(ours.contains("regions"));
    assert!(ours.contains(r#"status = "shipped""#));

    let naive = naive_round_trip(original).expect("naive path reads it");
    assert!(
        !naive.contains("# my notes survive"),
        "naive kept a comment?"
    );
    assert!(!naive.contains("regions"), "naive kept the unknown key?");
    assert!(
        naive.contains("macroring"),
        "naive should keep known fields"
    );
}
