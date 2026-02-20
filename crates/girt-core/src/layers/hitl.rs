use std::future::Future;
use std::pin::Pin;

use crate::decision::Decision;
use crate::error::DecisionError;
use crate::layers::DecisionLayer;
use crate::spec::GateInput;

/// HITL (Human-in-the-Loop) layer -- the final layer in the cascade.
///
/// When all automated layers fail to produce a confident decision, the HITL layer
/// surfaces the request to the user for a manual allow/deny decision.
///
/// In the Claude Code plugin context, this uses the native AskUserQuestion mechanism.
/// In standalone mode, it returns an Ask decision that the caller must resolve.
pub struct HitlLayer {
    responder: Box<dyn HitlResponder>,
}

/// Trait for HITL response mechanisms -- abstracted so we can mock in tests.
pub trait HitlResponder: Send + Sync {
    fn prompt<'a>(
        &'a self,
        input: &'a GateInput,
        context: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<HitlResponse, DecisionError>> + Send + 'a>>;
}

/// The response from a human-in-the-loop prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HitlResponse {
    Allow,
    Deny { reason: String },
}

/// Default HITL responder that returns an error (no responder configured).
///
/// In production, the proxy will handle ASK decisions by surfacing them
/// through the MCP protocol or Claude Code plugin.
pub struct DeferringResponder;

impl HitlResponder for DeferringResponder {
    fn prompt<'a>(
        &'a self,
        _input: &'a GateInput,
        _context: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<HitlResponse, DecisionError>> + Send + 'a>> {
        Box::pin(async move {
            Err(DecisionError::HitlError(
                "No HITL responder configured, defaulting to safe deny".into(),
            ))
        })
    }
}

impl HitlLayer {
    pub fn new(responder: Box<dyn HitlResponder>) -> Self {
        Self { responder }
    }

    pub fn with_default() -> Self {
        Self {
            responder: Box::new(DeferringResponder),
        }
    }
}

impl DecisionLayer for HitlLayer {
    fn name(&self) -> &str {
        "hitl"
    }

    fn evaluate<'a>(
        &'a self,
        input: &'a GateInput,
    ) -> Pin<Box<dyn Future<Output = Result<Option<Decision>, DecisionError>> + Send + 'a>> {
        Box::pin(async move {
            let context = match input {
                GateInput::Creation(spec) => {
                    format!("Capability request '{}': {}", spec.name, spec.description)
                }
                GateInput::Execution(req) => {
                    format!("Tool invocation '{}'", req.tool_name)
                }
            };

            match self.responder.prompt(input, &context).await {
                Ok(HitlResponse::Allow) => {
                    tracing::info!("HITL: user approved");
                    Ok(Some(Decision::Allow))
                }
                Ok(HitlResponse::Deny { reason }) => {
                    tracing::info!(reason = %reason, "HITL: user denied");
                    Ok(Some(Decision::Deny { reason }))
                }
                Err(e) => {
                    // No HITL responder or it failed: return Ask so the caller knows
                    // human input is needed
                    tracing::warn!(error = %e, "HITL responder unavailable");
                    Ok(Some(Decision::Ask {
                        prompt: context,
                        context: format!("HITL required: {e}"),
                    }))
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{CapabilityConstraints, CapabilitySpec};

    fn make_input() -> GateInput {
        GateInput::Creation(CapabilitySpec {
            name: "ambiguous_tool".into(),
            description: "Might be dangerous, might not".into(),
            inputs: serde_json::Value::Null,
            outputs: serde_json::Value::Null,
            constraints: CapabilityConstraints::default(),
        })
    }

    #[tokio::test]
    async fn default_responder_returns_ask() {
        let layer = HitlLayer::with_default();
        let input = make_input();

        let result = layer.evaluate(&input).await.unwrap();
        assert!(matches!(result, Some(Decision::Ask { .. })));
    }

    struct AlwaysAllowResponder;
    impl HitlResponder for AlwaysAllowResponder {
        fn prompt<'a>(
            &'a self,
            _input: &'a GateInput,
            _context: &'a str,
        ) -> Pin<Box<dyn Future<Output = Result<HitlResponse, DecisionError>> + Send + 'a>>
        {
            Box::pin(async move { Ok(HitlResponse::Allow) })
        }
    }

    #[tokio::test]
    async fn allow_responder_returns_allow() {
        let layer = HitlLayer::new(Box::new(AlwaysAllowResponder));
        let input = make_input();

        let result = layer.evaluate(&input).await.unwrap();
        assert!(matches!(result, Some(Decision::Allow)));
    }

    struct AlwaysDenyResponder;
    impl HitlResponder for AlwaysDenyResponder {
        fn prompt<'a>(
            &'a self,
            _input: &'a GateInput,
            _context: &'a str,
        ) -> Pin<Box<dyn Future<Output = Result<HitlResponse, DecisionError>> + Send + 'a>>
        {
            Box::pin(async move {
                Ok(HitlResponse::Deny {
                    reason: "user said no".into(),
                })
            })
        }
    }

    #[tokio::test]
    async fn deny_responder_returns_deny() {
        let layer = HitlLayer::new(Box::new(AlwaysDenyResponder));
        let input = make_input();

        let result = layer.evaluate(&input).await.unwrap();
        assert!(matches!(result, Some(Decision::Deny { .. })));
    }
}
