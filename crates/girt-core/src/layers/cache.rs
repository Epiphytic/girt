use std::collections::HashMap;
use tokio::sync::RwLock;

use crate::decision::Decision;
use crate::error::DecisionError;
use crate::layers::DecisionLayer;
use crate::spec::GateInput;

/// Decision cache layer — caches previous decisions by spec/request hash.
///
/// A previously-denied spec with the same hash is auto-denied.
/// A previously-allowed spec skips to the build pipeline.
/// DEFER decisions are cached with a pointer to the deferred-to tool.
pub struct CacheLayer {
    entries: RwLock<HashMap<String, Decision>>,
}

impl CacheLayer {
    pub fn new() -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
        }
    }

    /// Store a decision in the cache.
    pub async fn store(&self, hash: String, decision: Decision) {
        let mut entries = self.entries.write().await;
        entries.insert(hash, decision);
    }

    /// Remove a decision from the cache.
    pub async fn invalidate(&self, hash: &str) {
        let mut entries = self.entries.write().await;
        entries.remove(hash);
    }

    /// Number of cached entries.
    pub async fn len(&self) -> usize {
        let entries = self.entries.read().await;
        entries.len()
    }

    /// Whether the cache is empty.
    pub async fn is_empty(&self) -> bool {
        let entries = self.entries.read().await;
        entries.is_empty()
    }
}

impl Default for CacheLayer {
    fn default() -> Self {
        Self::new()
    }
}

impl DecisionLayer for CacheLayer {
    fn name(&self) -> &str {
        "cache"
    }

    fn evaluate<'a>(
        &'a self,
        input: &'a GateInput,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Option<Decision>, DecisionError>> + Send + 'a>,
    > {
        Box::pin(async move {
            let hash = input.hash();
            let entries = self.entries.read().await;

            if let Some(cached) = entries.get(&hash) {
                tracing::info!(
                    hash = %hash,
                    decision = ?cached,
                    "Cache hit"
                );
                return Ok(Some(cached.clone()));
            }

            Ok(None)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{CapabilityConstraints, CapabilitySpec};

    fn make_spec(name: &str) -> (GateInput, String) {
        let spec = CapabilitySpec {
            name: name.into(),
            description: "test".into(),
            inputs: serde_json::Value::Null,
            outputs: serde_json::Value::Null,
            constraints: CapabilityConstraints::default(),
        };
        let hash = spec.spec_hash();
        (GateInput::Creation(spec), hash)
    }

    #[tokio::test]
    async fn cache_miss_returns_none() {
        let cache = CacheLayer::new();
        let (input, _) = make_spec("unknown_tool");

        let result = cache.evaluate(&input).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn cache_hit_returns_stored_decision() {
        let cache = CacheLayer::new();
        let (input, hash) = make_spec("cached_tool");

        cache
            .store(
                hash,
                Decision::Deny {
                    reason: "previously denied".into(),
                },
            )
            .await;

        let result = cache.evaluate(&input).await.unwrap();
        assert!(matches!(result, Some(Decision::Deny { .. })));
    }

    #[tokio::test]
    async fn cache_invalidation_clears_entry() {
        let cache = CacheLayer::new();
        let (input, hash) = make_spec("invalidated_tool");

        cache.store(hash.clone(), Decision::Allow).await;
        assert_eq!(cache.len().await, 1);

        cache.invalidate(&hash).await;
        assert_eq!(cache.len().await, 0);

        let result = cache.evaluate(&input).await.unwrap();
        assert!(result.is_none());
    }
}
