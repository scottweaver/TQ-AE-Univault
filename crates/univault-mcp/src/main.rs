//! Read-only MCP server exposing Titan Quest game data over stdio.
//!
//! The client (Claude Code, Claude Desktop, any MCP client) spawns
//! this binary as a child process and speaks JSON-RPC on its
//! stdin/stdout. Every tool call re-reads the underlying files — the
//! game or the GUI may write at any time, so nothing here is
//! authoritative and nothing is ever written back
//! (see ARCHITECTURE.md, "MCP surface").

use rmcp::ServiceExt;
use rmcp::transport::stdio;

mod server;
mod view;
mod world;

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let service = server::Univault::from_env().serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
