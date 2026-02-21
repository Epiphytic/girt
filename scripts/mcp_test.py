#!/usr/bin/env python3
"""Quick MCP client smoke test for GIRT."""
import json, subprocess, sys, time, os, select, threading, queue as q

GIRT_BIN = os.path.expanduser("~/.cargo/bin/girt")
CONFIG = os.path.join(os.path.dirname(__file__), "..", "girt.toml")

_lines = q.Queue()

def _reader(pipe):
    for raw in pipe:
        _lines.put(raw)
    _lines.put(None)  # EOF sentinel

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
        "name": "add_two_numbers",
        "description": (
            "Add two numbers. Takes JSON with 'a' and 'b' (numbers), "
            "returns JSON with 'result' (number). Pure arithmetic, no I/O."
        ),
        "inputs": {"a": "number", "b": "number"},
        "outputs": {"result": "number"},
    }

    print(f"[bel] Starting girt: {GIRT_BIN}", flush=True)
    proc = subprocess.Popen(
        [GIRT_BIN, "--config", CONFIG],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )

    # Background thread reads stdout line-by-line into a queue
    t = threading.Thread(target=_reader, args=(proc.stdout,), daemon=True)
    t.start()

    # Also drain stderr in background so it doesn't block
    stderr_buf = []
    def _err_reader(pipe):
        for line in pipe:
            stderr_buf.append(line.decode())
    threading.Thread(target=_err_reader, args=(proc.stderr,), daemon=True).start()

    try:
        # ── initialize ────────────────────────────────────────────────────────
        send(proc, {
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "bel-mcp-test", "version": "0.1.0"},
            }
        })
        resp = recv(timeout=15)
        server_name = resp.get("result", {}).get("serverInfo", {}).get("name", "?")
        print(f"[bel] Connected to: {server_name}", flush=True)

        send(proc, {"jsonrpc": "2.0", "method": "notifications/initialized", "params": {}})
        time.sleep(0.2)

        # ── tools/list ────────────────────────────────────────────────────────
        send(proc, {"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}})
        resp = recv(timeout=10)
        tools = [t["name"] for t in resp.get("result", {}).get("tools", [])]
        print(f"[bel] Available tools: {tools}", flush=True)
        assert "request_capability" in tools, f"request_capability missing!"

        # ── request_capability ────────────────────────────────────────────────
        print(f"\n[bel] → request_capability: {spec['name']}", flush=True)
        print(f"[bel]   Creation Gate → Orchestrator (4 LLM agents) → WasmCompiler → girt-runtime", flush=True)
        print(f"[bel]   Patience — ~2-4 min...\n", flush=True)
        t0 = time.time()

        send(proc, {
            "jsonrpc": "2.0", "id": 3,
            "method": "tools/call",
            "params": {"name": "request_capability", "arguments": spec},
        })

        resp = recv(timeout=360)
        elapsed = time.time() - t0

        content = resp.get("result", {}).get("content", [{}])
        text = content[0].get("text", "") if content else ""

        print(f"[bel] Response ({elapsed:.1f}s):", flush=True)
        try:
            parsed = json.loads(text)
            status = parsed.get("status", "?")
            print(f"  status           : {status}", flush=True)
            if status == "built":
                print(f"  build_iterations : {parsed.get('build_iterations')}", flush=True)
                print(f"  tests            : {parsed.get('tests_passed')}/{parsed.get('tests_run')}", flush=True)
                print(f"  exploits blocked : {parsed.get('exploits_attempted')} attempted, {parsed.get('exploits_succeeded')} succeeded", flush=True)

                # verify new tool appears
                send(proc, {"jsonrpc": "2.0", "id": 4, "method": "tools/list", "params": {}})
                resp2 = recv(timeout=10)
                tools2 = [t["name"] for t in resp2.get("result", {}).get("tools", [])]
                marker = "✓" if spec["name"] in tools2 else "⚠"
                print(f"\n[bel] {marker} tools/list: {tools2}", flush=True)
            else:
                print(f"  full response: {json.dumps(parsed, indent=2)}", flush=True)
        except json.JSONDecodeError:
            print(f"  raw: {text}", flush=True)

    except Exception as e:
        print(f"\n[bel] ERROR: {e}", flush=True)
        if stderr_buf:
            print("\n[girt stderr (last 20 lines)]")
            print("".join(stderr_buf[-20:]), flush=True)
        sys.exit(1)
    finally:
        proc.terminate()
        proc.wait()
        if stderr_buf:
            print("\n[girt logs]\n" + "".join(stderr_buf[-30:]), flush=True)

if __name__ == "__main__":
    main()
