//! Ticket 11 acceptance: a scripted MCP stdio session against the real cc-mcp
//! process — initialize → tools/list → every tool called over the protocol, walking
//! the whole pipeline the way an AI client would (spec story 24-27). The matte and
//! TTS engines are the deterministic fakes via the same env seams the cc-core
//! tests use; ffmpeg itself is real, so the produced artifacts are real files.

use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};

mod common;
use common::{make_fixtures, seed_deck};

// ---------------------------------------------------------------------------
// tiny synchronous JSON-RPC client over the child's stdio
// ---------------------------------------------------------------------------

struct McpClient {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<std::process::ChildStdout>,
    next_id: u64,
    /// every line this client ever received — the transcript the API-key scan runs on
    transcript: Vec<String>,
}

#[derive(Debug)]
struct ToolReply {
    is_error: bool,
    /// the tool's structured JSON (what the CLI would have printed)
    value: Value,
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
        let stdout = BufReader::new(child.stdout.take().expect("stdout pipe"));
        let mut client = McpClient {
            child,
            stdin,
            stdout,
            next_id: 1,
            transcript: Vec::new(),
        };
        // protocol handshake
        let init = client.request(
            "initialize",
            json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": { "name": "protocol-test", "version": "0" }
            }),
        );
        assert_eq!(init["serverInfo"]["name"], "coursecast");
        assert_eq!(init["capabilities"]["tools"], json!({}));
        client.notify("notifications/initialized");
        client
    }

    fn send(&mut self, body: &Value) {
        let line = serde_json::to_string(body).unwrap();
        self.stdin.write_all(line.as_bytes()).unwrap();
        self.stdin.write_all(b"\n").unwrap();
        self.stdin.flush().unwrap();
    }

    fn notify(&mut self, method: &str) {
        let id = self.next_id;
        self.send(&json!({ "jsonrpc": "2.0", "method": method }));
        self.next_id = id;
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        loop {
            let mut line = String::new();
            let n = self
                .stdout
                .read_line(&mut line)
                .expect("cc-mcp stdout alive");
            assert!(n > 0, "cc-mcp closed stdout while waiting for id {id}");
            self.transcript.push(line.clone());
            let value: Value = serde_json::from_str(line.trim()).expect("stdout line is JSON");
            if value.get("id").and_then(Value::as_u64) == Some(id) {
                if let Some(error) = value.get("error") {
                    panic!("request {method} failed at protocol level: {error}");
                }
                return value["result"].clone();
            }
            // notifications or unrelated responses are skipped
        }
    }

    fn call_tool(&mut self, name: &str, arguments: Value) -> ToolReply {
        let result = self.request(
            "tools/call",
            json!({ "name": name, "arguments": arguments }),
        );
        ToolReply {
            is_error: result["isError"].as_bool().unwrap_or(false),
            value: result["structuredContent"].clone(),
        }
    }

    fn expect_ok(&mut self, name: &str, arguments: Value) -> Value {
        let reply = self.call_tool(name, arguments);
        assert!(
            !reply.is_error,
            "tool {name} unexpectedly failed: {}",
            serde_json::to_string_pretty(&reply.value).unwrap_or_default()
        );
        assert_eq!(reply.value["ok"], json!(true), "tool {name} body ok flag");
        reply.value
    }

    fn expect_error(&mut self, name: &str, arguments: Value, code: &str) -> Value {
        let reply = self.call_tool(name, arguments);
        assert!(
            reply.is_error,
            "tool {name} unexpectedly succeeded: {reply:?}"
        );
        assert_eq!(reply.value["ok"], json!(false));
        assert_eq!(
            reply.value["error"]["code"],
            json!(code),
            "tool {name} error code mismatch: {}",
            serde_json::to_string_pretty(&reply.value).unwrap_or_default()
        );
        reply.value
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        // closing stdin ends the serving loop; make sure the process is gone either way
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// ---------------------------------------------------------------------------
// fixture builders live in tests/common (shared with the CLI isomorphism test)
// ---------------------------------------------------------------------------

const SECRET_KEY: &str = "sk-live-DO-NOT-ECHO-7f3a";

// ---------------------------------------------------------------------------
// the acceptance session
// ---------------------------------------------------------------------------

#[test]
fn ai_walks_the_whole_pipeline_over_mcp_stdio() {
    let tmp = tempfile::tempdir().unwrap();
    let fx = make_fixtures(tmp.path());
    let data_dir = tmp.path().join("data");
    let mut mcp = McpClient::spawn(
        &data_dir,
        &[
            ("CC_MATTE_ENGINE", Path::new("fake")),
            ("CC_RVM_MODEL", &fx.model),
            ("CC_CREDENTIALS_FILE", &fx.credentials_file),
        ],
    );

    // --- tools/list: exactly the 16-tool face of PRD §7.2 ---
    let listed = mcp.request("tools/list", json!({}));
    let names: Vec<&str> = listed["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    let mut expected = [
        "project_create",
        "project_list",
        "project_status",
        "project_delete",
        "slide_render",
        "script_get",
        "script_set",
        "script_import_notes",
        "voice_configure",
        "voice_synthesize",
        "person_matte",
        "person_status",
        "person_matte_skip",
        "layout_set",
        "render",
        "artifact_clean",
    ];
    expected.sort_unstable();
    let mut sorted = names.clone();
    sorted.sort_unstable();
    assert_eq!(sorted, expected, "the 16 tools of PRD §7.2");
    assert!(
        listed["tools"]
            .as_array()
            .unwrap()
            .iter()
            .all(|t| t["inputSchema"].is_object()),
        "every tool carries an input schema"
    );

    // --- 1. project_create ---
    let created = mcp.expect_ok(
        "project_create",
        json!({
            "name": "协议测试微课", "resolution": "640x360"
        }),
    );
    let id = created["project_id"].as_str().unwrap().to_string();
    let project_root = PathBuf::from(created["dir"].as_str().unwrap());
    assert_eq!(created["stage"], json!("created"));
    assert!(id.len() >= 8 && id.len() <= 32 && id.chars().all(|c| c.is_ascii_alphanumeric()));

    // --- project_list: the agent can discover the ID it just created ---
    let projects = mcp.expect_ok("project_list", json!({}));
    assert!(
        projects["projects"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["project_id"] == id.as_str()),
        "project_list returns the created project: {projects}"
    );

    // --- 2. project_status: fresh project misses the user inputs ---
    let status = mcp.expect_ok("project_status", json!({ "project_id": id }));
    assert_eq!(
        status["missing"],
        json!(["deck", "script", "voice_ref", "person"])
    );
    assert_eq!(status["person"]["has_video"], false);
    assert_eq!(status["person"]["matte_done"], false);

    // person_status / person_matte_skip: the same facts and card-fallback choice
    let person = mcp.expect_ok("person_status", json!({ "project_id": id }));
    assert_eq!(person["has_video"], false);
    assert_eq!(person["matte_done"], false);
    let skipped = mcp.expect_ok("person_matte_skip", json!({ "project_id": id }));
    assert_eq!(skipped["matte_done"], false);

    // --- 3. slide_render with a relative path inside the project root ---
    seed_deck(&project_root, &fx.assets, 3);
    let slides = mcp.expect_ok(
        "slide_render",
        json!({
            "project_id": id, "source_path": "source/deck-img"
        }),
    );
    assert_eq!(slides["page_count"], json!(3));
    assert_eq!(
        slides["slides"],
        json!([
            "work/slides/slide_001.png",
            "work/slides/slide_002.png",
            "work/slides/slide_003.png"
        ])
    );
    assert!(project_root.join("work/slides/slide_001.png").is_file());

    // --- 4. script_get before a script exists: empty page list, not an error ---
    let empty = mcp.expect_ok("script_get", json!({ "project_id": id }));
    assert_eq!(empty["exists"], false);
    assert_eq!(empty["pages"], json!([]));

    // --- 5. script_set (partial pages), then read it back ---
    let set = mcp.expect_ok(
        "script_set",
        json!({
            "project_id": id,
            "pages": [
                { "page": 1, "text": "第一页讲稿" },
                { "page": 3, "text": "第三页讲稿" }
            ]
        }),
    );
    assert_eq!(set["pages"][1], json!(""), "untouched page stays empty");
    let got = mcp.expect_ok("script_get", json!({ "project_id": id }));
    assert_eq!(got["pages"], json!(["第一页讲稿", "", "第三页讲稿"]));
    // page out of range is rejected
    mcp.expect_error(
        "script_set",
        json!({ "project_id": id, "pages": [{ "page": 4, "text": "越界" }] }),
        "page_out_of_range",
    );

    // --- 6. script_import_notes from an absolute pptx path (both pages have notes) ---
    let imported = mcp.expect_ok(
        "script_import_notes",
        json!({
            "project_id": id,
            "pptx_path": fx.assets.join("deck-notes.pptx").to_string_lossy()
        }),
    );
    assert_eq!(imported["pages"][0], json!("第一页备注"));
    // the pptx has two pages: page 3 keeps the text script_set wrote earlier (the
    // merge semantics of 只改某几页 — a bare notes page would overwrite with empty)
    assert_eq!(imported["pages"][2], json!("第三页讲稿"));

    // --- 7. voice_configure: audio ingest + key stored, never echoed ---
    let voice = mcp.expect_ok(
        "voice_configure",
        json!({
            "project_id": id,
            "engine": "mimo",
            "ref_audio": fx.assets.join("voice.wav").to_string_lossy(),
            "api_key": SECRET_KEY
        }),
    );
    assert_eq!(voice["api_key_stored"], json!(true));
    assert_eq!(voice["audio"]["path"], json!("source/voice_ref.wav"));
    assert!(project_root.join("source/voice_ref.wav").is_file());
    // the key landed in the credential store file seam and only there
    assert_eq!(
        fs::read_to_string(&fx.credentials_file).unwrap(),
        SECRET_KEY
    );

    // --- 8. voice_synthesize: every scripted page through the fake engine ---
    // (all three pages carry text now: pages 1-2 from the notes, page 3 kept its
    // earlier script_set text through the merge)
    let tts = mcp.expect_ok("voice_synthesize", json!({ "project_id": id }));
    assert_eq!(tts["generated"], json!([1, 2, 3]));
    assert_eq!(tts["skipped"], json!([]));
    assert_eq!(tts["artifacts"].as_array().unwrap().len(), 3);
    assert!(project_root.join("work/audio/slide_001.wav").is_file());

    // --- 9. person_matte: video ingested by absolute path, fake engine seam ---
    let matte = mcp.expect_ok(
        "person_matte",
        json!({
            "project_id": id,
            "video_path": fx.assets.join("person.mp4").to_string_lossy()
        }),
    );
    assert_eq!(matte["skipped"], json!(false));
    assert!(project_root.join("work/person/alpha.mov").is_file());
    assert!(project_root.join("work/person/preview.png").is_file());
    let person = mcp.expect_ok("person_status", json!({ "project_id": id }));
    assert_eq!(person["has_video"], true);
    assert_eq!(person["matte_done"], true);
    assert!(person["preview_abs"]
        .as_str()
        .unwrap()
        .ends_with("preview.png"));

    // --- 10. layout_set (partial keys keep the rest) ---
    let layout = mcp.expect_ok(
        "layout_set",
        json!({
            "project_id": id, "position": "bottom_left", "shape": "circle"
        }),
    );
    assert_eq!(layout["layout"]["position"], json!("bottom_left"));
    assert_eq!(layout["layout"]["shape"], json!("circle"));
    assert_eq!(
        layout["layout"]["width_pct"],
        json!(20.0),
        "unspecified keys keep defaults"
    );
    let layout_toml = fs::read_to_string(project_root.join("layout.toml")).unwrap();
    assert!(layout_toml.contains("bottom_left"));

    // free placement replaces the legacy preset; the partial shape key is kept
    let free = mcp.expect_ok(
        "layout_set",
        json!({
            "project_id": id, "x_pct": 25.0, "y_pct": 10.0, "width_pct": 20.0
        }),
    );
    assert_eq!(free["layout"]["mode"], json!("free"));
    assert_eq!(free["layout"]["x_pct"], json!(25.0));
    assert_eq!(free["layout"]["y_pct"], json!(10.0));
    assert_eq!(free["layout"]["shape"], json!("circle"));
    let layout_toml = fs::read_to_string(project_root.join("layout.toml")).unwrap();
    assert!(layout_toml.contains("x_pct ="));
    assert!(!layout_toml.contains("position ="));

    // --- 11. render: real ffmpeg, real mp4 ---
    let film = mcp.expect_ok("render", json!({ "project_id": id }));
    assert_eq!(film["output_path"], json!("output/final.mp4"));
    assert_eq!(
        film["pip"]["source"],
        json!("alpha"),
        "matte ran: alpha branch"
    );
    let final_mp4 = project_root.join("output/final.mp4");
    assert!(final_mp4.is_file(), "the film landed");
    assert!(final_mp4.metadata().unwrap().len() > 1024);

    // --- 12. project_status reports done; stage drives the next suggestion ---
    let done = mcp.expect_ok("project_status", json!({ "project_id": id }));
    assert_eq!(done["stage"], json!("done"));
    // the deck input slot is defined as source/deck.pdf|.pptx (cc-core status.rs);
    // an image-folder deck renders fine but does not flip that slot — same JSON the
    // CLI prints for this project shape
    assert_eq!(done["missing"], json!(["deck"]));
    assert_eq!(done["next"], json!("项目已完成，成片在 output/final.mp4"));

    // --- 13. artifact_clean refuses without an explicit confirm ---
    mcp.expect_error(
        "artifact_clean",
        json!({ "project_id": id, "stage": "render" }),
        "invalid_argument",
    );
    mcp.expect_error(
        "artifact_clean",
        json!({ "project_id": id, "stage": "render", "confirm": false }),
        "invalid_argument",
    );
    let cleaned = mcp.expect_ok(
        "artifact_clean",
        json!({
            "project_id": id, "stage": "render", "confirm": true
        }),
    );
    assert_eq!(cleaned["removed"], json!(["output/final.mp4"]));
    assert!(!final_mp4.exists(), "clean actually removed the film");
    let after = mcp.expect_ok("project_status", json!({ "project_id": id }));
    assert_eq!(
        after["stage"],
        json!("done"),
        "state-machine position untouched by clean"
    );

    // --- 14. project_delete needs an explicit confirm, then the project is gone ---
    mcp.expect_error(
        "project_delete",
        json!({ "project_id": id }),
        "invalid_argument",
    );
    let deleted = mcp.expect_ok(
        "project_delete",
        json!({ "project_id": id, "confirm": true }),
    );
    assert_eq!(deleted["deleted"], true);
    assert!(!project_root.exists(), "project directory removed");
    assert_eq!(
        mcp.expect_ok("project_list", json!({}))["projects"],
        json!([])
    );
    mcp.expect_error(
        "project_status",
        json!({ "project_id": id }),
        "project_not_found",
    );

    // --- the API key never appears anywhere in the whole session transcript ---
    let transcript = mcp.transcript.join("\n");
    assert!(
        !transcript.contains(SECRET_KEY),
        "the API key must never be echoed back over MCP"
    );
}

#[test]
fn boundary_rejects_path_escape_and_invalid_ids() {
    let tmp = tempfile::tempdir().unwrap();
    let fx = make_fixtures(tmp.path());
    let data_dir = tmp.path().join("data");
    let mut mcp = McpClient::spawn(
        &data_dir,
        &[
            ("CC_MATTE_ENGINE", Path::new("fake")),
            ("CC_RVM_MODEL", &fx.model),
            ("CC_CREDENTIALS_FILE", &fx.credentials_file),
        ],
    );

    let created = mcp.expect_ok("project_create", json!({ "name": "边界测试" }));
    let id = created["project_id"].as_str().unwrap().to_string();

    // relative path that escapes the project root is refused (path canonicalized first,
    // so the probe must exist on disk — like a real attacker-provided file)
    mcp.expect_error(
        "slide_render",
        json!({
            "project_id": id,
            "source_path": "../../../assets/img1.png"
        }),
        "path_escape",
    );

    // ...while an absolute path to a folder of the same images is the documented
    // external-input case (cc-core slide sources: a .pdf or an image folder; the
    // folder's non-image files are ignored)
    let slides = mcp.expect_ok(
        "slide_render",
        json!({
            "project_id": id,
            "source_path": fx.assets.to_string_lossy()
        }),
    );
    assert_eq!(slides["page_count"], json!(2));

    // project ids are [a-z0-9]{8,32}: traversal payloads and bad formats both die at
    // validation, before any filesystem access
    for bad in [
        "../evil",
        "UPPER-CASE",
        "short",
        "012345678901234567890123456789012",
    ] {
        mcp.expect_error(
            "project_status",
            json!({ "project_id": bad }),
            "invalid_project_id",
        );
    }
    mcp.expect_error(
        "project_status",
        json!({ "project_id": "nonexistent123" }),
        "project_not_found",
    );

    // unknown tool / unknown clean stage
    let unknown = mcp.call_tool("make_video", json!({}));
    assert!(unknown.is_error);
    assert_eq!(unknown.value["error"]["code"], json!("invalid_argument"));
    mcp.expect_error(
        "artifact_clean",
        json!({ "project_id": id, "stage": "rendering", "confirm": true }),
        "invalid_argument",
    );
}

#[test]
fn reruns_are_idempotent_over_the_protocol() {
    let tmp = tempfile::tempdir().unwrap();
    let fx = make_fixtures(tmp.path());
    let data_dir = tmp.path().join("data");
    let mut mcp = McpClient::spawn(
        &data_dir,
        &[
            ("CC_MATTE_ENGINE", Path::new("fake")),
            ("CC_RVM_MODEL", &fx.model),
            ("CC_CREDENTIALS_FILE", &fx.credentials_file),
        ],
    );

    let created = mcp.expect_ok(
        "project_create",
        json!({ "name": "幂等", "resolution": "640x360" }),
    );
    let id = created["project_id"].as_str().unwrap().to_string();
    let project_root = PathBuf::from(created["dir"].as_str().unwrap());

    seed_deck(&project_root, &fx.assets, 2);
    mcp.expect_ok(
        "slide_render",
        json!({ "project_id": id, "source_path": "source/deck-img" }),
    );
    mcp.expect_ok(
        "script_set",
        json!({
            "project_id": id,
            "pages": [{ "page": 1, "text": "页一" }, { "page": 2, "text": "页二" }]
        }),
    );
    let first = mcp.expect_ok("voice_synthesize", json!({ "project_id": id }));
    assert_eq!(first["generated"], json!([1, 2]));

    // second identical call: every page skipped (不重复烧引擎/额度, spec story 27)
    let second = mcp.expect_ok("voice_synthesize", json!({ "project_id": id }));
    assert_eq!(second["generated"], json!([]));
    assert_eq!(second["skipped"], json!([1, 2]));
}
