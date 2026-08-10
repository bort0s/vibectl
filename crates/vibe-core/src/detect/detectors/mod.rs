//! The fixed v1 detector set.
//!
//! No plugin loading, no dynamic libraries, no user-defined detectors. That is
//! a real limitation — someone will want Deno or Elixir on day two — and the
//! mitigation is that the trait is small enough that adding one is a file plus
//! a fixture directory. A declarative detector format is the v2 conversation
//! (ADR-0003 §7).

// These lints are the supporting half of the honesty enforcement: the type
// system stops a value existing without evidence, and these stop the
// shortcuts that would otherwise let a detector fabricate one under pressure.
#![deny(clippy::unwrap_used, clippy::expect_used)]

pub mod deploy;
pub mod stack;
pub mod vcs;

use super::Detector;

pub(super) fn builtin() -> Vec<Box<dyn Detector>> {
    vec![
        Box::new(stack::NodePackageJson),
        Box::new(stack::NodeLockfile),
        Box::new(stack::CargoToml),
        Box::new(stack::PyProject),
        Box::new(stack::PyRequirements),
        Box::new(stack::GoMod),
        Box::new(stack::ComposerJsonDetector),
        Box::new(vcs::GitRepo),
        Box::new(deploy::VercelConfig),
        Box::new(deploy::NetlifyConfig),
        Box::new(deploy::FlyConfig),
        Box::new(deploy::EnvExample),
    ]
}
