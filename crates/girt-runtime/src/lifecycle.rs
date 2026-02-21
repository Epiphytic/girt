// Ported from microsoft/wassette (MIT License, with GIRT-specific modifications)
// Copyright (c) Microsoft Corporation.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use tokio::sync::RwLock;
use wasmtime::Store;
use wasmtime::component::{InstancePre, Val};

use crate::error::RuntimeError;
use crate::runtime_context::RuntimeContext;
use crate::storage::{ComponentMeta, ComponentStorage};
use crate::wasistate::WasiState;

/// A component that has been compiled and is ready for instantiation.
struct LoadedComponent {
    instance_pre: InstancePre<WasiState>,
    meta: ComponentMeta,
}

/// The GIRT embedded WASM runtime.
///
/// `LifecycleManager` owns the Wasmtime engine, the component registry, and
/// the tool-name index. It is the single point of contact between `GirtProxy`
/// and the WASM execution layer.
///
/// # Threading
///
/// `LifecycleManager` is `Send + Sync` and is typically wrapped in `Arc`.
/// Component loading is protected by an `RwLock`; multiple concurrent tool
/// calls are supported (each call creates its own `Store`).
pub struct LifecycleManager {
    runtime: Arc<RuntimeContext>,
    storage: ComponentStorage,
    /// component_id → compiled component + metadata
    components: RwLock<HashMap<String, LoadedComponent>>,
    /// tool_name → component_id (one tool per component for now)
    tool_index: RwLock<HashMap<String, String>>,
}

impl LifecycleManager {
    pub fn new(storage_dir: Option<std::path::PathBuf>) -> anyhow::Result<Self> {
        let runtime = Arc::new(RuntimeContext::new()?);
        let base_dir = storage_dir.unwrap_or_else(ComponentStorage::default_path);
        let storage = ComponentStorage::new(base_dir);
        storage.init()?;
        Ok(Self {
            runtime,
            storage,
            components: RwLock::new(HashMap::new()),
            tool_index: RwLock::new(HashMap::new()),
        })
    }

    /// Load a previously built tool into the runtime from a .wasm path.
    ///
    /// The caller must also provide metadata written by the pipeline. If the
    /// component_id is already loaded, this is a no-op (returns quickly).
    ///
    /// After this returns, the tool appears in `list_tools()` and is callable
    /// via `call_tool()`.
    pub async fn load_component(
        &self,
        wasm_path: &Path,
        meta: ComponentMeta,
    ) -> Result<String, RuntimeError> {
        let component_id = meta.component_id.clone();

        // Fast path: already loaded
        {
            let guard = self.components.read().await;
            if guard.contains_key(&component_id) {
                tracing::debug!(component_id, "Component already loaded");
                return Ok(component_id);
            }
        }

        tracing::info!(component_id, path = %wasm_path.display(), "Loading component");

        // Store wasm + metadata on disk
        self.storage.store(wasm_path, &meta)?;

        // Compile (or load from cache)
        let component = self
            .storage
            .load_or_compile(&component_id, &self.runtime.engine)?;

        // Pre-instantiate (expensive; done once per component)
        let instance_pre = self
            .runtime
            .linker
            .instantiate_pre(&component)
            .map_err(|e| RuntimeError::InstantiationFailed(format!("{component_id}: {e}")))?;

        let tool_name = meta.tool_name.clone();

        // Register
        {
            let mut components = self.components.write().await;
            components.insert(component_id.clone(), LoadedComponent { instance_pre, meta });
        }
        {
            let mut index = self.tool_index.write().await;
            index.insert(tool_name.clone(), component_id.clone());
        }

        tracing::info!(component_id, tool_name, "Component loaded and ready");
        Ok(component_id)
    }

    /// Load all components persisted on disk (e.g. after a restart).
    ///
    /// Components that fail to load are logged and skipped.
    pub async fn load_persisted(&self) {
        let ids = match self.storage.list_component_ids() {
            Ok(ids) => ids,
            Err(e) => {
                tracing::warn!("Failed to list persisted components: {e}");
                return;
            }
        };

        for id in ids {
            let meta = match self.storage.load_meta(&id) {
                Ok(m) => m,
                Err(e) => {
                    tracing::warn!(component_id = id, "Failed to load metadata: {e}");
                    continue;
                }
            };

            let wasm_path = self.storage.wasm_path(&id);
            if !wasm_path.exists() {
                tracing::warn!(component_id = id, "wasm file missing, skipping");
                continue;
            }

            // Compile (or load precompiled)
            let component = match self.storage.load_or_compile(&id, &self.runtime.engine) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!(component_id = id, "Failed to compile: {e}");
                    continue;
                }
            };

            let instance_pre = match self.runtime.linker.instantiate_pre(&component) {
                Ok(ip) => ip,
                Err(e) => {
                    tracing::warn!(component_id = id, "Failed to pre-instantiate: {e}");
                    continue;
                }
            };

            let tool_name = meta.tool_name.clone();
            {
                let mut components = self.components.write().await;
                components.insert(id.clone(), LoadedComponent { instance_pre, meta });
            }
            {
                let mut index = self.tool_index.write().await;
                index.insert(tool_name.clone(), id.clone());
            }
            tracing::info!(component_id = id, tool_name, "Persisted component restored");
        }
    }

    /// Unload a component. It will no longer appear in `list_tools()`.
    pub async fn unload_component(&self, component_id: &str) -> Result<(), RuntimeError> {
        let mut components = self.components.write().await;
        let loaded = components
            .remove(component_id)
            .ok_or_else(|| RuntimeError::ComponentNotFound(component_id.to_string()))?;

        let mut index = self.tool_index.write().await;
        index.remove(&loaded.meta.tool_name);

        tracing::info!(component_id, "Component unloaded");
        Ok(())
    }

    /// Return MCP-style tool metadata for all loaded components.
    pub async fn list_tools(&self) -> Vec<ComponentMeta> {
        let components = self.components.read().await;
        components.values().map(|c| c.meta.clone()).collect()
    }

    /// Invoke a tool by MCP tool name.
    ///
    /// The `args` value is serialized to JSON and passed to the component's
    /// `run(input: string) -> result<string, string>` export. The returned
    /// string is expected to be a JSON value.
    pub async fn call_tool(
        &self,
        tool_name: &str,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value, RuntimeError> {
        // Resolve tool → component
        let component_id = {
            let index = self.tool_index.read().await;
            index
                .get(tool_name)
                .cloned()
                .ok_or_else(|| RuntimeError::ToolNotFound(tool_name.to_string()))?
        };

        let instance_pre = {
            let components = self.components.read().await;
            components
                .get(&component_id)
                .map(|c| c.instance_pre.clone())
                .ok_or_else(|| RuntimeError::ComponentNotFound(component_id.clone()))?
        };

        tracing::debug!(tool_name, component_id, "Invoking tool");

        // Create fresh per-invocation state
        let wasi_state = WasiState::new()
            .map_err(|e| RuntimeError::InvocationFailed(e.to_string()))?;
        let mut store = Store::new(&self.runtime.engine, wasi_state);

        // Instantiate
        let instance = instance_pre
            .instantiate_async(&mut store)
            .await
            .map_err(|e| RuntimeError::InstantiationFailed(format!("{tool_name}: {e}")))?;

        // Get the `run` export
        let run_func = instance
            .get_func(&mut store, "run")
            .ok_or_else(|| RuntimeError::InvocationFailed(
                format!("{tool_name}: no 'run' export found; component may not implement girt-tool world")
            ))?;

        // Serialize args to JSON string (the component model boundary)
        let input_json = serde_json::to_string(args)?;

        // Call run(input: string) → result<string, string>
        let params = [Val::String(input_json)];
        let mut results = vec![Val::Bool(false)]; // placeholder; overwritten by call

        run_func
            .call_async(&mut store, &params, &mut results)
            .await
            .map_err(|e| RuntimeError::InvocationFailed(format!("{tool_name}: {e}")))?;

        // Required after any component call that may return results
        run_func
            .post_return_async(&mut store)
            .await
            .map_err(|e| RuntimeError::InvocationFailed(format!("{tool_name} post_return: {e}")))?;

        // Decode result<string, string>
        let output_json = extract_run_result(tool_name, results)?;

        // Parse output as JSON (tools should return valid JSON)
        let output_value: serde_json::Value = serde_json::from_str(&output_json).unwrap_or_else(
            |_| serde_json::Value::String(output_json),
        );

        Ok(output_value)
    }

    /// Return true if the named tool is currently loaded.
    pub async fn has_tool(&self, tool_name: &str) -> bool {
        self.tool_index.read().await.contains_key(tool_name)
    }
}

/// Extract the string value from `result<string, string>` Val.
fn extract_run_result(
    tool_name: &str,
    results: Vec<Val>,
) -> Result<String, RuntimeError> {
    match results.into_iter().next() {
        Some(Val::Result(Ok(Some(boxed)))) => match *boxed {
            Val::String(s) => Ok(s),
            other => Err(RuntimeError::InvocationFailed(format!(
                "{tool_name}: expected string in Ok variant, got {other:?}"
            ))),
        },
        Some(Val::Result(Err(Some(boxed)))) => match *boxed {
            Val::String(e) => Err(RuntimeError::ToolError(e)),
            other => Err(RuntimeError::ToolError(format!("{other:?}"))),
        },
        Some(Val::Result(Ok(None))) => Ok("null".into()),
        Some(Val::Result(Err(None))) => Err(RuntimeError::ToolError("(no error detail)".into())),
        Some(other) => Err(RuntimeError::InvocationFailed(format!(
            "{tool_name}: unexpected return Val: {other:?}"
        ))),
        None => Err(RuntimeError::InvocationFailed(format!(
            "{tool_name}: component returned no values"
        ))),
    }
}
