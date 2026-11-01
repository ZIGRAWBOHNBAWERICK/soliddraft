//! Forming a sub-assembly from the open assembly's parts, in the window and over MCP.
//! What forming means, what it keeps and what it refuses are `soliddraft-assembly`'s (see
//! `docs/subassemblies.md`); this module holds the command, the panel the tree's context
//! menu opens, and what forming does to the session the window shows: the selection
//! follows the parts into the new file.

pub(crate) mod panel;
pub(crate) mod registry;
pub(crate) mod tools;

pub use panel::{Draft, FormAction, apply};
