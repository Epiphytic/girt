use std::path::{Path, PathBuf};

use crate::error::PipelineError;
use crate::types::BuildArtifact;

/// Local cache for built WASM tools.
///
/// Directory layout:
/// ```text
/// base_dir/
///   <tool_name>/
///     manifest.json   -- BuildArtifact metadata
///     source.rs       -- generated source code
///     policy.yaml     -- Wassette policy
/// ```
pub struct ToolCache {
    base_dir: PathBuf,
}

impl ToolCache {
    pub fn new(base_dir: PathBuf) -> Self {
        Self { base_dir }
    }

    /// Default cache location: ~/.girt/tools/
    pub fn default_path() -> PathBuf {
        dirs_path().join("tools")
    }

    /// Initialize the cache directory.
    pub async fn init(&self) -> Result<(), PipelineError> {
        tokio::fs::create_dir_all(&self.base_dir).await?;
        Ok(())
    }

    /// Store a build artifact in the cache.
    pub async fn store(&self, artifact: &BuildArtifact) -> Result<PathBuf, PipelineError> {
        let tool_dir = self.base_dir.join(&artifact.spec.name);
        tokio::fs::create_dir_all(&tool_dir).await?;

        // Write manifest (full artifact metadata)
        let manifest_path = tool_dir.join("manifest.json");
        let manifest_json = serde_json::to_string_pretty(artifact)?;
        tokio::fs::write(&manifest_path, manifest_json).await?;

        // Write source code
        let source_path = tool_dir.join("source.rs");
        tokio::fs::write(&source_path, &artifact.build_output.source_code).await?;

        // Write policy.yaml
        let policy_path = tool_dir.join("policy.yaml");
        tokio::fs::write(&policy_path, &artifact.build_output.policy_yaml).await?;

        // Write WIT definition
        if !artifact.build_output.wit_definition.is_empty() {
            let wit_path = tool_dir.join("world.wit");
            tokio::fs::write(&wit_path, &artifact.build_output.wit_definition).await?;
        }

        tracing::info!(
            tool = %artifact.spec.name,
            path = %tool_dir.display(),
            "Tool cached"
        );

        Ok(tool_dir)
    }

    /// Look up a cached tool by name.
    pub async fn get(&self, name: &str) -> Result<Option<BuildArtifact>, PipelineError> {
        let manifest_path = self.base_dir.join(name).join("manifest.json");

        if !manifest_path.exists() {
            return Ok(None);
        }

        let content = tokio::fs::read_to_string(&manifest_path).await?;
        let artifact: BuildArtifact = serde_json::from_str(&content)?;
        Ok(Some(artifact))
    }

    /// List all cached tool names.
    pub async fn list(&self) -> Result<Vec<String>, PipelineError> {
        let mut names = Vec::new();

        if !self.base_dir.exists() {
            return Ok(names);
        }

        let mut entries = tokio::fs::read_dir(&self.base_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.is_dir()
                && let Some(name) = path.file_name()
            {
                let manifest = path.join("manifest.json");
                if manifest.exists() {
                    names.push(name.to_string_lossy().into_owned());
                }
            }
        }
        names.sort();
        Ok(names)
    }

    /// Remove a cached tool.
    pub async fn remove(&self, name: &str) -> Result<(), PipelineError> {
        let tool_dir = self.base_dir.join(name);
        if tool_dir.exists() {
            tokio::fs::remove_dir_all(&tool_dir).await?;
            tracing::info!(tool = %name, "Tool removed from cache");
        }
        Ok(())
    }

    /// Get the base directory path.
    pub fn base_dir(&self) -> &Path {
        &self.base_dir
    }
}

fn dirs_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".girt")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{BuildOutput, QaResult, RefinedSpec, SecurityResult, SpecAction};
    use girt_core::spec::{CapabilityConstraints, CapabilitySpec};
    use tempfile::TempDir;

    fn make_artifact(name: &str) -> BuildArtifact {
        let spec = CapabilitySpec {
            name: name.into(),
            description: format!("Test tool: {name}"),
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
            escalated: false,
            escalated_tickets: vec![],
        }
    }

    #[tokio::test]
    async fn store_and_retrieve() {
        let tmp = TempDir::new().unwrap();
        let cache = ToolCache::new(tmp.path().to_path_buf());
        cache.init().await.unwrap();

        let artifact = make_artifact("my_tool");
        cache.store(&artifact).await.unwrap();

        let retrieved = cache.get("my_tool").await.unwrap();
        assert!(retrieved.is_some());
        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.spec.name, "my_tool");
        assert_eq!(retrieved.build_iterations, 1);
    }

    #[tokio::test]
    async fn list_cached_tools() {
        let tmp = TempDir::new().unwrap();
        let cache = ToolCache::new(tmp.path().to_path_buf());
        cache.init().await.unwrap();

        cache.store(&make_artifact("alpha")).await.unwrap();
        cache.store(&make_artifact("beta")).await.unwrap();

        let tools = cache.list().await.unwrap();
        assert_eq!(tools, vec!["alpha", "beta"]);
    }

    #[tokio::test]
    async fn get_returns_none_for_missing() {
        let tmp = TempDir::new().unwrap();
        let cache = ToolCache::new(tmp.path().to_path_buf());
        cache.init().await.unwrap();

        let result = cache.get("nonexistent").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn remove_deletes_tool() {
        let tmp = TempDir::new().unwrap();
        let cache = ToolCache::new(tmp.path().to_path_buf());
        cache.init().await.unwrap();

        cache.store(&make_artifact("deleteme")).await.unwrap();
        assert!(cache.get("deleteme").await.unwrap().is_some());

        cache.remove("deleteme").await.unwrap();
        assert!(cache.get("deleteme").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn store_writes_source_and_policy_files() {
        let tmp = TempDir::new().unwrap();
        let cache = ToolCache::new(tmp.path().to_path_buf());
        cache.init().await.unwrap();

        let artifact = make_artifact("file_check");
        let tool_dir = cache.store(&artifact).await.unwrap();

        assert!(tool_dir.join("manifest.json").exists());
        assert!(tool_dir.join("source.rs").exists());
        assert!(tool_dir.join("policy.yaml").exists());
        assert!(tool_dir.join("world.wit").exists());
    }
}
