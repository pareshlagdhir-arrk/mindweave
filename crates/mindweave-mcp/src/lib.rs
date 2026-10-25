//! Mindweave's MCP server: lets AI agents (in any MCP client) create,
//! read and edit `.mindweave` documents and look at them as pictures.
//!
//! The server speaks the Model Context Protocol over stdio (see
//! [`server`]); its tools (see [`tools`]) call the command layer of
//! `mindweave-core`, one file operation per call.

pub mod args;
pub mod pictures;
pub mod render;
pub mod server;
pub mod tools;

pub use server::Server;
pub use tools::Context;

#[cfg(test)]
mod tests;
