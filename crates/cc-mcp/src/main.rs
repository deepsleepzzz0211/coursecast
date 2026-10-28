//! cc-mcp: MCP server shell over cc-core (ticket 11, PRD §7). stdio transport via
//! rmcp; the 16 tools of PRD §7.2 are thin wrappers over the same cc-core calls the
//! CLI uses — same JSON, same files, same state machine (spec story 29: AI 通过
//! shell 调 CLI 一样能完成全流程，MCP 与之同源). Configuration per PRD §7.4:
//!
//! ```json
//! { "mcpServers": { "coursecast": {
//!     "command": "cc-mcp", "args": ["--data-dir", "D:/coursecast/data"] } } }
//! ```

mod paths;
mod server;
mod tools;

use clap::Parser;
use rmcp::ServiceExt;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "cc-mcp", about = "课影 CourseCast MCP server (stdio)")]
struct Args {
    /// Data directory; projects live under `data-dir/projects/project_id/`
    #[arg(long, global = true)]
    data_dir: Option<PathBuf>,
}

// stdio serving is single-connection work; the current-thread runtime is enough
#[tokio::main(flavor = "current_thread")]
async fn main() {
    cc_core::runtime::prepare();
    let args = Args::parse();
    let data_dir = args.data_dir.unwrap_or_else(|| PathBuf::from("data"));
    let service = server::CourseCastMcp::new(data_dir)
        .serve((tokio::io::stdin(), tokio::io::stdout()))
        .await
        .expect("cc-mcp: stdio transport failed to start");
    service
        .waiting()
        .await
        .expect("cc-mcp: service loop failed");
}
