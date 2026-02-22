#!/usr/bin/env python3
"""
Live test for the GIRT approval flow.

Submits a request for a Slack notification tool — a capability that
involves credentials and an external service, making it a good candidate
for the Creation Gate to return Ask.

When the Gate returns Ask, the proxy routes it to the discord_approval WASM,
which posts a request to the #girt-approvals Discord channel.

Watch that channel and react with 👍 to approve or 👎 to deny.
The build will proceed or abort based on your response.
"""
import json, subprocess, sys, time, os, queue as q, threading

GIRT_BIN = os.path.expanduser("~/.cargo/bin/girt")
CONFIG = os.path.join(os.path.dirname(__file__), "..", "girt.toml")

_lines = q.Queue()

def _reader(pipe):
    for raw in pipe:
        _lines.put(raw)
    _lines.put(None)

def send(proc, msg):
    proc.stdin.write((json.dumps(msg) + "\n").encode())
    proc.stdin.flush()

def recv(timeout=30):
    try:
        raw = _lines.get(timeout=timeout)
    except q.Empty:
        raise TimeoutError(f"No response within {timeout}s")
    if raw is None:
        raise EOFError("girt closed stdout")
    return json.loads(raw.decode().strip())

def main():
    spec = {
        "name": "signal_send",
        "description": (
            "Send a message to a Signal phone number using the Signal CLI REST API "
            "(signal-cli HTTP gateway). Accepts a recipient phone number, message text, "
            "and optional attachment URLs. Returns delivery status."
        ),
        "inputs": {
            "recipient":   "string — E.164 phone number of the recipient (e.g. +12065551234)",
            "message":     "string — message text to send",
            "sender":      "string — E.164 phone number of the sender account registered with signal-cli",
            "api_url":     "string — base URL of the signal-cli REST API (e.g. http://localhost:8080)",
            "attachments": "array of strings — optional URLs of files to attach",
        },
        "outputs": {
            "ok":        "boolean — true if the message was accepted for delivery",
            "timestamp": "number — Signal message timestamp",
        },
        "constraints": {
            "network": ["localhost", "api.signal.org", "cdn.signal.org"],
            "storage": [],
            "secrets": [],
        },
    }

    print("[bel] Starting girt:", GIRT_BIN, flush=True)
    env = os.environ.copy()
    env["GIRT_LOG"] = "girt=info"
    proc = subprocess.Popen(
        [GIRT_BIN, "--config", CONFIG],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=sys.stderr,
        env=env,
    )
    t = threading.Thread(target=_reader, args=(proc.stdout,), daemon=True)
    t.start()
    time.sleep(0.5)

    # --- initialize ---
    send(proc, {"jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": {"protocolVersion": "2024-11-05",
                           "clientInfo": {"name": "test", "version": "0.0.1"},
                           "capabilities": {}}})
    r = recv()
    print(f"[bel] Connected to: {r.get('result', {}).get('serverInfo', {}).get('name', 'unknown')}", flush=True)

    # --- initialized notification (required by MCP protocol) ---
    send(proc, {"jsonrpc": "2.0", "method": "notifications/initialized", "params": {}})
    time.sleep(0.2)

    # --- list tools ---
    send(proc, {"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}})
    r = recv()
    tools = [t["name"] for t in r.get("result", {}).get("tools", [])]
    print(f"[bel] Available tools: {tools}", flush=True)

    # --- request capability ---
    print(f"[bel] → request_capability: {spec['name']}", flush=True)
    print(f"[bel] Creation Gate is in 'llm' mode — Signal has privacy taint, expect ASK", flush=True)
    print(f"[bel] Watch #girt-approvals in Discord — react 👍 to approve, 👎 to deny", flush=True)
    send(proc, {"jsonrpc": "2.0", "id": 3, "method": "tools/call",
                "params": {"name": "request_capability", "arguments": spec}})

    # Wait up to 10 minutes (Creation Gate + approval + build)
    start = time.time()
    result = recv(timeout=600)
    elapsed = time.time() - start
    print(f"[bel] Got response in {elapsed:.1f}s", flush=True)

    content = result.get("result", {}).get("content", [{}])
    text = content[0].get("text", "") if content else ""
    is_error = result.get("result", {}).get("isError", False)

    try:
        parsed = json.loads(text)
        status = parsed.get("status", "unknown")
        print(f"\n[bel] Status: {status}", flush=True)

        if status == "built":
            print(f"[bel] ✅ Tool built successfully!", flush=True)
            print(f"[bel]    Build iterations:   {parsed.get('build_iterations')}", flush=True)
            print(f"[bel]    Tests:              {parsed.get('tests_passed')}/{parsed.get('tests_run')}", flush=True)
            print(f"[bel]    Escalated:          {parsed.get('escalated')}", flush=True)
            if parsed.get('timings'):
                t = parsed['timings']
                print(f"[bel]    Architect:          {t.get('architect_ms')}ms", flush=True)
                print(f"[bel]    Planner:            {t.get('planner_ms')}ms", flush=True)
        elif status == "denied":
            print(f"[bel] ❌ Request denied: {parsed.get('reason')}", flush=True)
            print(f"[bel]    Authorized by:  {parsed.get('authorized_by')}", flush=True)
            print(f"[bel]    Evidence:       {parsed.get('evidence_url')}", flush=True)
        elif status == "approval_failed":
            print(f"[bel] ⚠️  Approval failed: {parsed.get('error')}", flush=True)
        else:
            print(f"[bel] Response: {json.dumps(parsed, indent=2)}", flush=True)
    except json.JSONDecodeError:
        print(f"[bel] Raw response: {text}", flush=True)

    proc.terminate()
    proc.wait(timeout=3)

if __name__ == "__main__":
    main()
