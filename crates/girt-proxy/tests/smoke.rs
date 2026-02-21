//! Smoke tests for the `girt` binary.
//!
//! Verifies the binary starts, responds to CLI flags, and subcommands work
//! without requiring a running MCP session or Anthropic credentials.

use std::process::Command;

fn girt() -> Command {
    Command::new(env!("CARGO_BIN_EXE_girt"))
}

// ── Help / basic CLI ──────────────────────────────────────────────────────────

#[test]
fn binary_responds_to_help() {
    let output = girt().arg("--help").output().expect("failed to execute girt");
    assert!(output.status.success(), "girt --help should exit 0");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("GIRT"), "help output should mention GIRT");
    assert!(stdout.contains("auth"), "help output should list auth subcommand");
    assert!(stdout.contains("serve"), "help output should list serve subcommand");
}

#[test]
fn auth_subcommand_help() {
    let output = girt()
        .args(["auth", "--help"])
        .output()
        .expect("failed to execute girt auth --help");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("login"));
    assert!(stdout.contains("status"));
    assert!(stdout.contains("logout"));
}

#[test]
fn auth_login_help() {
    let output = girt()
        .args(["auth", "login", "--help"])
        .output()
        .expect("failed to execute girt auth login --help");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("console"), "login should document --console flag");
}

// ── Auth status without credentials ──────────────────────────────────────────

#[test]
fn auth_status_no_credentials_exits_cleanly() {
    // Point at a temp dir that definitely has no auth.json.
    // We can't inject a custom token path via CLI yet, but we can verify
    // the command exits 0 and doesn't panic even without real credentials.
    // (The store falls back gracefully to "not logged in".)
    let output = girt()
        .args(["auth", "status"])
        .env("HOME", "/tmp") // no ~/.config/girt/auth.json here
        .output()
        .expect("failed to execute girt auth status");

    // Should exit cleanly (status 0) even with no credentials stored.
    assert!(
        output.status.success(),
        "auth status should exit 0 when not logged in\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Not logged in") || stderr.contains("Run"),
        "should print a helpful message when not logged in: {stderr}"
    );
}

// ── Serve with missing config ─────────────────────────────────────────────────

#[test]
fn serve_fails_cleanly_with_no_config() {
    // Run from /tmp where there's no girt.toml and no ~/.config/girt/girt.toml.
    let output = girt()
        .arg("serve")
        .current_dir("/tmp")
        .env("HOME", "/tmp")
        .output()
        .expect("failed to execute girt serve");

    assert!(
        !output.status.success(),
        "girt serve should fail when no girt.toml is found"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("girt.toml") || stderr.contains("config"),
        "error message should mention config: {stderr}"
    );
}

#[test]
fn unknown_subcommand_exits_nonzero() {
    let output = girt()
        .arg("nonexistent-subcommand")
        .output()
        .expect("failed to execute girt");
    assert!(!output.status.success(), "unknown subcommand should exit non-zero");
}
