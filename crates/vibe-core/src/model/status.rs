use std::fmt;
use std::str::FromStr;

use serde::Serialize;

/// A project's lifecycle judgement.
///
/// `Dead` means *abandoned before completion*. It is deliberately not a synonym
/// for archived: archival is a separate, orthogonal flag on
/// [`crate::model::Project`], so a `Shipped` project can be filed away without
/// being relabelled `Dead`. See ADR-0002 §6.
///
/// [`Status::Other`] exists so a value written by a future build round-trips
/// verbatim instead of being rewritten or rejected (ADR-0002 §5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(into = "String")]
pub enum Status {
    Idea,
    Active,
    Paused,
    Shipped,
    Dead,
    /// A value this build does not recognise. Displayed verbatim, never
    /// rewritten except by an explicit status change.
    Other(String),
}

impl Status {
    /// The values this build understands, for CLI help and validation.
    pub const KNOWN: [&'static str; 5] = ["idea", "active", "paused", "shipped", "dead"];

    /// Whether this value was recognised by this build.
    #[must_use]
    pub fn is_known(&self) -> bool {
        !matches!(self, Status::Other(_))
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Status::Idea => "idea",
            Status::Active => "active",
            Status::Paused => "paused",
            Status::Shipped => "shipped",
            Status::Dead => "dead",
            Status::Other(other) => other.as_str(),
        };
        f.write_str(s)
    }
}

impl FromStr for Status {
    /// Parsing never fails: an unrecognised value becomes [`Status::Other`].
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "idea" => Status::Idea,
            "active" => Status::Active,
            "paused" => Status::Paused,
            "shipped" => Status::Shipped,
            "dead" => Status::Dead,
            other => Status::Other(other.to_owned()),
        })
    }
}

impl From<Status> for String {
    fn from(s: Status) -> String {
        s.to_string()
    }
}

/// Repository visibility, with the same forward-compatibility escape hatch as
/// [`Status`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(into = "String")]
pub enum Visibility {
    Public,
    Private,
    Other(String),
}

impl fmt::Display for Visibility {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Visibility::Public => "public",
            Visibility::Private => "private",
            Visibility::Other(other) => other.as_str(),
        };
        f.write_str(s)
    }
}

impl FromStr for Visibility {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "public" => Visibility::Public,
            "private" => Visibility::Private,
            other => Visibility::Other(other.to_owned()),
        })
    }
}

impl From<Visibility> for String {
    fn from(v: Visibility) -> String {
        v.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_status_round_trips_verbatim() {
        let s: Status = "hibernating".parse().unwrap();
        assert_eq!(s, Status::Other("hibernating".to_owned()));
        assert_eq!(s.to_string(), "hibernating");
        assert!(!s.is_known());
    }

    #[test]
    fn every_known_status_round_trips() {
        for name in Status::KNOWN {
            let parsed: Status = name.parse().unwrap();
            assert!(parsed.is_known(), "{name} should be recognised");
            assert_eq!(parsed.to_string(), name);
        }
    }
}
