use crate::cache::ToolCache;
use crate::error::PipelineError;
use crate::types::BuildArtifact;

/// Publishes build artifacts to local cache and (eventually) OCI registries.
pub struct Publisher {
    cache: ToolCache,
}

/// Result of publishing an artifact.
#[derive(Debug)]
pub struct PublishResult {
    pub tool_name: String,
    pub local_path: std::path::PathBuf,
    pub oci_reference: Option<String>,
}

impl Publisher {
    pub fn new(cache: ToolCache) -> Self {
        Self { cache }
    }

    /// Initialize the publisher (creates cache directory).
    pub async fn init(&self) -> Result<(), PipelineError> {
        self.cache.init().await
    }

    /// Publish a build artifact.
    ///
    /// Currently stores locally. OCI push will be added when registry
    /// integration is implemented.
    pub async fn publish(&self, artifact: &BuildArtifact) -> Result<PublishResult, PipelineError> {
        let tool_name = artifact.spec.name.clone();

        // Store in local cache
        let local_path = self.cache.store(artifact).await?;

        tracing::info!(
            tool = %tool_name,
            path = %local_path.display(),
            "Artifact published to local cache"
        );

        // OCI publishing stub -- will be implemented in Phase 5
        let oci_reference = None;
        if oci_reference.is_some() {
            tracing::info!(tool = %tool_name, "Artifact pushed to OCI registry");
        }

        Ok(PublishResult {
            tool_name,
            local_path,
            oci_reference,
        })
    }

    /// Get the underlying cache for lookups.
    pub fn cache(&self) -> &ToolCache {
        &self.cache
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{BuildOutput, QaResult, RefinedSpec, SecurityResult, SpecAction};
    use girt_core::spec::{CapabilityConstraints, CapabilitySpec};
    use tempfile::TempDir;

    fn make_artifact() -> BuildArtifact {
        let spec = CapabilitySpec {
            name: "published_tool".into(),
            description: "A published tool".into(),
            inputs: serde_json::Value::Null,
            outputs: serde_json::Value::Null,
            constraints: CapabilityConstraints::default(),
        };

        BuildArtifact {
            spec: spec.clone(),
            refined_spec: RefinedSpec {
                action: SpecAction::Build,
                spec,
                design_notes: "test".into(),
                extend_target: None,
                extend_features: None,
            },
            build_output: BuildOutput {
                source_code: "fn main() {}".into(),
                wit_definition: "package test:tool;".into(),
                policy_yaml: "version: \"1.0\"".into(),
                language: "rust".into(),
            },
            qa_result: QaResult {
                passed: true,
                tests_run: 5,
                tests_passed: 5,
                tests_failed: 0,
                bug_tickets: vec![],
            },
            security_result: SecurityResult {
                passed: true,
                exploits_attempted: 6,
                exploits_succeeded: 0,
                bug_tickets: vec![],
            },
            build_iterations: 1,
        }
    }

    #[tokio::test]
    async fn publishes_to_local_cache() {
        let tmp = TempDir::new().unwrap();
        let cache = ToolCache::new(tmp.path().to_path_buf());
        let publisher = Publisher::new(cache);
        publisher.init().await.unwrap();

        let artifact = make_artifact();
        let result = publisher.publish(&artifact).await.unwrap();

        assert_eq!(result.tool_name, "published_tool");
        assert!(result.local_path.exists());
        assert!(result.oci_reference.is_none());

        // Verify we can look it up via cache
        let cached = publisher.cache().get("published_tool").await.unwrap();
        assert!(cached.is_some());
    }
}
