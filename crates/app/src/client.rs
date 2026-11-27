//! symbiote.sock client. Spawns the daemon when it isn't running and reconnects
//! (respawning if needed) when the connection drops — the UI never dies with the daemon.

use futures::channel::mpsc::UnboundedSender;
use serde_json::json;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use symbiote_core::daemon::sock_path;
use symbiote_core::model::{ClientMsg, UiMsg};
use symbiote_core::util::log;

#[derive(Clone)]
pub struct Client {
    writer: Arc<Mutex<Option<UnixStream>>>,
}

/// The `symbiote` CLI ships next to `symbiote-app`; $SYMBIOTE_BIN overrides.
pub fn daemon_bin() -> PathBuf {
    std::env::var_os("SYMBIOTE_BIN").map(PathBuf::from).unwrap_or_else(|| {
        std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("symbiote"))).unwrap_or_else(|| PathBuf::from("symbiote"))
    })
}

fn connect_or_spawn(root: &Path) -> std::io::Result<UnixStream> {
    let sock = sock_path(root);
    if let Ok(s) = UnixStream::connect(&sock) {
        return Ok(s);
    }
    let bin = daemon_bin();
    let child = Command::new(&bin).args(["daemon", "--root"]).arg(root).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).process_group(0).spawn()?;
    log("info", "worker_spawned", json!({"worker": "daemon", "pid": child.id(), "exe": bin}));
    let t = Instant::now();
    loop {
        match UnixStream::connect(&sock) {
            Ok(s) => return Ok(s),
            Err(e) if t.elapsed() > Duration::from_secs(5) => return Err(e),
            Err(_) => std::thread::sleep(Duration::from_millis(50)),
        }
    }
}

impl Client {
    pub fn start(root: PathBuf, tx: UnboundedSender<UiMsg>) -> Self {
        let writer: Arc<Mutex<Option<UnixStream>>> = Arc::new(Mutex::new(None));
        let w = writer.clone();
        std::thread::spawn(move || loop {
            match connect_or_spawn(&root) {
                Ok(stream) => {
                    log("info", "ipc_connected", json!({"sock": sock_path(&root)}));
                    let Ok(mut out) = stream.try_clone() else { continue };
                    let _ = out.write_all(b"{\"type\":\"hello\",\"role\":\"ui\"}\n");
                    *w.lock().unwrap() = Some(out);
                    for line in BufReader::new(stream).lines() {
                        let Ok(line) = line else { break };
                        match serde_json::from_str::<UiMsg>(&line) {
                            Ok(m) => {
                                if tx.unbounded_send(m).is_err() {
                                    return; // UI is gone
                                }
                            }
                            Err(e) => log("warn", "framing_error", json!({"raw": line.chars().take(256).collect::<String>(), "error": e.to_string()})),
                        }
                    }
                    *w.lock().unwrap() = None;
                    log("warn", "ipc_disconnected", json!({"retry_in_ms": 500}));
                }
                Err(e) => log("error", "ipc_connect_failed", json!({"message": e.to_string(), "retry_in_ms": 500})),
            }
            if tx.is_closed() {
                return;
            }
            std::thread::sleep(Duration::from_millis(500));
        });
        Self { writer }
    }

    pub fn send(&self, m: &ClientMsg) {
        let Ok(line) = serde_json::to_string(m) else { return };
        if let Some(w) = self.writer.lock().unwrap().as_mut() {
            if let Err(e) = w.write_all(format!("{line}\n").as_bytes()) {
                log("warn", "ipc_send_failed", json!({"message": e.to_string()}));
            }
        }
    }
}
