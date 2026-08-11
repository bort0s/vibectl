//! The template environment.
//!
//! `minijinja` with templates compiled in via `include_str!` and **no
//! filesystem loader configured**. A loader would make template resolution
//! depend on the working directory, turning "render this project" into "render
//! this project with whatever templates happen to be lying next to it" — an
//! execution-adjacent surprise in a tool that goes to some length elsewhere to
//! make its behaviour independent of ambient state. User-supplied templates are
//! a v2 conversation and would need their own containment answer (ADR-0007 §7).

use std::fmt;
use std::str::FromStr;

use serde::Serialize;

use crate::error::CoreError;
use crate::model::Manifest;

/// What `vibe render` can produce.
///
/// A closed set, for the same reason `GitOp` is closed: the alternative is a
/// free-form target name that resolves to a path, and a path a user names is a
/// write primitive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RenderTarget {
    ClaudeMd,
    AgentsMd,
    ReadmeMd,
}

impl RenderTarget {
    /// Every target, for `--help` and for tests that must cover all of them.
    pub const ALL: &'static [RenderTarget] = &[
        RenderTarget::ClaudeMd,
        RenderTarget::AgentsMd,
        RenderTarget::ReadmeMd,
    ];

    /// The file name this target writes, relative to the project directory.
    ///
    /// A fixed string per variant, never derived from user input.
    #[must_use]
    pub fn file_name(self) -> &'static str {
        match self {
            RenderTarget::ClaudeMd => "CLAUDE.md",
            RenderTarget::AgentsMd => "AGENTS.md",
            RenderTarget::ReadmeMd => "README.md",
        }
    }

    fn template(self) -> &'static str {
        match self {
            RenderTarget::ClaudeMd => include_str!("templates/claude.md.j2"),
            RenderTarget::AgentsMd => include_str!("templates/agents.md.j2"),
            RenderTarget::ReadmeMd => include_str!("templates/readme.md.j2"),
        }
    }
}

impl fmt::Display for RenderTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.file_name())
    }
}

impl FromStr for RenderTarget {
    type Err = ();

    /// Accepts the short name or the file name, case-insensitively.
    fn from_str(s: &str) -> Result<Self, ()> {
        match s.trim().to_ascii_lowercase().as_str() {
            "claude" | "claude.md" => Ok(RenderTarget::ClaudeMd),
            "agents" | "agents.md" => Ok(RenderTarget::AgentsMd),
            "readme" | "readme.md" => Ok(RenderTarget::ReadmeMd),
            _ => Err(()),
        }
    }
}

/// Render one target's body. **Without** the marker — [`super::marker::wrap`]
/// adds that, so the hash is always taken over exactly what is written.
pub fn render_body(manifest: &Manifest, target: RenderTarget) -> Result<String, CoreError> {
    let mut env = minijinja::Environment::new();
    // Whitespace control on, so a `{% for %}` does not leave a ragged blank
    // line in a file a human reads.
    env.set_trim_blocks(true);
    env.set_lstrip_blocks(true);

    let name = target.file_name();
    env.add_template(name, target.template())
        .and_then(|()| {
            env.get_template(name)?
                .render(minijinja::context! { m => manifest })
        })
        .map(|out| tidy(&out))
        .map_err(|e| CoreError::RenderFailed {
            target: name,
            why: e.to_string(),
        })
}

/// Collapse runs of blank lines and guarantee exactly one trailing newline.
///
/// Templates driven by optional fields produce ragged output — three blank
/// lines where two sections were both empty. Doing this here rather than in
/// each template means every target gets it and no template has to remember.
fn tidy(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut blanks = 0usize;
    for line in s.lines() {
        if line.trim().is_empty() {
            blanks += 1;
            if blanks > 1 {
                continue;
            }
        } else {
            blanks = 0;
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    while out.ends_with("\n\n") {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ManifestDocument;

    fn manifest(text: &str) -> Manifest {
        ManifestDocument::from_text("/p/.vibe/project.toml", text)
            .unwrap()
            .parse()
            .unwrap()
    }

    const FULL: &str = r#"schema_version = "1.1"

[project]
name = "macroring"
description = "Mobile-first PWA for nutrition tracking"
status = "active"
created = "2026-03-12"

[stack]
runtime = "node@22"
frameworks = ["react@19", "vite"]
services = ["supabase"]

[repo]
remote = "github.com/user/macroring"

[deploy]
url = "https://macroring.vercel.app"
env_required = ["SUPABASE_URL"]

[context]
decisions = ["iOS-native design system"]
next = ["validate draggable sheet on device"]

[agents]
installed = ["engineering-code-reviewer"]
"#;

    /// The manifest a real half-finished project actually has: a name, and
    /// almost nothing else. Every template must survive it.
    const SPARSE: &str = "[project]\nname = \"stub\"\n";

    #[test]
    fn every_target_renders_a_full_manifest() {
        let m = manifest(FULL);
        for target in RenderTarget::ALL {
            let out = render_body(&m, *target).expect("renders");
            assert!(out.contains("macroring"), "{target}: {out}");
            assert!(out.ends_with('\n'), "{target} must end with one newline");
            assert!(!out.contains("\n\n\n"), "{target} has a ragged gap:\n{out}");
        }
    }

    /// The honesty rule, at the render boundary: a field nothing detected must
    /// not appear as an empty heading or an invented value.
    #[test]
    fn every_target_renders_a_sparse_manifest_without_inventing_anything() {
        let m = manifest(SPARSE);
        for target in RenderTarget::ALL {
            let out = render_body(&m, *target).expect("renders");
            assert!(out.contains("stub"), "{target}: {out}");
            // Rust `Debug` leakage: these can only arrive by a template
            // interpolating an `Option` or a struct directly.
            for ghost in ["None", "null", "Some(", "undefined"] {
                assert!(
                    !out.contains(ghost),
                    "{target} leaked a placeholder {ghost:?}:\n{out}"
                );
            }
            // A dash is checked by *position*, not by presence: `—` is ordinary
            // punctuation in the footer prose, and banning the character
            // outright made this test fail on a sentence rather than on a
            // missing value. What must not happen is a dash standing in for a
            // value — a bullet or a label with nothing after it.
            for line in out.lines() {
                let t = line.trim();
                assert!(
                    !(t == "—" || t.ends_with(": —") || t.ends_with(':') || t == "- —"),
                    "{target} rendered an empty value on `{line}`:\n{out}"
                );
            }
            assert!(!out.contains("\n\n\n"), "{target} has a ragged gap:\n{out}");
        }
    }

    #[test]
    fn rendering_is_deterministic() {
        let m = manifest(FULL);
        for target in RenderTarget::ALL {
            assert_eq!(
                render_body(&m, *target).unwrap(),
                render_body(&m, *target).unwrap()
            );
        }
    }

    #[test]
    fn target_names_parse_from_both_forms_and_reject_anything_else() {
        assert_eq!("claude".parse(), Ok(RenderTarget::ClaudeMd));
        assert_eq!("CLAUDE.md".parse(), Ok(RenderTarget::ClaudeMd));
        assert_eq!("readme".parse(), Ok(RenderTarget::ReadmeMd));
        assert_eq!("agents.md".parse(), Ok(RenderTarget::AgentsMd));
        // Case is not meaning: `CLAUDE` and `claude` name the same target, and
        // rejecting one of them would be strictness with nothing behind it.
        assert_eq!("CLAUDE".parse(), Ok(RenderTarget::ClaudeMd));

        // Not a path, and not a near-match: the set is closed, so a target
        // name can never become a place to write.
        for bad in [
            "../../etc/passwd",
            "notes.md",
            "",
            "claude.md.j2",
            "CLAUDE.md.bak",
            "claude ",
        ] {
            let parsed = bad.trim().parse::<RenderTarget>();
            assert!(
                bad.trim() == "claude" || parsed.is_err(),
                "{bad:?} was accepted"
            );
        }
        assert!("../../etc/passwd".parse::<RenderTarget>().is_err());
    }

    #[test]
    fn tidy_collapses_gaps_and_pins_one_trailing_newline() {
        assert_eq!(tidy("a\n\n\n\nb\n\n\n"), "a\n\nb\n");
        assert_eq!(tidy("a"), "a\n");
        assert_eq!(tidy("a  \n"), "a\n");
        assert_eq!(tidy(""), "");
    }
}
