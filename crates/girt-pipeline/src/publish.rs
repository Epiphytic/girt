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

    pub async fn publish_with_wasm(
        &self,
        artifact: &BuildArtifact,
        wasm_path: &std::path::Path,
    ) -> Result<PublishResult, PipelineError> {
        let tool_name = artifact.spec.name.clone();

        let local_path = self.cache.store(artifact).await?;

        let cached_wasm = local_path.join("tool.wasm");
        tokio::fs::copy(wasm_path, &cached_wasm).await?;

        tracing::info!(
            tool = %tool_name,
            path = %local_path.display(),
            "Artifact published to local cache with WASM binary"
        );

        Ok(PublishResult {
            tool_name,
            local_path,
            oci_reference: None,
        })
    }

    pub async fn push_oci(
        &self,
        artifact: &BuildArtifact,
        wasm_path: &std::path::Path,
        registry_url: &str,
        tag: &str,
    ) -> Result<String, PipelineError> {
        let tool_name = &artifact.spec.name;
        let reference = format!("{}/{}:{}", registry_url, tool_name, tag);

        let cache_dir = self.cache.base_dir().join(tool_name);
        let manifest_path = cache_dir.join("manifest.json");
        let policy_path = cache_dir.join("policy.yaml");

        for path in [wasm_path, manifest_path.as_path(), policy_path.as_path()] {
            if !path.exists() {
                return Err(PipelineError::PublishError(format!(
                    "Required file missing: {}",
                    path.display()
                )));
            }
        }

        let output = tokio::process::Command::new("oras")
            .arg("push")
            .arg(&reference)
            .arg(format!(
                "{}:application/vnd.wasm.component.layer.v0+wasm",
                wasm_path.display()
            ))
            .arg(format!(
                "{}:application/vnd.girt.policy.v1+yaml",
                policy_path.display()
            ))
            .arg(format!(
                "{}:application/vnd.girt.manifest.v1+json",
                manifest_path.display()
            ))
            .output()
            .await
            .map_err(|e| {
                PipelineError::PublishError(format!(
                    "Failed to run oras: {e}. Is it installed?"
                ))
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(PipelineError::PublishError(format!(
                "oras push failed: {stderr}"
            )));
        }

        tracing::info!(tool = %tool_name, reference = %reference, "Pushed to OCI registry");
        Ok(reference)
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
                complexity_hint: None,
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
            timings: Default::default(),
        }
    }

    #[tokio::test]
    async fn publish_full_stores_wasm_in_cache() {
        let tmp = TempDir::new().unwrap();
        let cache = ToolCache::new(tmp.path().join("tools"));
        let publisher = Publisher::new(cache);
        publisher.init().await.unwrap();

        let artifact = make_artifact();

        let wasm_dir = tmp.path().join("build");
        std::fs::create_dir_all(&wasm_dir).unwrap();
        let wasm_path = wasm_dir.join("published_tool.wasm");
        std::fs::write(&wasm_path, b"fake wasm bytes").unwrap();

        let result = publisher
            .publish_with_wasm(&artifact, &wasm_path)
            .await
            .unwrap();

        assert_eq!(result.tool_name, "published_tool");
        assert!(result.local_path.join("tool.wasm").exists());
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
