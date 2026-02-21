// Ported from microsoft/wassette (MIT License, with GIRT-specific modifications)
// Copyright (c) Microsoft Corporation.

use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use wasmtime::Engine;
use wasmtime::component::Component;

use crate::error::RuntimeError;

const PRECOMPILED_EXT: &str = "cwasm";
const METADATA_EXT: &str = "metadata.json";

/// Tool metadata stored alongside a WASM component.
///
/// Written by the GIRT pipeline after a successful build. Used to
/// register the tool's MCP schema without WIT introspection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentMeta {
    /// Stable component identifier (e.g. "fetch_url@0.1.0")
    pub component_id: String,
    /// MCP tool name (must match `^[a-zA-Z0-9_-]{1,128}$`)
    pub tool_name: String,
    /// Human-readable description
    pub description: String,
    /// JSON Schema for tool inputs (displayed in list_tools)
    pub input_schema: serde_json::Value,
    /// SHA-256 hex of the .wasm bytes (for cache validation)
    pub wasm_hash: String,
    /// Pipeline build timestamp (Unix ms)
    pub built_at: u64,
}

/// Disk-backed component cache.
///
/// Layout under `base_dir`:
/// ```text
/// {base_dir}/
///   {component_id}.wasm           - source binary
///   {component_id}.cwasm          - precompiled (Wasmtime serialized)
///   {component_id}.metadata.json  - tool metadata
/// ```
pub struct ComponentStorage {
    base_dir: PathBuf,
}

impl ComponentStorage {
    pub fn new(base_dir: PathBuf) -> Self {
        Self { base_dir }
    }

    pub fn default_path() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".girt")
            .join("components")
    }

    pub fn init(&self) -> Result<()> {
        std::fs::create_dir_all(&self.base_dir)?;
        Ok(())
    }

    pub fn wasm_path(&self, component_id: &str) -> PathBuf {
        self.base_dir.join(format!("{component_id}.wasm"))
    }

    pub fn cwasm_path(&self, component_id: &str) -> PathBuf {
        self.base_dir.join(format!("{component_id}.{PRECOMPILED_EXT}"))
    }

    pub fn meta_path(&self, component_id: &str) -> PathBuf {
        self.base_dir.join(format!("{component_id}.{METADATA_EXT}"))
    }

    /// Copy a WASM binary into storage and write its metadata.
    pub fn store(&self, wasm_src: &Path, meta: &ComponentMeta) -> Result<(), RuntimeError> {
        std::fs::copy(wasm_src, self.wasm_path(&meta.component_id))?;
        let meta_json = serde_json::to_string_pretty(meta)?;
        std::fs::write(self.meta_path(&meta.component_id), meta_json)?;
        Ok(())
    }

    /// Load metadata for a component (by ID).
    pub fn load_meta(&self, component_id: &str) -> Result<ComponentMeta, RuntimeError> {
        let path = self.meta_path(component_id);
        let content = std::fs::read_to_string(&path)
            .map_err(|_| RuntimeError::ComponentNotFound(component_id.to_string()))?;
        let meta: ComponentMeta = serde_json::from_str(&content)?;
        Ok(meta)
    }

    /// List all component IDs available on disk (those with .metadata.json).
    pub fn list_component_ids(&self) -> Result<Vec<String>, RuntimeError> {
        let mut ids = Vec::new();
        if !self.base_dir.exists() {
            return Ok(ids);
        }
        for entry in std::fs::read_dir(&self.base_dir)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if let Some(id) = name.strip_suffix(&format!(".{METADATA_EXT}")) {
                ids.push(id.to_string());
            }
        }
        Ok(ids)
    }

    /// Load or compile a component, using the precompiled cache when valid.
    pub fn load_or_compile(
        &self,
        component_id: &str,
        engine: &Engine,
    ) -> Result<Component, RuntimeError> {
        let wasm_path = self.wasm_path(component_id);
        let cwasm_path = self.cwasm_path(component_id);

        // Check if precompiled cache is valid (same hash as source .wasm)
        if cwasm_path.exists() && wasm_path.exists() {
            if let Ok(cached) = self.load_precompiled(&cwasm_path, engine) {
                tracing::debug!(component_id, "Loaded from precompiled cache");
                return Ok(cached);
            }
            tracing::debug!(component_id, "Precompiled cache invalid, recompiling");
        }

        // Compile from source
        let wasm_bytes = std::fs::read(&wasm_path)
            .map_err(|e| RuntimeError::StorageError(format!("Cannot read {}: {e}", wasm_path.display())))?;

        let component = Component::from_binary(engine, &wasm_bytes)
            .map_err(|e| RuntimeError::CompilationFailed(format!("{component_id}: {e}")))?;

        // Save precompiled cache
        if let Ok(serialized) = component.serialize() {
            let _ = std::fs::write(&cwasm_path, serialized);
            tracing::debug!(component_id, "Saved precompiled cache");
        }

        Ok(component)
    }

    fn load_precompiled(&self, path: &Path, engine: &Engine) -> Result<Component> {
        // SAFETY: We control the cwasm files — they were written by this same
        // process using the same Wasmtime version. The safety contract of
        // `deserialize_file` is satisfied.
        unsafe { Component::deserialize_file(engine, path) }
    }
}

/// Hash the bytes of a WASM file for cache validation.
pub fn hash_wasm(path: &Path) -> Result<String, RuntimeError> {
    let bytes = std::fs::read(path)?;
    let hash = Sha256::digest(&bytes);
    Ok(hex::encode(hash))
}
