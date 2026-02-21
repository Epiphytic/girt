// Ported from microsoft/wassette (MIT License)
// Copyright (c) Microsoft Corporation.

use anyhow::Result;
use wasmtime::{Config, Engine};
use wasmtime::component::Linker;

use crate::wasistate::WasiState;

/// Shared Wasmtime engine and linker.
///
/// `RuntimeContext` is constructed once and shared across all component
/// loads and invocations. The engine is thread-safe; the linker is
/// pre-configured with WASI p2 and WASI HTTP host functions.
pub struct RuntimeContext {
    pub engine: Engine,
    pub linker: Linker<WasiState>,
}

impl RuntimeContext {
    pub fn new() -> Result<Self> {
        let mut config = Config::new();
        config.wasm_component_model(true);
        config.async_support(true);
        // Future: config.consume_fuel(true) for CPU limits

        let engine = Engine::new(&config)?;
        let mut linker: Linker<WasiState> = Linker::new(&engine);

        // Wire WASI p2 host functions (filesystem, clocks, random, stdio, …)
        wasmtime_wasi::p2::add_to_linker_async(&mut linker)?;

        // Wire WASI HTTP host functions (outgoing HTTP requests)
        wasmtime_wasi_http::add_only_http_to_linker_async(&mut linker)?;

        tracing::debug!("RuntimeContext initialized (component-model + async + WASI p2 + HTTP)");

        Ok(Self { engine, linker })
    }
}

impl Default for RuntimeContext {
    fn default() -> Self {
        Self::new().expect("RuntimeContext::new should not fail with default config")
    }
}
