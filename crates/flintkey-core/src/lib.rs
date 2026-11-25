//! The core of the Flintkey launcher, independent of any rendering engine.
//!
//! Everything here runs without a window: reading and writing the config folder, deciding what a
//! command launches, resolving secrets, and ranking search results. The desktop app is a thin layer on top.

pub mod alfred;
pub mod command;
pub mod diagnostic;
pub mod launch;
pub mod paths;
pub mod search;
pub mod secrets;
pub mod settings;
pub mod store;
pub mod usage;

mod atomic;
mod document;

pub use command::{Action, Command, Os, parse_command};
pub use diagnostic::Diagnostic;
