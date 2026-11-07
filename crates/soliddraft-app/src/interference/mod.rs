//! Interference Detection and Clearance Verification in the window and over MCP. What a
//! check proves and refuses is `soliddraft-assembly`'s (see `docs/interference.md`); this
//! module holds the commands, the tools and the Interference panel that lists results.

pub(crate) mod panel;
pub(crate) mod registry;
pub(crate) mod tools;

pub use panel::{Draft, InterferenceAction, Mode, apply};
