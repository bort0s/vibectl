//! Agent management: the store, the lockfile, and ownership.
//!
//! The premise everything follows from: **`.claude/agents/` is not ours.** It
//! will hold hand-written agents and agents installed by other tools, so `vibe`
//! may only touch what it put there. [`lock`] is the record of what that is.
//!
//! Agents are **opaque payload**. We read only enough frontmatter to identify
//! one and never interpret the body — Agency Agents is pre-1.0, and anything
//! deeper means every upstream release breaks us.
//!
//! See ADR-0006.

pub mod lock;
mod state;

pub use lock::{LOCK_VERSION, LockLoad, LockedAgent, Lockfile, content_hash, hash_is_verifiable};
pub use state::{AgentReport, AgentState, InstalledAgent, StoreAgent, compute};
