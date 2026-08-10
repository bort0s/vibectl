use std::process::ExitCode;

/// The exit-code contract from ADR-0002 §3.
///
/// The distinction that matters is `Partial`: a script must be able to tell
/// "the registry was read, one project was unreadable" from "the registry
/// failed". Collapsing both into `1` makes a partially-readable registry
/// indistinguishable from a broken one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    /// Everything requested succeeded.
    Success,
    /// The command produced no useful result.
    Failure,
    /// Results were produced, but at least one entry could not be read.
    ///
    /// Read commands only. A write is all-or-nothing at the decision level — a
    /// `WritePlan` either applied or did not — so no write command can produce
    /// this. Unreachable until `list`/`show`/`scan` land in P2–P3; defined now
    /// because retrofitting an exit code after scripts exist is a breaking
    /// change to a contract nobody wrote down.
    Partial,
}

impl From<Exit> for ExitCode {
    fn from(e: Exit) -> ExitCode {
        match e {
            Exit::Success => ExitCode::from(0),
            Exit::Failure => ExitCode::from(1),
            Exit::Partial => ExitCode::from(2),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_match_the_documented_contract() {
        // Guarding the numbers themselves: these are a public contract for
        // scripts, and a renumbering would be silent everywhere else.
        assert_eq!(
            format!("{:?}", ExitCode::from(Exit::Success)),
            format!("{:?}", ExitCode::from(0u8))
        );
        assert_eq!(
            format!("{:?}", ExitCode::from(Exit::Failure)),
            format!("{:?}", ExitCode::from(1u8))
        );
        assert_eq!(
            format!("{:?}", ExitCode::from(Exit::Partial)),
            format!("{:?}", ExitCode::from(2u8))
        );
    }
}
