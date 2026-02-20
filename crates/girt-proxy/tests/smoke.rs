//! Smoke tests for the GIRT proxy binary.
//!
//! These tests verify that the `girt` binary builds and starts correctly.
//! Full end-to-end tests with Wassette require the `wassette` binary
//! and are run separately via the integration test script.

use std::process::Command;

/// Verify the binary exists and responds to --help.
#[test]
fn binary_responds_to_help() {
    let output = Command::new(env!("CARGO_BIN_EXE_girt"))
        .arg("--help")
        .output()
        .expect("failed to execute girt binary");

    assert!(output.status.success(), "girt --help should exit 0");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("GIRT MCP Proxy"),
        "help output should mention GIRT MCP Proxy"
    );
}

/// Verify binary exits with error when pointed at a nonexistent Wassette binary.
#[test]
fn fails_with_missing_wassette() {
    let output = Command::new(env!("CARGO_BIN_EXE_girt"))
        .arg("--wassette-bin")
        .arg("/nonexistent/wassette")
        .output()
        .expect("failed to execute girt binary");

    assert!(
        !output.status.success(),
        "should fail when wassette binary doesn't exist"
    );
}
