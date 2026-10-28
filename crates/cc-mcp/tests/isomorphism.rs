//! Ticket 11 checklist #2: 每个工具返回值与对应 CLI 子命令的 JSON 同构 — proven the
//! hard way, by running both shells over the same fixtures and comparing. The CLI is
//! the primary test seam (spec Testing Decisions); the MCP face must print the same
//! shapes because both assemble their JSON from the same cc-core results.

mod common;

use common::{make_fixtures, seed_deck, Fixtures};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The cc-cli binary sits next to cc-mcp under target/debug (workspace test runs
/// build it for cc-cli's own integration tests). When running this crate alone it
/// may be absent, so build the sibling on demand.
fn cc_cli_exe() -> PathBuf {
    let sibling = Path::new(env!("CARGO_BIN_EXE_cc-mcp"))
        .parent()
        .unwrap()
        .join("cc-cli.exe");
    if !sibling.is_file() {
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml");
        let out = Command::new(env!("CARGO"))
            .args([
                "build",
                "--manifest-path",
                manifest.to_str().unwrap(),
                "-p",
                "cc-cli",
                "--bin",
                "cc-cli",
            ])
            .output()
            .expect("cargo build cc-cli runs");
        assert!(
            out.status.success(),
            "building cc-cli failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    sibling
}

/// The stdio client from protocol.rs, trimmed to what this test needs.
struct McpClient {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    stdout: std::io::BufReader<std::process::ChildStdout>,
    next_id: u64,
}

impl McpClient {
    fn spawn(data_dir: &Path, env: &[(&str, &Path)]) -> Self {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_cc-mcp"));
        cmd.arg("--data-dir")
            .arg(data_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        for (key, value) in env {
            cmd.env(key, value);
        }
        let mut child = cmd.spawn().expect("cc-mcp spawns");
        let stdin = child.stdin.take().expect("stdin pipe");
        let stdout = std::io::BufReader::new(child.stdout.take().expect("stdout pipe"));
        let mut client = McpClient {
            child,
            stdin,
            stdout,
            next_id: 1,
        };
        client.request(
            "initialize",
            json!({ "protocolVersion": "2025-06-18", "capabilities": {},
                    "clientInfo": { "name": "isomorphism", "version": "0" } }),
        );
        client.notify("notifications/initialized");
        client
    }

    fn send(&mut self, body: &Value) {
        use std::io::Write;
        let line = serde_json::to_string(body).unwrap();
        self.stdin.write_all(line.as_bytes()).unwrap();
        self.stdin.write_all(b"\n").unwrap();
        self.stdin.flush().unwrap();
    }

    fn notify(&mut self, method: &str) {
        self.send(&json!({ "jsonrpc": "2.0", "method": method }));
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        loop {
            let mut line = String::new();
            use std::io::BufRead;
            let n = self.stdout.read_line(&mut line).expect("stdout alive");
            assert!(n > 0, "cc-mcp closed stdout waiting for id {id}");
            let value: Value = serde_json::from_str(line.trim()).expect("JSON line");
            if value.get("id").and_then(Value::as_u64) == Some(id) {
                return value["result"].clone();
            }
        }
    }

    fn tool(&mut self, name: &str, arguments: Value) -> Value {
        let result = self.request(
            "tools/call",
            json!({ "name": name, "arguments": arguments }),
        );
        assert!(
            !result["isError"].as_bool().unwrap_or(false),
            "tool {name} failed: {result}"
        );
        result["structuredContent"].clone()
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn cli_json(data_dir: &Path, args: &[&str], env: &[(&str, &Path)]) -> Value {
    let mut cmd = Command::new(cc_cli_exe());
    cmd.arg("--data-dir").arg(data_dir).args(args);
    for (key, value) in env {
        cmd.env(key, value);
    }
    let output = cmd.output().unwrap();
    let value: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        panic!(
            "CLI stdout not JSON: {e}\n{}",
            String::from_utf8_lossy(&output.stdout)
        )
    });
    assert!(
        output.status.success(),
        "cc-cli {args:?} failed: {}",
        serde_json::to_string_pretty(&value).unwrap_or_default()
    );
    value
}

fn sorted_keys(value: &Value) -> Vec<String> {
    let mut keys: Vec<String> = value.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    keys
}

fn assert_same_shape(context: &str, cli: &Value, mcp: &Value) {
    assert_eq!(
        sorted_keys(cli),
        sorted_keys(mcp),
        "{context}: CLI and MCP JSON shapes differ\nCLI: {cli}\nMCP: {mcp}"
    );
}

#[test]
fn mcp_tools_print_the_same_json_as_the_cli_subcommands() {
    let tmp = tempfile::tempdir().unwrap();
    let fx: Fixtures = make_fixtures(tmp.path());
    let cli_data = tmp.path().join("data-cli");
    let mcp_data = tmp.path().join("data-mcp");
    let cli_env: Vec<(&str, &Path)> = vec![("CC_CREDENTIALS_FILE", &fx.credentials_file)];
    let mut mcp = McpClient::spawn(&mcp_data, &cli_env);

    // (cli, via) = what the CLI printed and what the MCP tool returned for the same step

    // --- project create ↔ project_create ---
    let cli = cli_json(
        &cli_data,
        &[
            "project",
            "create",
            "--json",
            "--name",
            "同构对照",
            "--resolution",
            "640x360",
        ],
        &cli_env,
    );
    let via_mcp = mcp.tool(
        "project_create",
        json!({ "name": "同构对照", "resolution": "640x360" }),
    );
    assert_same_shape("project_create", &cli, &via_mcp);
    assert_eq!(cli["stage"], via_mcp["stage"]);

    let cli_id = cli["project_id"].as_str().unwrap().to_string();
    let mcp_id = via_mcp["project_id"].as_str().unwrap().to_string();
    let cli_root = PathBuf::from(cli["dir"].as_str().unwrap());
    let mcp_root = PathBuf::from(via_mcp["dir"].as_str().unwrap());

    // --- project list ↔ project_list (each data dir has exactly this project) ---
    let cli = cli_json(&cli_data, &["project", "list", "--json"], &cli_env);
    let via_mcp = mcp.tool("project_list", json!({}));
    assert_same_shape("project_list", &cli, &via_mcp);
    assert_eq!(cli["projects"].as_array().unwrap().len(), 1);
    assert_eq!(cli["projects"][0]["name"], via_mcp["projects"][0]["name"]);
    assert_eq!(cli["projects"][0]["stage"], via_mcp["projects"][0]["stage"]);

    // --- project status ↔ project_status ---
    let cli = cli_json(
        &cli_data,
        &["project", "status", "--json", "--project", &cli_id],
        &cli_env,
    );
    let via_mcp = mcp.tool("project_status", json!({ "project_id": mcp_id }));
    assert_same_shape("project_status", &cli, &via_mcp);
    assert_eq!(cli["missing"], via_mcp["missing"]);
    assert_eq!(cli["person"]["matte_done"], via_mcp["person"]["matte_done"]);

    // --- person status ↔ person_status ---
    let cli = cli_json(
        &cli_data,
        &["person_status", "--json", "--project", &cli_id],
        &cli_env,
    );
    let via_mcp = mcp.tool("person_status", json!({ "project_id": mcp_id }));
    assert_same_shape("person_status", &cli, &via_mcp);
    assert_eq!(cli["has_video"], false);
    assert_eq!(cli["matte_done"], via_mcp["matte_done"]);

    // --- person keep-card choice ↔ person_matte_skip ---
    let cli = cli_json(
        &cli_data,
        &["person_matte_skip", "--json", "--project", &cli_id],
        &cli_env,
    );
    let via_mcp = mcp.tool("person_matte_skip", json!({ "project_id": mcp_id }));
    assert_same_shape("person_matte_skip", &cli, &via_mcp);
    assert_eq!(cli["matte_done"], false);

    // --- slide_render ↔ slide_render ---
    seed_deck(&cli_root, &fx.assets, 2);
    seed_deck(&mcp_root, &fx.assets, 2);
    let cli = cli_json(
        &cli_data,
        &[
            "slide_render",
            "--json",
            "--project",
            &cli_id,
            "--source",
            "source/deck-img",
        ],
        &cli_env,
    );
    let via_mcp = mcp.tool(
        "slide_render",
        json!({ "project_id": mcp_id, "source_path": "source/deck-img" }),
    );
    assert_same_shape("slide_render", &cli, &via_mcp);
    assert_eq!(cli["page_count"], via_mcp["page_count"]);
    assert_eq!(cli["slides"], via_mcp["slides"]);

    // --- voice_synthesize ↔ voice_synthesize (fake engine both sides) ---
    fs::write(cli_root.join("script.md"), "第一页\n\n---\n\n第二页").unwrap();
    fs::write(mcp_root.join("script.md"), "第一页\n\n---\n\n第二页").unwrap();
    let cli = cli_json(
        &cli_data,
        &["voice_synthesize", "--json", "--project", &cli_id],
        &cli_env,
    );
    let via_mcp = mcp.tool("voice_synthesize", json!({ "project_id": mcp_id }));
    assert_same_shape("voice_synthesize", &cli, &via_mcp);
    assert_eq!(cli["generated"], via_mcp["generated"]);
    assert_eq!(cli["skipped"], via_mcp["skipped"]);

    // --- voice_configure ↔ voice_configure (audio + key; key never echoed either way) ---
    fs::copy(
        fx.assets.join("voice.wav"),
        tmp.path().join("voice-copy.wav"),
    )
    .unwrap();
    let cli = cli_json(
        &cli_data,
        &[
            "voice_configure",
            "--json",
            "--project",
            &cli_id,
            "--audio",
            tmp.path().join("voice-copy.wav").to_str().unwrap(),
            "--api-key",
            "sk-iso-key",
        ],
        &cli_env,
    );
    let via_mcp = mcp.tool(
        "voice_configure",
        json!({
            "project_id": mcp_id, "engine": "mimo",
            "ref_audio": tmp.path().join("voice-copy.wav").to_string_lossy(),
            "api_key": "sk-iso-key"
        }),
    );
    assert_same_shape("voice_configure", &cli, &via_mcp);
    assert_eq!(cli["api_key_stored"], via_mcp["api_key_stored"]);
    assert_eq!(cli["audio"]["path"], via_mcp["audio"]["path"]);

    // --- voice preference switch ↔ voice_configure(engine=edge, edge_voice=yunxi) ---
    let cli = cli_json(
        &cli_data,
        &[
            "voice_configure",
            "--json",
            "--project",
            &cli_id,
            "--engine",
            "edge",
            "--edge-voice",
            "yunxi",
        ],
        &cli_env,
    );
    let via_mcp = mcp.tool(
        "voice_configure",
        json!({
            "project_id": mcp_id, "engine": "edge", "edge_voice": "yunxi"
        }),
    );
    assert_same_shape("voice_configure(edge)", &cli, &via_mcp);
    assert_eq!(cli["tts"], via_mcp["tts"]);
    assert_eq!(cli["tts"]["engine"], "edge");
    assert_eq!(cli["tts"]["edge_voice"], "yunxi");

    // --- render ↔ render (card branch: person.mp4 present, no matte) ---
    fs::copy(
        fx.assets.join("person.mp4"),
        cli_root.join("source/person.mp4"),
    )
    .unwrap();
    fs::copy(
        fx.assets.join("person.mp4"),
        mcp_root.join("source/person.mp4"),
    )
    .unwrap();
    // the CLI runs timeline as its own subcommand before render; the MCP render tool
    // folds the (idempotent) timeline run in — same cc-core timeline runner underneath
    cli_json(
        &cli_data,
        &["timeline", "--json", "--project", &cli_id],
        &cli_env,
    );
    let cli = cli_json(
        &cli_data,
        &["render", "--json", "--project", &cli_id],
        &cli_env,
    );
    let via_mcp = mcp.tool("render", json!({ "project_id": mcp_id }));
    assert_same_shape("render", &cli, &via_mcp);
    assert_eq!(cli["output_path"], via_mcp["output_path"]);
    assert_eq!(cli["pip"]["source"], via_mcp["pip"]["source"]);
    assert_eq!(cli["layout"], via_mcp["layout"]);
    assert!(mcp_root.join("output/final.mp4").is_file());

    // final status: both shells report the same pipeline position
    let cli = cli_json(
        &cli_data,
        &["project", "status", "--json", "--project", &cli_id],
        &cli_env,
    );
    let via_mcp = mcp.tool("project_status", json!({ "project_id": mcp_id }));
    assert_eq!(cli["stage"], via_mcp["stage"]);

    // --- project delete ↔ project_delete (explicit confirm; same shape) ---
    let cli = cli_json(
        &cli_data,
        &[
            "project",
            "delete",
            "--json",
            "--project",
            &cli_id,
            "--confirm",
        ],
        &cli_env,
    );
    let via_mcp = mcp.tool(
        "project_delete",
        json!({ "project_id": mcp_id, "confirm": true }),
    );
    assert_same_shape("project_delete", &cli, &via_mcp);
    assert_eq!(cli["deleted"], true);
    assert!(!cli_root.exists(), "CLI delete removed the project dir");
    assert!(!mcp_root.exists(), "MCP delete removed the project dir");
}

/// person_matte's CLI twin always loads the real RVM model; running it here would
/// drag the 900-frame inference budget into a shape test. The MCP response keys are
/// pinned against the CLI's json! literal field list instead (crates/cc-cli/src/
/// main.rs, Cmd::PersonMatte) — the values come from the same MatteReport.
#[test]
fn person_matte_mcp_shape_matches_the_cli_field_list() {
    let tmp = tempfile::tempdir().unwrap();
    let fx = make_fixtures(tmp.path());
    let mut mcp = McpClient::spawn(
        &tmp.path().join("data"),
        &[
            ("CC_MATTE_ENGINE", Path::new("fake")),
            ("CC_RVM_MODEL", &fx.model),
            ("CC_CREDENTIALS_FILE", &fx.credentials_file),
        ],
    );
    let created = mcp.tool(
        "project_create",
        json!({ "name": "matte 形状", "resolution": "640x360" }),
    );
    let id = created["project_id"].as_str().unwrap().to_string();
    let root = PathBuf::from(created["dir"].as_str().unwrap());
    seed_deck(&root, &fx.assets, 1);
    mcp.tool(
        "slide_render",
        json!({ "project_id": id, "source_path": "source/deck-img" }),
    );
    fs::write(root.join("script.md"), "唯一一页").unwrap();
    mcp.tool("voice_synthesize", json!({ "project_id": id }));

    let matte = mcp.tool(
        "person_matte",
        json!({
            "project_id": id,
            "video_path": fx.assets.join("person.mp4").to_string_lossy()
        }),
    );
    // field list mirrors the CLI's person_matte --json literal (plus ok, which the
    // CLI also prints) — one source of truth, two shells
    let mut expected = [
        "ok",
        "project_id",
        "stage",
        "skipped",
        "frames",
        "width",
        "height",
        "fps",
        "elapsed_sec",
        "fingerprint",
        "artifacts",
        "preview",
        "preview_abs",
        "alpha_abs",
        "next",
    ];
    expected.sort_unstable();
    assert_eq!(sorted_keys(&matte), expected);
}
