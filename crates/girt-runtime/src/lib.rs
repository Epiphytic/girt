// girt-runtime — embedded WASM component runtime for GIRT
//
// Ported from microsoft/wassette (MIT License).
// Copyright (c) Microsoft Corporation.
// Modifications Copyright (c) Epiphytic.

//! Embedded WASM component runtime for GIRT.
//!
//! Provides [`LifecycleManager`], the single interface between `girt-proxy`
//! and the Wasmtime execution layer. See ADR-010 for the design rationale.
//!
//! # Quick start
//!
//! ```rust,no_run
//! use girt_runtime::{LifecycleManager, ComponentMeta};
//! use std::path::Path;
//!
//! # async fn run() -> anyhow::Result<()> {
//! let manager = LifecycleManager::new(None)?;
//!
//! // Restore any tools built in a previous session
//! manager.load_persisted().await;
//!
//! // After the pipeline builds a new tool:
//! let meta = ComponentMeta {
//!     component_id: "fetch_url@0.1.0".into(),
//!     tool_name: "fetch_url".into(),
//!     description: "Fetch a URL and return its body".into(),
//!     input_schema: serde_json::json!({
//!         "type": "object",
//!         "properties": { "url": { "type": "string" } },
//!         "required": ["url"]
//!     }),
//!     wasm_hash: "abc123".into(),
//!     built_at: 0,
//! };
//! manager.load_component(Path::new("/path/to/tool.wasm"), meta).await?;
//!
//! // Call the tool
//! let result = manager.call_tool("fetch_url", &serde_json::json!({"url": "https://example.com"})).await?;
//! # Ok(())
//! # }
//! ```

pub mod error;
pub mod lifecycle;
pub mod runtime_context;
pub mod storage;
pub mod wasistate;

pub use error::RuntimeError;
pub use lifecycle::LifecycleManager;
pub use storage::ComponentMeta;
