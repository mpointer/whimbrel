//! Connectors (M3.3 Task 7): the Node MCP servers Whimbrel runs to read a Google Calendar.
//!
//! Each call spawns `node <connector>/dist/index.js`, hands it the account's access token
//! in an environment variable, makes one tool call and disconnects — the Mac's shape, and
//! the shape `core/mcp-host` supports. Nothing about the connector protocol lives here:
//! the request and the decoding are `api::connectors::calendar`'s.

pub mod accounts;
pub mod calendar;
pub mod gmail;
pub mod node;
pub mod write;
