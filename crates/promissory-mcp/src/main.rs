//! `promissory-mcp`: MCP server over stdio. stdout carries the protocol; diagnostics go to stderr.

use anyhow::{anyhow, Result};
use clap::Parser;
use rmcp::ServiceExt;
use promissory_mcp::{
    app::{App, DEFAULT_APP},
    server::SoMcp,
};

#[derive(Parser)]
#[command(name = "promissory-mcp", about = "Promissory MCP server for AI agents (stdio)")]
struct Args {
    /// promissory-app base URL; must be on this machine
    #[arg(long, default_value = DEFAULT_APP)]
    app: String,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let args = Args::parse();
    let app = App::new(&args.app).map_err(|e| anyhow!(e))?;
    eprintln!("promissory-mcp: serving on stdio, promissory-app at {}", app.base());
    let running = SoMcp::new(app).serve(rmcp::transport::stdio()).await?;
    running.waiting().await?;
    Ok(())
}
