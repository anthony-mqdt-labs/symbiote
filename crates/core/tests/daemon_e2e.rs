//! Tracer bullet: zsh hook event → daemon → typed UI object updates, against the real binary.

use serde_json::Value;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use symbiote_core::daemon::sock_path;

const BIN: &str = env!("CARGO_BIN_EXE_symbiote");

struct Daemon {
    child: Child,
    root: PathBuf,
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn workspace(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("sym-e2e-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn spawn(root: &Path, llm_url: &str) -> Daemon {
    let child = Command::new(BIN)
        .args(["daemon", "--root"])
        .arg(root)
        .env("SYMBIOTE_LLM_URL", llm_url)
        .env("SYMBIOTE_LOG_DIR", root.join("logs"))
        .spawn()
        .unwrap();
    let d = Daemon { child, root: root.to_path_buf() };
    let t = Instant::now();
    while UnixStream::connect(sock_path(root)).is_err() {
        assert!(t.elapsed() < Duration::from_secs(5), "daemon socket never came up");
        std::thread::sleep(Duration::from_millis(50));
    }
    d
}

struct Ui {
    w: UnixStream,
    r: BufReader<UnixStream>,
}

impl Ui {
    fn connect(root: &Path) -> Self {
        let w = UnixStream::connect(sock_path(root)).unwrap();
        w.set_read_timeout(Some(Duration::from_millis(200))).unwrap();
        let mut ui = Ui { r: BufReader::new(w.try_clone().unwrap()), w };
        ui.send(r#"{"type":"hello","role":"ui"}"#);
        ui
    }

    fn send(&mut self, line: &str) {
        self.w.write_all(format!("{line}\n").as_bytes()).unwrap();
    }

    /// Read messages until one satisfies `pred`, or panic after `secs`.
    fn until(&mut self, secs: u64, what: &str, pred: impl Fn(&Value) -> bool) -> Value {
        let t = Instant::now();
        let mut line = String::new();
        while t.elapsed() < Duration::from_secs(secs) {
            line.clear();
            match self.r.read_line(&mut line) {
                Ok(n) if n > 0 => {
                    let v: Value = serde_json::from_str(&line).unwrap();
                    if pred(&v) {
                        return v;
                    }
                }
                _ => {}
            }
        }
        panic!("timed out waiting for {what}");
    }
}

fn emit(root: &Path, cmd: &str, exit: i32) {
    let st = Command::new(BIN)
        .args(["emit", "cmd_end", "--exit", &exit.to_string(), "--duration", "12", "--"])
        .arg(cmd)
        .env("SYMBIOTE_ROOT", root)
        .env("SYMBIOTE_LOG_DIR", root.join("logs"))
        .status()
        .unwrap();
    assert!(st.success());
}

fn tasks(v: &Value) -> bool {
    v["type"] == "set_tasks"
}

#[test]
fn tier0_terminal_event_to_ui_objects() {
    let root = workspace("t0");
    let _d = spawn(&root, "http://127.0.0.1:9/v1"); // nothing listens: Tier 0
    let mut ui = Ui::connect(&root);

    let mode = ui.until(5, "mode", |v| v["type"] == "mode");
    assert_eq!(mode["offline"], true);
    assert_eq!(mode["label"], "[offline: drill mode]");
    let picker = ui.until(5, "track picker", |v| v["type"] == "track_picker");
    let ids: Vec<&str> = picker["tracks"].as_array().unwrap().iter().map(|t| t["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["go", "rust", "ts"], "recommended track first");
    assert_eq!(picker["allow_custom"], false, "no free-form brief while offline");

    // An unknown track while offline fails fast with the spec's exact message.
    ui.send(r#"{"type":"start_track","track":"haskell"}"#);
    let p = ui.until(5, "fail-fast", |v| v["type"] == "track_picker" && !v["message"].is_null());
    assert_eq!(p["message"], "No pre-built track for 'haskell' and LLM is offline. Start spark or set SYMBIOTE_LLM_URL.");

    ui.send(r#"{"type":"start_track","track":"ts"}"#);
    let t = ui.until(30, "slice 00 red", |v| tasks(v) && v["slice_id"] == "00-anchor" && v["probe_status"] == "red");
    assert!(t["lesson"].as_str().unwrap().contains("integer cents"), "Tier 0 shows the structured lesson");
    assert!(root.join("src/money.ts").exists() && root.join("src/money.test.ts").exists());
    assert!(root.join(".symbiote/slices/00-anchor.yaml").exists());

    // Same failure twice → stall → mentor hint + stderr pointer pinned to the Task List.
    emit(&root, "bun test src/money.test.ts", 1);
    ui.until(30, "first red run", |v| tasks(v) && v["steps"][0]["done"] == true && v["probe_status"] == "red");
    emit(&root, "bun test src/money.test.ts", 1);
    let stalled = ui.until(30, "stall hint", |v| tasks(v) && !v["hint"].is_null());
    assert!(stalled["hint"].as_str().unwrap().contains("Number.isInteger"));
    let state: Value = serde_json::from_str(&std::fs::read_to_string(root.join(".symbiote/state.json")).unwrap()).unwrap();
    assert_eq!(state["consecutive_same_err"], 2);
    assert_eq!(state["persona"], "mentor");
    let stream = std::fs::read_to_string(root.join(".symbiote/stream.jsonl")).unwrap();
    assert!(stream.contains(r#""event":"stall_detected""#));

    // The learner fixes the code; the hook reports a run; the daemon re-probes and goes green.
    let sol = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tracks/ts/slices/00-anchor/solution/src/money.ts");
    std::fs::copy(sol, root.join("src/money.ts")).unwrap();
    emit(&root, "bun test src/money.test.ts", 0);
    // complete_slice pushes the card, then the unlocked drill.
    let card = ui.until(30, "glossary card", |v| v["type"] == "upsert_glossary" && !v["story"].is_null());
    let quiz = ui.until(5, "drill unlocked", |v| v["type"] == "push_quiz" && !v["quiz"].is_null());
    assert_eq!(quiz["quiz"]["kind"], "drill");
    assert!(card["terms"].as_array().unwrap().iter().any(|t| t["term"] == "Integer cents"));
    assert!(std::fs::read_to_string(root.join(".symbiote/glossary.md")).unwrap().contains("**Integer cents**"));
    let events = std::fs::read_to_string(root.join(".symbiote/events.jsonl")).unwrap();
    assert!(events.contains(r#""event":"slice_pass""#) && events.contains(r#""interval_days":1"#));

    // Answer with the authored index (the widget shuffles; the wire carries the original).
    let id = quiz["quiz"]["id"].as_str().unwrap().to_string();
    ui.send(&format!(r#"{{"type":"quiz_answer","quiz_id":"{id}","choice":1}}"#));
    let res = ui.until(5, "quiz result", |v| v["type"] == "quiz_result");
    assert_eq!(res["correct"], true);
    ui.send(r#"{"type":"advance"}"#);
    ui.until(30, "slice 01 materialized", |v| tasks(v) && v["slice_id"] == "01-discriminated-union" && v["probe_status"] == "red");
    assert!(std::fs::read_to_string(root.join(".symbiote/blueprint.json")).unwrap().contains(r#""status": "fog""#));

    // Logging contract: structured JSONL with the required fields.
    let day = std::fs::read_dir(root.join("logs/symbiote-daemon")).unwrap().next().unwrap().unwrap().path();
    let first: Value = serde_json::from_str(std::fs::read_to_string(day).unwrap().lines().next().unwrap()).unwrap();
    for k in ["ts", "level", "component", "event", "session_id", "pid"] {
        assert!(!first[k].is_null(), "log line missing {k}");
    }
}

/// Minimal OpenAI-compatible server. Flip `up` to false to simulate spark dying mid-session.
fn fake_llm(up: Arc<AtomicBool>) -> String {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/v1", l.local_addr().unwrap());
    std::thread::spawn(move || {
        for mut s in l.incoming().flatten() {
            let mut buf = [0u8; 65536];
            let mut req = Vec::new();
            // Read headers + body (Content-Length) — enough for ureq's requests.
            loop {
                let n = s.read(&mut buf).unwrap_or(0);
                req.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&req);
                if let Some(h) = text.find("\r\n\r\n") {
                    let len = text[..h].lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0))).unwrap_or(0);
                    if req.len() >= h + 4 + len {
                        break;
                    }
                }
                if n == 0 {
                    break;
                }
            }
            let text = String::from_utf8_lossy(&req);
            let (status, body) = if !up.load(Ordering::SeqCst) {
                ("503 Service Unavailable", "{}".to_string())
            } else if text.starts_with("GET /v1/models") {
                ("200 OK", r#"{"data":[{"id":"fake-model"}]}"#.to_string())
            } else if text.contains("Candidate answer") {
                let grade = r#"{"grade":"solid","misconception_shape":"","feedback":"Correct: the never-typed default fails to compile.","trigger_words":["exhaustiveness check"],"avoid":"it catches stuff","say":"the compiler proves every variant is handled","story":"Events grew a variant. assertNever made the missing branch a compile error."}"#;
                ("200 OK", serde_json::json!({"choices":[{"message":{"content":grade}}],"usage":{"completion_tokens":40}}).to_string())
            } else {
                ("200 OK", r#"{"choices":[{"message":{"content":"Check `Number.isInteger(cents)` first."}}],"usage":{"completion_tokens":7}}"#.to_string())
            };
            let _ = write!(s, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        }
    });
    url
}

#[test]
fn tier1_focus_then_mid_session_fallback() {
    let root = workspace("t1");
    let up = Arc::new(AtomicBool::new(true));
    let _d = spawn(&root, &fake_llm(up.clone()));
    let mut ui = Ui::connect(&root);
    let mode = ui.until(5, "mode", |v| v["type"] == "mode");
    assert_eq!(mode["offline"], false);
    assert_eq!(mode["label"], "tier1 · fake-model");

    ui.send(r#"{"type":"start_track","track":"ts"}"#);
    let red = ui.until(30, "slice red", |v| tasks(v) && v["probe_status"] == "red");
    assert!(red["lesson"].is_null(), "Tier 1 teaches in the Focus Region, not the Tier 0 lesson");

    ui.send(r#"{"type":"input","text":"/hint"}"#);
    let f = ui.until(20, "focus answer", |v| v["type"] == "set_focus" && v["busy"] == false && v["text"].as_str().unwrap_or("").contains("Number.isInteger"));
    assert!(f["text"].as_str().unwrap().lines().count() <= 15);

    // spark goes away: two consecutive 5xx → Tier 0, chat disabled.
    up.store(false, Ordering::SeqCst);
    ui.send(r#"{"type":"input","text":"why is it red?"}"#);
    ui.until(20, "first failure surfaced", |v| v["type"] == "set_focus" && v["text"].as_str().unwrap_or("").contains("failed"));
    ui.send(r#"{"type":"input","text":"still red?"}"#);
    let off = ui.until(20, "fallback", |v| v["type"] == "mode" && v["offline"] == true);
    assert_eq!(off["label"], "[offline: drill mode]");
    let state: Value = serde_json::from_str(&std::fs::read_to_string(root.join(".symbiote/state.json")).unwrap()).unwrap();
    assert_eq!(state["llm_mode"], "tier0_offline");
}

fn solve(root: &Path, slice: &str, file: &str) {
    let sol = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tracks/ts/slices").join(slice).join("solution").join(file);
    std::fs::copy(sol, root.join(file)).unwrap();
}

/// Layer 2 LSP counts reach the Task List; a fluent ring-1 green earns one interview question
/// (WORKFLOW.md persona 4) while the anchor slice and stalled slices never do.
#[test]
fn tier1_lsp_diag_and_auto_interviewer() {
    let root = workspace("t1i");
    let _d = spawn(&root, &fake_llm(Arc::new(AtomicBool::new(true))));
    let mut ui = Ui::connect(&root);
    ui.until(5, "tier1", |v| v["type"] == "mode" && v["offline"] == false);
    ui.send(r#"{"type":"start_track","track":"ts"}"#);
    ui.until(30, "slice 00 red", |v| tasks(v) && v["slice_id"] == "00-anchor" && v["probe_status"] == "red");

    // What the Neovim shim sends (one line per connection).
    let mut nv = UnixStream::connect(sock_path(&root)).unwrap();
    writeln!(nv, r#"{{"type":"lsp_diag","path":"src/money.ts","errors":2}}"#).unwrap();
    drop(nv);
    let t = ui.until(10, "lsp errors on card", |v| tasks(v) && !v["lsp_errors"].as_array().unwrap().is_empty());
    assert_eq!(t["lsp_errors"][0], "src/money.ts: 2 errors");
    assert!(std::fs::read_to_string(root.join(".symbiote/stream.jsonl")).unwrap().contains(r#""event":"lsp_diag""#));
    let mut nv = UnixStream::connect(sock_path(&root)).unwrap();
    writeln!(nv, r#"{{"type":"lsp_diag","path":"src/money.ts","errors":0}}"#).unwrap();
    drop(nv);
    ui.until(10, "lsp errors cleared", |v| tasks(v) && v["lsp_errors"].as_array().unwrap().is_empty());

    // Ring 0 anchor: green → supervisor review, not an interview.
    solve(&root, "00-anchor", "src/money.ts");
    emit(&root, "bun test src/money.test.ts", 0);
    ui.until(30, "drill 00", |v| v["type"] == "push_quiz" && !v["quiz"].is_null());
    let state: Value = serde_json::from_str(&std::fs::read_to_string(root.join(".symbiote/state.json")).unwrap()).unwrap();
    assert_ne!(state["persona"], "interviewer", "never interview on the anchor slice");
    ui.send(r#"{"type":"advance"}"#);
    ui.until(30, "slice 01 red", |v| tasks(v) && v["slice_id"] == "01-discriminated-union" && v["probe_status"] == "red");

    // Ring 1, green on the first run → interviewer.
    solve(&root, "01-discriminated-union", "src/events.ts");
    emit(&root, "bun test src/events.test.ts", 0);
    ui.until(30, "drill 01", |v| v["type"] == "push_quiz" && !v["quiz"].is_null());
    let t = Instant::now();
    loop {
        let state: Value = serde_json::from_str(&std::fs::read_to_string(root.join(".symbiote/state.json")).unwrap()).unwrap();
        if state["persona"] == "interviewer" {
            break;
        }
        assert!(t.elapsed() < Duration::from_secs(10), "fluent green should switch to the interviewer, got {}", state["persona"]);
        std::thread::sleep(Duration::from_millis(100));
    }
    // The next free-form input is graded as the answer.
    ui.send(r#"{"type":"input","text":"Exhaustive switch with assertNever catches new variants at compile time."}"#);
    ui.until(20, "graded feedback", |v| v["type"] == "set_focus" && v["text"].as_str().unwrap_or("").contains("never-typed"));
    let events = std::fs::read_to_string(root.join(".symbiote/events.jsonl")).unwrap();
    assert!(events.contains(r#""event":"mock_grade""#) && events.contains(r#""grade":"solid""#));
}
