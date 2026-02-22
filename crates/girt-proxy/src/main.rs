use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use girt_core::engine::DecisionEngine;
use girt_pipeline::cache::ToolCache;
use girt_pipeline::config::GirtConfig;
use girt_pipeline::publish::Publisher;
use girt_runtime::LifecycleManager;
use girt_secrets::{AnthropicOAuthStore, OAuthMode};
use rmcp::ServiceExt;
use tracing_subscriber::{EnvFilter, fmt};

mod approval;
mod evaluator;
mod proxy;

use approval::ApprovalManager;
use evaluator::GateLlmEvaluator;
use girt_pipeline::tool_sync::ToolSync;
use proxy::GirtProxy;

// ── CLI ───────────────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(
    name = "girt",
    about = "GIRT — Generative Isolated Runtime for Tools",
    long_about = "GIRT MCP Proxy. Routes agent tool requests through the Hookwise decision \
                  engine, builds WASM components on demand, and manages their lifecycle.\n\n\
                  Running without a subcommand starts the MCP proxy server (stdio transport)."
)]
struct Cli {
    /// Path to girt.toml config file.
    /// Default search order: ./girt.toml → ~/.config/girt/girt.toml
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Run the MCP proxy server on stdio (default when no subcommand is given).
    Serve,
    /// Manage Anthropic OAuth credentials.
    Auth {
        #[command(subcommand)]
        action: AuthCommand,
    },
}

#[derive(Subcommand)]
enum AuthCommand {
    /// Authenticate with Anthropic via OAuth 2.0 (PKCE).
    ///
    /// Opens a browser authorization URL. After authorizing, paste the
    /// `code#state` response string at the prompt.
    Login {
        /// Use Console mode to create an API key instead of a Max subscription token.
        #[arg(long)]
        console: bool,
    },
    /// Show the status of stored credentials.
    Status,
    /// Remove stored credentials.
    Logout,
}

// ── Entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    // Logs go to stderr — stdout is reserved for MCP stdio transport.
    fmt()
        .with_env_filter(EnvFilter::from_env("GIRT_LOG"))
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();

    match cli.command {
        None | Some(Command::Serve) => run_serve(cli.config).await,
        Some(Command::Auth { action }) => run_auth(action).await,
    }
}

// ── Serve ─────────────────────────────────────────────────────────────────────

/// Run the MCP proxy server on stdio.
async fn run_serve(config_flag: Option<PathBuf>) -> Result<()> {
    let config_path = resolve_config(config_flag).context("Failed to locate girt.toml")?;

    tracing::info!(config = %config_path.display(), "Starting GIRT MCP proxy");

    // Load config
    let config = GirtConfig::from_file(&config_path)
        .with_context(|| format!("Failed to load config from {}", config_path.display()))?;
    tracing::info!(
        provider = ?config.llm.provider,
        model = %config.llm.model,
        "Config loaded"
    );

    // Inject GIRT OAuth token into env if present (and ANTHROPIC_API_KEY not already set).
    // This slots into AnthropicLlmClient::from_env_or()'s first-priority check without
    // requiring any API change to girt-pipeline.
    inject_oauth_token_if_needed().await;

    // Inject Discord bot token for the approval WASM, if not already set.
    if std::env::var("DISCORD_BOT_TOKEN").is_err() {
        if let Some(token) = read_openclaw_discord_token() {
            tracing::info!("Injecting Discord bot token from OpenClaw config into DISCORD_BOT_TOKEN");
            // Safety: single-threaded at startup; no other threads spawned yet.
            unsafe { std::env::set_var("DISCORD_BOT_TOKEN", token) };
        } else {
            tracing::debug!("No Discord token in OpenClaw config — DISCORD_BOT_TOKEN not set");
        }
    }

    // Initialize LLM client from config
    let llm = config
        .build_llm_client()
        .context("Failed to initialize LLM client")?;
    tracing::info!("LLM client initialized");

    // Load optional coding standards (injected into Engineer's system prompt)
    let coding_standards = config.load_coding_standards();

    // Initialize the Hookwise decision engine.
    let engine = Arc::new(match config.security.creation_gate.as_str() {
        "policy_only" => {
            tracing::warn!(
                "Creation Gate in POLICY-ONLY mode — LLM/HITL approval bypassed. \
                 Bootstrap use only. Switch back to 'llm' after the approval WASM is built."
            );
            DecisionEngine::with_policy_only_creation(Box::new(GateLlmEvaluator::new(
                Arc::clone(&llm),
            )))
        }
        _ => DecisionEngine::with_real_llm(
            Box::new(GateLlmEvaluator::new(Arc::clone(&llm))),
            Box::new(GateLlmEvaluator::new(Arc::clone(&llm))),
        ),
    });
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

    // Build the approval manager if [approval] is configured in girt.toml
    let approval_manager: Option<Arc<ApprovalManager>> =
        if let Some(approval_cfg) = config.approval {
            match ApprovalManager::new(Arc::clone(&runtime), approval_cfg.clone()) {
                Ok(manager) => {
                    tracing::info!(
                        component = %approval_cfg.component,
                        channel_id = %approval_cfg.channel_id,
                        overall_timeout_secs = approval_cfg.overall_timeout_secs,
                        "Approval manager initialized — Ask decisions routed to discord_approval WASM"
                    );
                    Some(Arc::new(manager))
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "Failed to initialize approval manager — Ask decisions will surface to MCP caller"
                    );
                    None
                }
            }
        } else {
            tracing::info!("No [approval] config — Ask decisions will surface to MCP caller");
            None
        };

    // Build optional tool-source sync
    let tool_sync: Option<Arc<ToolSync>> =
        if let Some(ref repo_url) = config.registry.source_repo {
            let local_path = config.registry.source_repo_local
                .as_deref()
                .map(|p| {
                    if p.starts_with('~') {
                        dirs::home_dir()
                            .unwrap_or_else(|| std::path::PathBuf::from("."))
                            .join(&p[2..])
                    } else {
                        std::path::PathBuf::from(p)
                    }
                })
                .unwrap_or_else(ToolSync::default_local_path);

            tracing::info!(
                repo = %repo_url,
                local = %local_path.display(),
                "Tool source sync enabled"
            );
            Some(Arc::new(ToolSync::new(repo_url, local_path)))
        } else {
            tracing::info!("No registry.source_repo configured — tool source sync disabled");
            None
        };

    // Create proxy handler
    let proxy = GirtProxy::new(engine, llm, publisher, runtime, coding_standards, config.pipeline.max_iterations, config.pipeline.on_circuit_breaker, config.build.cargo_component_bin, approval_manager, tool_sync);

    // Serve on stdio (agent connects here)
    let stdio = rmcp::transport::io::stdio();
    let server = proxy.serve(stdio).await?;

    tracing::info!("GIRT proxy serving on stdio");

    server.waiting().await?;

    tracing::info!("GIRT proxy shutting down");
    Ok(())
}

/// Read the Discord bot token from the OpenClaw config file.
///
/// OpenClaw stores it at `~/.openclaw/openclaw.json → channels.discord.token`.
/// Returns `None` if the file is missing, malformed, or the key is absent.
fn read_openclaw_discord_token() -> Option<String> {
    let config_path = dirs::home_dir()?.join(".openclaw").join("openclaw.json");
    let content = std::fs::read_to_string(&config_path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&content).ok()?;
    json.get("channels")?
        .get("discord")?
        .get("token")?
        .as_str()
        .map(|s| s.to_string())
}

/// Check `AnthropicOAuthStore` and, if it holds a valid token and
/// `ANTHROPIC_API_KEY` is not already set, inject it into the process environment.
///
/// This lets `AnthropicLlmClient::from_env_or()` pick up the GIRT OAuth token
/// at its highest-priority check (env var) without modifying `girt-pipeline`.
async fn inject_oauth_token_if_needed() {
    // Don't overwrite an explicit env var.
    if std::env::var("ANTHROPIC_API_KEY").is_ok() {
        return;
    }

    let store = AnthropicOAuthStore::new();
    match store.get_valid_token().await {
        Ok(Some(token)) => {
            tracing::info!("Injecting GIRT OAuth token into ANTHROPIC_API_KEY");
            // Safety: single-threaded at this point in startup; no other threads yet.
            unsafe { std::env::set_var("ANTHROPIC_API_KEY", token) };
        }
        Ok(None) => {
            tracing::debug!("No GIRT OAuth token stored — skipping injection");
        }
        Err(e) => {
            tracing::warn!(error = %e, "Failed to read GIRT OAuth token — continuing without it");
        }
    }
}

// ── Auth subcommands ──────────────────────────────────────────────────────────

async fn run_auth(action: AuthCommand) -> Result<()> {
    let store = AnthropicOAuthStore::new();

    match action {
        AuthCommand::Login { console } => {
            let mode = if console {
                OAuthMode::Console
            } else {
                OAuthMode::Max
            };
            run_auth_login(&store, mode).await
        }
        AuthCommand::Status => run_auth_status(&store).await,
        AuthCommand::Logout => run_auth_logout(&store),
    }
}

async fn run_auth_login(store: &AnthropicOAuthStore, mode: OAuthMode) -> Result<()> {
    let mode_name = match mode {
        OAuthMode::Max => "Claude Max/Pro subscription",
        OAuthMode::Console => "Anthropic Console (API key creation)",
    };

    eprintln!("Starting Anthropic OAuth login ({mode_name})...\n");

    let flow = AnthropicOAuthStore::start_login_flow(mode)
        .context("Failed to start OAuth flow")?;

    eprintln!("1. Open this URL in your browser:\n");
    eprintln!("   {}\n", flow.authorization_url);
    eprintln!("2. Authorize the application.");
    eprintln!("3. You will receive a response in the format:  code#state");
    eprintln!("   Paste the full response below and press Enter:\n");

    let mut response = String::new();
    std::io::stdin()
        .read_line(&mut response)
        .context("Failed to read response from stdin")?;
    let response = response.trim();

    if response.is_empty() {
        anyhow::bail!("No response provided. Login cancelled.");
    }

    store
        .complete_login(response, &flow)
        .await
        .context("Failed to exchange code for tokens")?;

    eprintln!("\n✓ Authenticated successfully. Credentials saved to ~/.config/girt/auth.json");
    Ok(())
}

async fn run_auth_status(store: &AnthropicOAuthStore) -> Result<()> {
    match store.status().await.context("Failed to read credentials")? {
        None => {
            eprintln!("Not logged in. Run `girt auth login` to authenticate.");
        }
        Some(status) => {
            let state = if status.is_expired {
                "⚠ expired (will auto-refresh on next use)"
            } else {
                "✓ valid"
            };
            eprintln!("Status: {state}");
            eprintln!("Token:  {}…", status.access_token_prefix);
            let expires = chrono_from_unix(status.expires_at_unix);
            eprintln!("Expiry: {expires}");
            eprintln!(
                "Refresh token: {}",
                if status.has_refresh_token { "stored" } else { "not stored" }
            );
        }
    }
    Ok(())
}

fn run_auth_logout(store: &AnthropicOAuthStore) -> Result<()> {
    store.logout().context("Failed to remove credentials")?;
    eprintln!("✓ Credentials removed.");
    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Resolve config path using standard search order:
/// 1. Explicit --config flag
/// 2. ./girt.toml (relative to cwd)
/// 3. ~/.config/girt/girt.toml (user-level installation)
fn resolve_config(explicit: Option<PathBuf>) -> anyhow::Result<PathBuf> {
    if let Some(p) = explicit {
        return Ok(p);
    }
    let local = PathBuf::from("girt.toml");
    if local.exists() {
        return Ok(local);
    }
    if let Some(home) = dirs::home_dir() {
        let user_config = home.join(".config").join("girt").join("girt.toml");
        if user_config.exists() {
            return Ok(user_config);
        }
    }
    anyhow::bail!(
        "No girt.toml found. Looked in: ./girt.toml and ~/.config/girt/girt.toml\n\
         Run from the girt repo directory, or copy girt.toml to ~/.config/girt/girt.toml"
    )
}

/// Format a Unix timestamp for human display (no chrono dep needed — manual).
fn chrono_from_unix(unix_secs: u64) -> String {
    use std::time::{Duration, UNIX_EPOCH};
    let dt = UNIX_EPOCH + Duration::from_secs(unix_secs);
    match dt.duration_since(std::time::SystemTime::now()) {
        Ok(remaining) => {
            let mins = remaining.as_secs() / 60;
            if mins < 60 {
                format!("in {mins} minutes")
            } else {
                let hours = mins / 60;
                format!("in {hours}h {}m", mins % 60)
            }
        }
        Err(_) => "already expired".to_string(),
    }
}
