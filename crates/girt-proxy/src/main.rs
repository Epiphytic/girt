use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use girt_core::engine::DecisionEngine;
use girt_pipeline::cache::ToolCache;
use girt_pipeline::llm::StubLlmClient;
use girt_pipeline::publish::Publisher;
use rmcp::{
    ServiceExt,
    transport::{ConfigureCommandExt, TokioChildProcess},
};
use tokio::process::Command;
use tracing_subscriber::{EnvFilter, fmt};

mod proxy;

use proxy::GirtProxy;

#[derive(Parser)]
#[command(
    name = "girt",
    about = "GIRT MCP Proxy -- routes agent requests through decision gates to Wassette"
)]
struct Cli {
    /// Path to the Wassette binary
    #[arg(long, default_value = "wassette")]
    wassette_bin: String,

    /// Arguments to pass to Wassette
    #[arg(long, num_args = 0..)]
    wassette_args: Vec<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize structured logging
    // Logs go to stderr so they don't interfere with MCP stdio transport on stdout
    fmt()
        .with_env_filter(EnvFilter::from_env("GIRT_LOG"))
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();

    tracing::info!(
        wassette_bin = %cli.wassette_bin,
        "Starting GIRT MCP proxy"
    );

    // Initialize the Hookwise decision engine with default layers
    let engine = Arc::new(DecisionEngine::with_defaults());
    tracing::info!("Decision engine initialized");

    // Initialize LLM client
    // TODO: Replace with real Anthropic client when anthropic feature is enabled
    let llm: Arc<dyn girt_pipeline::llm::LlmClient> = Arc::new(StubLlmClient::constant(
        r#"{"error": "LLM not configured. Set ANTHROPIC_API_KEY to enable builds."}"#,
    ));
    tracing::info!("LLM client initialized (stub mode)");

    // Initialize tool cache and publisher
    let cache = ToolCache::new(ToolCache::default_path());
    cache.init().await?;
    let publisher = Arc::new(Publisher::new(cache));
    tracing::info!("Tool cache initialized");

    // Spawn Wassette as a child process and connect as MCP client
    let wassette_transport =
        TokioChildProcess::new(Command::new(&cli.wassette_bin).configure(|cmd| {
            cmd.args(&cli.wassette_args);
        }))?;

    let wassette_service = ().serve(wassette_transport).await?;

    let wassette_init = wassette_service
        .peer_info()
        .cloned()
        .expect("Wassette should return server info on initialize");
    let wassette_peer = wassette_service.peer().clone();

    tracing::info!(
        server = ?wassette_init.server_info,
        "Connected to Wassette"
    );

    // Create proxy handler with decision engine and pipeline
    let proxy = GirtProxy::new(wassette_peer, wassette_init, engine, llm, publisher);

    // Serve on stdio (agent connects here)
    let stdio = rmcp::transport::io::stdio();
    let server = proxy.serve(stdio).await?;

    tracing::info!("GIRT proxy serving on stdio");

    // Run until the agent disconnects
    server.waiting().await?;

    tracing::info!("GIRT proxy shutting down");
    Ok(())
}
