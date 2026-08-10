use std::fmt;
use std::str::FromStr;

use serde::Serialize;

/// The major component this build understands. Only this is compared when
/// deciding whether a manifest is readable at all.
pub const SCHEMA_MAJOR: u16 = 1;
/// The minor component this build understands. Compared only to decide whether
/// to warn.
pub const SCHEMA_MINOR: u16 = 0;

/// `major.minor`, stored in the manifest as a **quoted string**.
///
/// Not a TOML float. `schema_version = 1.10` would parse as `1.1` and order
/// before `1.9`, which is a silent correctness bug with a two-year fuse. Not a
/// bare integer either: a single number cannot distinguish "there is more here
/// than you know about" (additive, safe) from "what you think you know is
/// wrong" (breaking), and conflating them forces every additive field to break
/// every deployed binary. See ADR-0002 §1–§2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(into = "String")]
pub struct SchemaVersion {
    pub major: u16,
    pub minor: u16,
}

impl SchemaVersion {
    /// The version this build writes.
    pub const CURRENT: SchemaVersion = SchemaVersion {
        major: SCHEMA_MAJOR,
        minor: SCHEMA_MINOR,
    };

    #[must_use]
    pub const fn new(major: u16, minor: u16) -> Self {
        Self { major, minor }
    }

    /// How this build should treat a manifest declaring this version.
    #[must_use]
    pub fn compat(self) -> Compat {
        if self.major != SCHEMA_MAJOR {
            Compat::MajorMismatch
        } else if self.minor > SCHEMA_MINOR {
            Compat::MinorNewer
        } else {
            Compat::Supported
        }
    }
}

/// The three cases from ADR-0002 §3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Compat {
    /// Same major, minor at or below ours.
    Supported,
    /// Same major, higher minor. Proceed, ignore unrecognised keys, warn once.
    MinorNewer,
    /// Different major. Refuse *this manifest* — not the whole command.
    MajorMismatch,
}

impl Compat {
    /// Whether a command that produces a file may act on this manifest.
    #[must_use]
    pub fn is_writable(self) -> bool {
        matches!(self, Compat::Supported | Compat::MinorNewer)
    }
}

impl Default for SchemaVersion {
    /// A manifest with no `schema_version` is treated as `1.0`, not as an
    /// error — hand-written manifests must work (ADR-0002 §4).
    fn default() -> Self {
        SchemaVersion::new(1, 0)
    }
}

impl fmt::Display for SchemaVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

impl From<SchemaVersion> for String {
    fn from(v: SchemaVersion) -> String {
        v.to_string()
    }
}

/// Why a `schema_version` string could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseSchemaVersionError {
    pub found: String,
}

impl fmt::Display for ParseSchemaVersionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "expected a `major.minor` version string, found `{}`",
            self.found
        )
    }
}

impl std::error::Error for ParseSchemaVersionError {}

impl FromStr for SchemaVersion {
    type Err = ParseSchemaVersionError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = || ParseSchemaVersionError {
            found: s.to_owned(),
        };
        let (major, minor) = s.split_once('.').ok_or_else(err)?;
        // Reject anything with a third component rather than silently ignoring
        // it: `1.0.3` means the author believed in a patch level we do not have,
        // and quietly reading it as `1.0` would hide that disagreement.
        if minor.contains('.') {
            return Err(err());
        }
        Ok(SchemaVersion {
            major: major.trim().parse().map_err(|_| err())?,
            minor: minor.trim().parse().map_err(|_| err())?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_major_minor() {
        assert_eq!(
            "1.0".parse::<SchemaVersion>().unwrap(),
            SchemaVersion::new(1, 0)
        );
        assert_eq!(
            "2.14".parse::<SchemaVersion>().unwrap(),
            SchemaVersion::new(2, 14)
        );
    }

    #[test]
    fn rejects_shapes_that_are_not_major_minor() {
        for bad in ["1", "1.0.3", "x.y", "1.", ".1", "", "-1.0"] {
            assert!(
                bad.parse::<SchemaVersion>().is_err(),
                "`{bad}` should not parse"
            );
        }
    }

    /// The reason this type is a string and not a float. As a TOML float,
    /// `1.10` is `1.1`, which orders *below* `1.9` — so a build supporting 1.9
    /// would happily accept a 1.10 manifest as older than itself.
    #[test]
    fn minor_ten_outranks_minor_nine() {
        let ten = "1.10".parse::<SchemaVersion>().unwrap();
        let nine = "1.9".parse::<SchemaVersion>().unwrap();
        assert!(ten > nine, "1.10 must order above 1.9");
        assert_eq!(ten.minor, 10);

        // The float reading this design avoids:
        assert!("1.10".parse::<f64>().unwrap() < "1.9".parse::<f64>().unwrap());
    }

    #[test]
    fn compat_refuses_only_on_major() {
        assert_eq!(SchemaVersion::new(1, 0).compat(), Compat::Supported);
        assert_eq!(SchemaVersion::new(1, 99).compat(), Compat::MinorNewer);
        assert_eq!(SchemaVersion::new(2, 0).compat(), Compat::MajorMismatch);
        assert_eq!(SchemaVersion::new(0, 9).compat(), Compat::MajorMismatch);

        assert!(Compat::MinorNewer.is_writable());
        assert!(!Compat::MajorMismatch.is_writable());
    }
}
