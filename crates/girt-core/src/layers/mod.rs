pub mod cache;
pub mod cli_check;
pub mod hitl;
pub mod llm;
pub mod policy;
pub mod registry;
pub mod similarity;

use std::future::Future;
use std::pin::Pin;

use crate::decision::Decision;
use crate::error::DecisionError;
use crate::spec::GateInput;

/// A single layer in the decision cascade.
///
/// Each layer examines the input and either returns a decision (short-circuiting
/// the cascade) or returns `None` to pass through to the next layer.
pub trait DecisionLayer: Send + Sync {
    /// The display name of this layer (for logging).
    fn name(&self) -> &str;

    /// Evaluate the input. Returns `Some(decision)` to short-circuit, or `None` to pass through.
    fn evaluate<'a>(
        &'a self,
        input: &'a GateInput,
    ) -> Pin<Box<dyn Future<Output = Result<Option<Decision>, DecisionError>> + Send + 'a>>;
}
