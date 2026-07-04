//! T1.5 — external tools declared in the manifest reach a definition-run agent.
//!
//! A `[tools.mcp.<name>]` grant must spawn the stdio server, discover its tools,
//! and register them as ordinary capabilities on the assembled agent; a
//! `[tools.plugin.<name>]` grant does the same over the subprocess plugin ABI.
//! We assert the discovered tool shows up on the agent's `describe()` card —
//! registration is what makes it callable (sandbox-by-registration, §7.1).

use std::path::PathBuf;

use hugr_toolkit::AgentDefinition;
use hugr_toolkit::runtime::build_agent;

fn python3_available() -> bool {
    std::process::Command::new("python3")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// The workspace `target/debug` dir, derived from the test binary's own path
/// (`target/debug/deps/<test>`).
fn workspace_bin(name: &str) -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let debug = exe.parent()?.parent()?; // deps → debug
    let candidate = debug.join(name);
    candidate.exists().then_some(candidate)
}

#[tokio::test]
async fn manifest_declared_mcp_tool_is_registered_on_the_agent() {
    if !python3_available() {
        eprintln!("skipping: python3 unavailable");
        return;
    }
    // A tiny stdio MCP server (same shape as the hugr-host C1 test), declared
    // entirely from the manifest.
    let manifest = r#"
[agent]
name = "mcp-agent"
[models.medium]
model = "m"

[tools.mcp.fake]
command = "python3"
args = ["-u", "-c", '''
import json, sys
for line in sys.stdin:
    if not line.strip():
        continue
    msg = json.loads(line)
    if "id" not in msg:
        continue
    method = msg.get("method")
    if method == "initialize":
        result = {"protocolVersion": "2024-11-05", "capabilities": {}, "serverInfo": {"name": "fake-mcp", "version": "0"}}
    elif method == "tools/list":
        result = {"tools": [{"name": "echo", "description": "Echo a message.", "inputSchema": {"type": "object", "properties": {"message": {"type": "string"}}, "required": ["message"]}}]}
    elif method == "tools/call":
        args = msg.get("params", {}).get("arguments", {})
        result = {"content": [{"type": "text", "text": "echo:" + str(args.get("message", ""))}], "isError": False}
    else:
        print(json.dumps({"jsonrpc": "2.0", "id": msg["id"], "error": {"code": -32601, "message": "unknown method"}}), flush=True)
        continue
    print(json.dumps({"jsonrpc": "2.0", "id": msg["id"], "result": result}), flush=True)
''']
"#;
    let def = AgentDefinition::parse(manifest, "hugr.toml").unwrap();
    let (agent, warnings) = build_agent(&def)
        .await
        .expect("MCP server should be loaded from the manifest");
    assert!(warnings.is_empty(), "{warnings:?}");

    let card = agent.describe();
    let names: Vec<_> = card.tools.iter().map(|t| t.name.as_str()).collect();
    assert!(
        names.contains(&"mcp__fake__echo"),
        "manifest-declared MCP tool must be registered; got {names:?}"
    );
}

#[tokio::test]
async fn manifest_declared_plugin_tool_is_registered_on_the_agent() {
    let Some(plugin) = workspace_bin("hugr_example_plugin") else {
        eprintln!(
            "skipping: hugr_example_plugin binary not built (run `cargo build -p hugr-example-plugin`)"
        );
        return;
    };
    let manifest = format!(
        r#"
[agent]
name = "plugin-agent"
[models.medium]
model = "m"

[tools.plugin.example]
command = "{}"
"#,
        plugin.display()
    );
    let def = AgentDefinition::parse(&manifest, "hugr.toml").unwrap();
    let (agent, warnings) = build_agent(&def)
        .await
        .expect("subprocess plugin should be loaded from the manifest");
    assert!(warnings.is_empty(), "{warnings:?}");

    let card = agent.describe();
    let names: Vec<_> = card.tools.iter().map(|t| t.name.as_str()).collect();
    assert!(
        names.contains(&"reverse"),
        "manifest-declared plugin tool must be registered; got {names:?}"
    );
}
