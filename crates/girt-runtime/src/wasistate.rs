// Ported from microsoft/wassette (MIT License)
// Copyright (c) Microsoft Corporation.

use wasmtime::component::ResourceTable;
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};
use wasmtime_wasi_http::{WasiHttpCtx, WasiHttpView};

/// Per-invocation WASM state.
///
/// A fresh `WasiState` is created for each tool call, so components are
/// stateless across invocations (ephemeral WASI execution).
///
/// Security posture (deny-default):
/// - No filesystem preopens
/// - No host environment variables
/// - stdout/stderr forwarded to tracing (captured by WasiCtxBuilder)
/// - Network access via WASI HTTP only (policy enforced at the gate layer)
pub struct WasiState {
    ctx: WasiCtx,
    table: ResourceTable,
    http: WasiHttpCtx,
}

impl WasiView for WasiState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.ctx,
            table: &mut self.table,
        }
    }
}

impl WasiHttpView for WasiState {
    fn ctx(&mut self) -> &mut WasiHttpCtx {
        &mut self.http
    }
    fn table(&mut self) -> &mut ResourceTable {
        &mut self.table
    }
}

impl WasiState {
    /// Build a minimal WASI sandbox for tool execution.
    pub fn new() -> anyhow::Result<Self> {
        let ctx = WasiCtxBuilder::new()
            // No filesystem preopens — deny-default
            // No env vars — components are isolated from host environment
            .build();

        Ok(Self {
            ctx,
            table: ResourceTable::new(),
            http: WasiHttpCtx::new(),
        })
    }
}

impl Default for WasiState {
    fn default() -> Self {
        Self::new().expect("WasiState::new should not fail")
    }
}
