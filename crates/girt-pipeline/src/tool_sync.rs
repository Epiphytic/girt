//! Tool registry sync — publishes built tool source to a git repository.
//!
//! After each successful build, `ToolSync::sync` copies the tool's source
//! files into a worktree of the configured repository and pushes to main.
//!
//! ## Worktree strategy
//! Each sync creates a dedicated git worktree so concurrent Claude Code
//! sessions (or multiple pipeline runs) don't step on each other.
//! The worktree is cleaned up after the push, win or lose.
//!
//! ## Repo layout
//! ```text
//! girt-tools/
//!   {tool_name}/
//!     source.rs       ← generated Rust source
//!     world.wit       ← WIT world definition
//!     manifest.json   ← tool metadata (spec, QA, timing)
//!     policy.yaml     ← security policy
//!     README.md       ← auto-generated from manifest
//! ```

use std::path::{Path, PathBuf};

use tokio::process::Command;

use crate::error::PipelineError;
use crate::publish::PublishResult;
use crate::types::BuildArtifact;

/// Syncs built tool source to a remote git repository.
#[derive(Debug, Clone)]
pub struct ToolSync {
    /// Remote repository URL (e.g. `https://github.com/Epiphytic/girt-tools`).
    pub repo_url: String,
    /// Local clone of the registry (default: `~/.girt/tool-registry`).
    pub local_path: PathBuf,
}

impl ToolSync {
    pub fn new(repo_url: impl Into<String>, local_path: PathBuf) -> Self {
        Self {
            repo_url: repo_url.into(),
            local_path,
        }
    }

    pub fn default_local_path() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".girt")
            .join("tool-registry")
    }

    /// Sync a published tool to the registry repository.
    ///
    /// 1. Clone or fetch the repo.
    /// 2. Create a fresh worktree for this sync.
    /// 3. Write tool files into `{tool_name}/`.
    /// 4. Commit and push to `main`.
    /// 5. Clean up worktree.
    pub async fn sync(
        &self,
        artifact: &BuildArtifact,
        result: &PublishResult,
    ) -> Result<(), PipelineError> {
        let tool_name = &artifact.spec.name;

        tracing::info!(
            tool = %tool_name,
            repo = %self.repo_url,
            "Syncing tool source to registry"
        );

        self.ensure_clone().await?;
        self.fetch_main().await?;

        let worktree = self.create_worktree(tool_name).await?;

        let sync_result = self.write_and_push(&worktree, tool_name, artifact, result).await;

        // Always clean up the worktree
        self.remove_worktree(&worktree).await;

        match sync_result {
            Ok(()) => {
                tracing::info!(tool = %tool_name, "Tool registry sync complete");
                Ok(())
            }
            Err(e) => {
                tracing::warn!(tool = %tool_name, error = %e, "Tool registry sync failed (non-fatal)");
                Err(e)
            }
        }
    }

    // ── Git operations ────────────────────────────────────────────────────────

    async fn ensure_clone(&self) -> Result<(), PipelineError> {
        if self.local_path.join(".git").exists() {
            return Ok(());
        }
        tracing::info!(repo = %self.repo_url, path = %self.local_path.display(), "Cloning tool registry");
        let status = Command::new("git")
            .args(["clone", "--depth", "1", &self.repo_url])
            .arg(&self.local_path)
            .status()
            .await
            .map_err(|e| PipelineError::IoError(e))?;

        if !status.success() {
            return Err(PipelineError::LlmError(format!(
                "git clone failed for {}",
                self.repo_url
            )));
        }
        Ok(())
    }

    async fn fetch_main(&self) -> Result<(), PipelineError> {
        let status = Command::new("git")
            .args(["-C"])
            .arg(&self.local_path)
            .args(["fetch", "origin", "main"])
            .status()
            .await
            .map_err(|e| PipelineError::IoError(e))?;

        if !status.success() {
            tracing::warn!("git fetch failed — proceeding anyway");
        }
        Ok(())
    }

    async fn create_worktree(&self, tool_name: &str) -> Result<PathBuf, PipelineError> {
        let worktree = std::env::temp_dir().join(format!(
            "girt-sync-{}-{}",
            tool_name,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0)
        ));

        let status = Command::new("git")
            .args(["-C"])
            .arg(&self.local_path)
            .arg("worktree")
            .arg("add")
            .arg(&worktree)
            .arg("main")
            .status()
            .await
            .map_err(|e| PipelineError::IoError(e))?;

        if !status.success() {
            return Err(PipelineError::LlmError(format!(
                "git worktree add failed for {}",
                worktree.display()
            )));
        }

        tracing::debug!(worktree = %worktree.display(), "Created sync worktree");
        Ok(worktree)
    }

    async fn write_and_push(
        &self,
        worktree: &Path,
        tool_name: &str,
        artifact: &BuildArtifact,
        result: &PublishResult,
    ) -> Result<(), PipelineError> {
        // Write tool files into {worktree}/{tool_name}/
        let tool_dir = worktree.join(tool_name);
        tokio::fs::create_dir_all(&tool_dir)
            .await
            .map_err(PipelineError::IoError)?;

        // Copy files from local publish dir
        self.copy_tool_files(&result.local_path, &tool_dir).await?;

        // Generate README
        let readme = generate_readme(tool_name, artifact);
        tokio::fs::write(tool_dir.join("README.md"), readme)
            .await
            .map_err(PipelineError::IoError)?;

        // git add
        let status = Command::new("git")
            .args(["-C"])
            .arg(worktree)
            .args(["add", "."])
            .status()
            .await
            .map_err(|e| PipelineError::IoError(e))?;

        if !status.success() {
            return Err(PipelineError::LlmError("git add failed".into()));
        }

        // Check if there's anything to commit
        let diff = Command::new("git")
            .args(["-C"])
            .arg(worktree)
            .args(["diff", "--cached", "--quiet"])
            .status()
            .await
            .map_err(|e| PipelineError::IoError(e))?;

        if diff.success() {
            tracing::info!(tool = %tool_name, "No changes to sync (tool unchanged)");
            return Ok(());
        }

        // git commit
        let version = "0.1.0"; // TODO: extract from manifest
        let commit_msg = format!("feat(tools): publish {tool_name}@{version}");
        let status = Command::new("git")
            .args(["-C"])
            .arg(worktree)
            .args([
                "commit",
                "-m",
                &commit_msg,
                "--author",
                "GIRT Pipeline <girt@epiphytic.dev>",
            ])
            .status()
            .await
            .map_err(|e| PipelineError::IoError(e))?;

        if !status.success() {
            return Err(PipelineError::LlmError("git commit failed".into()));
        }

        // git push
        let status = Command::new("git")
            .args(["-C"])
            .arg(worktree)
            .args(["push", "origin", "main"])
            .status()
            .await
            .map_err(|e| PipelineError::IoError(e))?;

        if !status.success() {
            return Err(PipelineError::LlmError(format!(
                "git push to {} failed",
                self.repo_url
            )));
        }

        Ok(())
    }

    async fn copy_tool_files(
        &self,
        src: &Path,
        dst: &Path,
    ) -> Result<(), PipelineError> {
        for name in &["source.rs", "world.wit", "manifest.json", "policy.yaml"] {
            let src_file = src.join(name);
            if src_file.exists() {
                tokio::fs::copy(&src_file, dst.join(name))
                    .await
                    .map_err(PipelineError::IoError)?;
            }
        }
        Ok(())
    }

    async fn remove_worktree(&self, worktree: &Path) {
        let _ = Command::new("git")
            .args(["-C"])
            .arg(&self.local_path)
            .arg("worktree")
            .arg("remove")
            .arg(worktree)
            .arg("--force")
            .status()
            .await;
    }
}

// ── README generation ─────────────────────────────────────────────────────────

fn generate_readme(tool_name: &str, artifact: &BuildArtifact) -> String {
    let spec = &artifact.spec;
    let qa = &artifact.qa_result;
    let sec = &artifact.security_result;

    let inputs = if spec.inputs.is_object() {
        spec.inputs
            .as_object()
            .map(|m| {
                m.iter()
                    .map(|(k, v)| format!("- `{}`: {}", k, v.as_str().unwrap_or("string")))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default()
    } else {
        String::new()
    };

    let outputs = if spec.outputs.is_object() {
        spec.outputs
            .as_object()
            .map(|m| {
                m.iter()
                    .map(|(k, v)| format!("- `{}`: {}", k, v.as_str().unwrap_or("string")))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default()
    } else {
        String::new()
    };

    format!(
        "# {tool_name}\n\n\
         {description}\n\n\
         ## Inputs\n\n{inputs}\n\n\
         ## Outputs\n\n{outputs}\n\n\
         ## Build info\n\n\
         - Build iterations: {iterations}\n\
         - Tests: {tests_passed}/{tests_run} passed\n\
         - Security exploits attempted: {exploits}, succeeded: {exploits_ok}\n\
         - Escalated: {escalated}\n\n\
         _Built by [GIRT](https://github.com/Epiphytic/girt)_\n",
        tool_name = tool_name,
        description = spec.description,
        inputs = inputs,
        outputs = outputs,
        iterations = artifact.build_iterations,
        tests_passed = qa.tests_passed,
        tests_run = qa.tests_run,
        exploits = sec.exploits_attempted,
        exploits_ok = sec.exploits_succeeded,
        escalated = artifact.escalated,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_local_path_is_under_home() {
        let path = ToolSync::default_local_path();
        assert!(path.ends_with(".girt/tool-registry"));
    }
}
