use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
    .with_writer(std::io::stderr)   // IMPORTANT: logs must go to stderr,
    // stdout is reserved for MCP JSON-RPC.
    .with_env_filter(
        tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "info".into()),
    )
    .init();

    satya_mcp::serve_stdio().await
}
