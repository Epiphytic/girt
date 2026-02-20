use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use girt_core::engine::DecisionEngine;
use girt_pipeline::cache::ToolCache;
use girt_pipeline::config::GirtConfig;
use girt_pipeline::publish::Publisher;
use girt_runtime::LifecycleManager;
use rmcp::ServiceExt;
use tracing_subscriber::{EnvFilter, fmt};

mod evaluator;
mod proxy;

use evaluator::GateLlmEvaluator;
use proxy::GirtProxy;

#[derive(Parser)]
#[command(
    name = "girt",
    about = "GIRT MCP Proxy -- routes agent requests through the Hookwise decision engine"
)]
struct Cli {
    /// Path to girt.toml config file
    #[arg(long, default_value = "girt.toml")]
    config: PathBuf,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Logs go to stderr — stdout is reserved for MCP stdio transport
    fmt()
        .with_env_filter(EnvFilter::from_env("GIRT_LOG"))
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();

    tracing::info!(config = %cli.config.display(), "Starting GIRT MCP proxy");

    // Load config
    let config = GirtConfig::from_file(&cli.config)
        .with_context(|| format!("Failed to load config from {}", cli.config.display()))?;
    tracing::info!(
        provider = ?config.llm.provider,
        model = %config.llm.model,
        "Config loaded"
    );

    // Initialize LLM client from config
    let llm = config
        .build_llm_client()
        .context("Failed to initialize LLM client")?;
    tracing::info!("LLM client initialized");

    // Initialize the Hookwise decision engine with real LLM evaluators
    // Both gates share the same underlying client via Arc
    let engine = Arc::new(DecisionEngine::with_real_llm(
        Box::new(GateLlmEvaluator::new(Arc::clone(&llm))),
        Box::new(GateLlmEvaluator::new(Arc::clone(&llm))),
    ));
    tracing::info!("Decision engine initialized with real LLM evaluator");

    // Initialize tool cache and publisher
    let cache = ToolCache::new(ToolCache::default_path());
    cache.init().await?;
    let publisher = Arc::new(Publisher::new(cache));
    tracing::info!("Tool cache initialized");

    // Initialize girt-runtime (ADR-010)
    let runtime = Arc::new(
        LifecycleManager::new(None).context("Failed to initialize girt-runtime")?,
    );
    // Restore components built in previous sessions
    runtime.load_persisted().await;
    tracing::info!("girt-runtime initialized");

    // Create proxy handler
    let proxy = GirtProxy::new(engine, llm, publisher, runtime);

    // Serve on stdio (agent connects here)
    let stdio = rmcp::transport::io::stdio();
    let server = proxy.serve(stdio).await?;

    tracing::info!("GIRT proxy serving on stdio");

    server.waiting().await?;

    tracing::info!("GIRT proxy shutting down");
    Ok(())
}
