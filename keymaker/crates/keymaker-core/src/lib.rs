//! keymaker — capability-mediated secret use for AI agents.
//!
//! The agent acts; it never holds. Three mechanisms, applied together:
//!
//! 1. **Confine the agent** ([`jail`]) so it cannot read the secret store.
//! 2. **Broker every use** ([`handle`], [`provider`], [`policy`]) so a value
//!    is never injected into anything the agent controls.
//! 3. **Expire what is handed out** ([`creds`]) so a stolen value dies.
//!
//! Everything here is a library first. The CLI and the GUI are both callers.

pub mod approvals;
pub mod audit;
pub mod broker;
pub mod clock;
pub mod creds;
pub mod error;
pub mod gui;
pub mod handle;
pub mod id;
pub mod jail;
pub mod landlock;
pub mod manifest;
pub mod mcp;
pub mod peer;
pub mod policy;
pub mod protocol;
pub mod provider;
pub mod redact;
pub mod runner;
pub mod scan;
pub mod server;
pub mod sigv4;
pub mod store;
pub mod transport;

pub use error::{Error, HandleError, Result};
