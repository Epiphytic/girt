use anyhow::Result;
use clap::Parser;
use rmcp::{
    ServiceExt,
    transport::{ConfigureCommandExt, TokioChildProcess},
};
use tokio::process::Command;
use tracing_subscriber::{EnvFilter, fmt};

mod proxy;

use proxy::GirtProxy;

#[derive(Parser)]
#[command(name = "girt", about = "GIRT MCP Proxy — routes agent requests to Wassette")]
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

    // Spawn Wassette as a child process and connect as MCP client
    let wassette_transport = TokioChildProcess::new(
        Command::new(&cli.wassette_bin).configure(|cmd| {
            cmd.args(&cli.wassette_args);
        }),
    )?;

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

    // Create proxy handler that forwards to Wassette
    let proxy = GirtProxy::new(wassette_peer, wassette_init);

    // Serve on stdio (agent connects here)
    let stdio = rmcp::transport::io::stdio();
    let server = proxy.serve(stdio).await?;

    tracing::info!("GIRT proxy serving on stdio");

    // Run until the agent disconnects
    server.waiting().await?;

    tracing::info!("GIRT proxy shutting down");
    Ok(())
}
