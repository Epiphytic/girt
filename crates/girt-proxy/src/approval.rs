//! Human-in-the-loop approval via the discord_approval WASM component.
//!
//! `ApprovalManager` drives the continue-signal loop: the discord_approval WASM
//! runs for up to `timeout_secs` per invocation and returns `{status: "pending",
//! message_id: "..."}` if no human has responded yet.  The manager re-invokes
//! with the resume token until it receives `approved` / `denied` or the overall
//! deadline expires.

use std::sync::Arc;
use std::time::{Duration, Instant};

use girt_pipeline::config::ApprovalConfig;
use girt_runtime::LifecycleManager;

/// Outcome of a completed approval request.
pub struct ApprovalResult {
    /// Whether the human approved (`true`) or denied (`false`).
    pub approved: bool,
    /// Username (or "timeout") of the person who responded.
    pub authorized_by: String,
    /// Discord permalink to the approval message.
    pub evidence_url: String,
}

/// Wraps `LifecycleManager` and runs the discord_approval WASM in a loop until
/// a terminal decision arrives or the overall deadline expires.
pub struct ApprovalManager {
    runtime: Arc<LifecycleManager>,
    config: ApprovalConfig,
    /// Resolved bot token (from `config.bot_token` or `$DISCORD_BOT_TOKEN`).
    bot_token: String,
}

impl ApprovalManager {
    /// Create a new `ApprovalManager`.
    ///
    /// Fails if no bot token is available (neither `config.bot_token` nor the
    /// `DISCORD_BOT_TOKEN` environment variable is set).
    pub fn new(runtime: Arc<LifecycleManager>, config: ApprovalConfig) -> Result<Self, String> {
        let bot_token = config
            .bot_token
            .clone()
            .or_else(|| std::env::var("DISCORD_BOT_TOKEN").ok())
            .ok_or_else(|| {
                "No Discord bot token: set DISCORD_BOT_TOKEN env var or \
                 approval.bot_token in girt.toml"
                    .to_string()
            })?;

        Ok(Self { runtime, config, bot_token })
    }

    /// Post an approval request to Discord and poll until a human responds.
    ///
    /// `question` is shown prominently in the Discord embed.
    /// `context` is additional JSON context (tool spec summary).
    ///
    /// Implements the continue-signal pattern: the WASM runs for at most
    /// `config.timeout_secs` per invocation and returns `pending` if no
    /// response has arrived.  We loop, passing the `message_id` resume token,
    /// until `approved`, `denied`, or `config.overall_timeout_secs` elapses.
    pub async fn request_approval(
        &self,
        question: &str,
        context: &str,
    ) -> Result<ApprovalResult, String> {
        let deadline = Instant::now() + Duration::from_secs(self.config.overall_timeout_secs);
        let mut message_id: Option<String> = None;
        let mut first_call = true;

        loop {
            if Instant::now() >= deadline {
                return Err(format!(
                    "Approval timed out after {}s with no human response",
                    self.config.overall_timeout_secs
                ));
            }

            // Build input JSON for this invocation
            let mut input = serde_json::json!({
                "question": question,
                "context": context,
                "channel_id": self.config.channel_id,
                "guild_id": self.config.guild_id,
                "bot_token": self.bot_token,
                "authorized_users": self.config.authorized_users,
                "timeout_secs": self.config.timeout_secs,
            });

            // On re-invocation pass the resume token so the WASM skips re-posting
            if let Some(ref mid) = message_id {
                input["message_id"] = serde_json::json!(mid);
            }

            tracing::info!(
                component = %self.config.component,
                first_call,
                message_id = ?message_id,
                "Invoking approval WASM"
            );

            match self.runtime.call_tool(&self.config.component, &input).await {
                Ok(result) => {
                    let status = result["status"].as_str().unwrap_or("pending");
                    tracing::info!(status, component = %self.config.component, "Approval WASM response");

                    match status {
                        "approved" => {
                            return Ok(ApprovalResult {
                                approved: true,
                                authorized_by: result["authorized_by"]
                                    .as_str()
                                    .unwrap_or("unknown")
                                    .to_string(),
                                evidence_url: result["evidence_url"]
                                    .as_str()
                                    .unwrap_or("")
                                    .to_string(),
                            });
                        }
                        "denied" => {
                            return Ok(ApprovalResult {
                                approved: false,
                                authorized_by: result["authorized_by"]
                                    .as_str()
                                    .unwrap_or("unknown")
                                    .to_string(),
                                evidence_url: result["evidence_url"]
                                    .as_str()
                                    .unwrap_or("")
                                    .to_string(),
                            });
                        }
                        "pending" => {
                            // Extract or retain the resume token
                            if let Some(mid) = result["message_id"].as_str() {
                                if !mid.is_empty() {
                                    message_id = Some(mid.to_string());
                                }
                            }
                            first_call = false;
                            // Brief pause before re-invoking to avoid hammering
                            tokio::time::sleep(Duration::from_secs(2)).await;
                        }
                        other => {
                            return Err(format!(
                                "Approval WASM returned unexpected status: {other:?}"
                            ));
                        }
                    }
                }
                Err(e) => {
                    return Err(format!(
                        "Approval WASM invocation failed ({}): {e}",
                        self.config.component
                    ));
                }
            }
        }
    }
}
