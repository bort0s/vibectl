//! Git state, read by shelling out.
//!
//! No `libgit2`. `git` already exists, already handles every repository layout
//! anyone has, and is not our code to get wrong.
//!
//! Every invocation passes `--no-optional-locks`, which stops git taking the
//! index lock or refreshing the index as a side effect of a read. That is a
//! small speed win and a correctness one: `scan` must not modify the
//! repository it is reading, and the tests in `scan_never_writes.rs` compare
//! `.git` before and after.
//!
//! `branch` and `dirty` are deliberately **not** detected, a deviation from
//! ADR-0003 §7. Neither appears in the manifest schema nor in the columns
//! `vibe list` shows, so neither has a consumer in v1 — and both are expensive:
//! `git status` walks the working tree, and every way of getting the branch
//! costs either a third subprocess or a ref-count-dependent format. They can
//! return the day a command actually displays them.
//!
//! The original note on `dirty`:
//! `git status` is the most expensive call we could make — it walks the working
//! tree — and the flag appears in neither the manifest schema nor the columns
//! `vibe list` shows, so it was pure cost. It can return the day a command
//! actually displays it.
//!
//! A repository with no remote and no commits is the case that matters here: it
//! must produce an explicit *nothing*, not a plausible-looking guess. A brand
//! new `git init` has no branch commit, no remote, and no last-commit date, and
//! all three come back as unknown with the reason attached.

use crate::detect::merge::text_finding;
use crate::detect::{
    DetectCtx, DetectError, Detector, DetectorId, Evidence, FieldPath, Finding, Interest,
    Specificity,
};
use crate::model::Confidence;

#[derive(Debug)]
pub struct GitRepo;

const GIT_ID: DetectorId = DetectorId("vcs.git");

impl Detector for GitRepo {
    fn id(&self) -> DetectorId {
        GIT_ID
    }

    fn interest(&self) -> &'static [Interest] {
        &[Interest::DirName(".git")]
    }

    fn produces(&self) -> &'static [FieldPath] {
        &[FieldPath::RepoRemote, FieldPath::GitLastCommit]
    }

    fn detect(&self, ctx: &DetectCtx<'_>) -> Result<Vec<Finding>, DetectError> {
        let mut out = Vec::new();

        // Two failures that look alike and are not:
        //
        // - `git` ran and said no (exit non-zero). The fact does not exist —
        //   a fresh `git init` genuinely has no remote and no HEAD.
        // - `git` could not run at all. We know nothing, and reporting that as
        //   "this repository has no remote" would be a claim about the user's
        //   repository that we have no basis for.
        //
        // The first is swallowed per-command below. The second propagates, so
        // the merge pass reports `NotAttempted` against every field this
        // detector would have produced.
        let remote = ctx.git(&["--no-optional-locks", "remote", "get-url", "origin"]);
        if let Err(e @ (DetectError::NotAttempted { .. } | DetectError::Timeout)) = &remote {
            return Err(e.clone());
        }

        if let Ok(o) = remote {
            if o.success() && !o.trimmed().is_empty() {
                let url = normalize_remote(o.trimmed());
                out.push(text_finding(
                    FieldPath::RepoRemote,
                    url,
                    Confidence::Certain,
                    Specificity::Manifest,
                    Evidence::from_command(&o.argv, o.trimmed()),
                    GIT_ID,
                ));
            }
        }

        // `--format=%cI` and nothing else. An earlier version folded branch
        // detection in with `%cI%n%D` to save one ~17ms spawn — a bad trade
        // that took two attempts to see. `%D` is the ref *decoration*, so git
        // must load and match every ref in the repository to produce it:
        //
        //     refs      %cI      %cI%n%D
        //        0    44 ms        37 ms
        //     2000    44 ms       439 ms
        //   50000     44 ms      4567 ms   (packed; 7354 ms loose)
        //
        // Measured on this machine. A repository reaches thousands of refs by
        // having a few hundred remote branches, since `git fetch` creates one
        // ref per branch. At 50k refs a single project cost more than twice the
        // whole 2s budget for fifty. The flat call was replaced by one that
        // grows with something the user controls and we do not.
        //
        // `--no-optional-locks` does not help here; it was measured too.
        let log = ctx.git(&["--no-optional-locks", "log", "-1", "--format=%cI"]);

        // Propagated, not swallowed. `if let Ok(o)` here discarded a timeout,
        // and the field then merged as `no_evidence` — "we looked and there is
        // nothing to say" — when what happened was "we ran out of time". That
        // is exactly the substitution ADR-0003 §8 exists to forbid, and the
        // `%D` cost above is what made it reachable on a real repository.
        if let Err(e @ (DetectError::NotAttempted { .. } | DetectError::Timeout)) = &log {
            return Err(e.clone());
        }

        if let Ok(o) = log {
            if o.success() && !o.trimmed().is_empty() {
                out.push(text_finding(
                    FieldPath::GitLastCommit,
                    o.trimmed(),
                    Confidence::Certain,
                    Specificity::Manifest,
                    Evidence::from_command(&o.argv, o.trimmed()),
                    GIT_ID,
                ));
            }
        }

        Ok(out)
    }
}

/// `git@github.com:user/repo.git` and `https://github.com/user/repo.git` both
/// become `github.com/user/repo`.
///
/// The manifest example in the spec uses that host/path form, and normalising
/// here means two clones of the same repository over different transports do
/// not read as different remotes.
fn normalize_remote(url: &str) -> String {
    let url = url.trim();
    let stripped = url
        .strip_prefix("git@")
        .map(|rest| rest.replacen(':', "/", 1))
        .or_else(|| url.strip_prefix("ssh://git@").map(str::to_owned))
        .or_else(|| url.strip_prefix("https://").map(str::to_owned))
        .or_else(|| url.strip_prefix("http://").map(str::to_owned))
        .unwrap_or_else(|| url.to_owned());

    stripped
        .trim_end_matches('/')
        .trim_end_matches(".git")
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remotes_normalise_to_host_and_path_whatever_the_transport() {
        for input in [
            "git@github.com:bort0s/vibectl.git",
            "https://github.com/bort0s/vibectl.git",
            "https://github.com/bort0s/vibectl",
            "http://github.com/bort0s/vibectl.git",
        ] {
            assert_eq!(
                normalize_remote(input),
                "github.com/bort0s/vibectl",
                "failed for {input}"
            );
        }
    }

    #[test]
    fn a_self_hosted_remote_is_left_recognisable() {
        assert_eq!(
            normalize_remote("git@git.example.internal:team/thing.git"),
            "git.example.internal/team/thing"
        );
    }
}
