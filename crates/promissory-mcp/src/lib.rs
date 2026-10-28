//! `promissory-mcp`: a Model Context Protocol server (stdio) that lets an AI agent use Promissory
//! allowances (docs/mcp.md).
//!
//! It talks only to promissory-app's local payer API and holds no keys. Read-only tools show the
//! agent's allowance and the payer's orders; the two tools that would spend or change anything
//! only record a pending request, which the payer approves or rejects in the web UI.

pub mod app;
pub mod server;
pub mod tools;
