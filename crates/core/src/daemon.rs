//! The single control plane. One actor thread owns all state and all `.symbiote/` writes;
//! sockets, the file watcher, probes, gates, and LLM calls feed it `Msg`s.

use crate::llm::{self, LlmConfig, LlmError};
use crate::model::*;
use crate::srs;
use crate::store::Store;
use crate::track::{self, Track};
use crate::util::{err_hash, log, now_iso, now_ms, parse_iso};
use crate::verify::{self, GateReport};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::time::{Duration, Instant};

const STALL_CHURN_SEC: u64 = 180;
const STALL_SAME_ERR: u32 = 2;
const RECOVERY_PROBE: Duration = Duration::from_secs(60);
const TICK: Duration = Duration::from_secs(5);
const COLD_RETURN_MS: i64 = 7 * 86_400_000;
const IDLE_EXIT: Duration = Duration::from_secs(30);
const SELF_WRITE_QUIET: Duration = Duration::from_secs(2);
const MAX_LINE: u64 = 256 * 1024;
/// Green within this many probe runs, with no stall, counts as fluent enough for an interview question.
const FLUENT_PROBE_RUNS: u32 = 3;
const OFFLINE_LABEL: &str = "[offline: drill mode]";

static SHUTDOWN: AtomicBool = AtomicBool::new(false);
static SIGNAL: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

/// `.symbiote/symbiote.sock`, or a temp-dir fallback when the path exceeds sun_path (104 bytes).
pub fn sock_path(root: &Path) -> PathBuf {
    let p = root.join(".symbiote").join("symbiote.sock");
    if p.as_os_str().len() < 100 {
        return p;
    }
    // Inside a per-user dir: $TMPDIR is per-user on macOS, but may fall back to shared /tmp.
    std::env::temp_dir().join(format!("symbiote-{}", unsafe { libc::getuid() })).join(format!("{}.sock", err_hash(&root.to_string_lossy())))
}

/// Creates the socket's parent if needed and refuses one another user could write into.
fn ensure_private_parent(sock: &Path) -> io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};
    let Some(dir) = sock.parent() else { return Ok(()) };
    if !dir.exists() {
        let _ = fs::DirBuilder::new().mode(0o700).create(dir);
    }
    let m = fs::symlink_metadata(dir)?;
    if !m.is_dir() || m.uid() != unsafe { libc::getuid() } || m.mode() & 0o022 != 0 {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, format!("{} is not a private directory owned by this user", dir.display())));
    }
    Ok(())
}

/// `--root`, else $SYMBIOTE_ROOT, else the nearest parent holding `.symbiote/`, else cwd.
pub fn resolve_root(arg: Option<PathBuf>) -> PathBuf {
    if let Some(r) = arg.or_else(|| std::env::var_os("SYMBIOTE_ROOT").map(PathBuf::from)) {
        return fs::canonicalize(&r).unwrap_or(r);
    }
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    cwd.ancestors().find(|a| a.join(".symbiote").is_dir()).map(Path::to_path_buf).unwrap_or(cwd)
}

pub enum Msg {
    Connected(u64, UnixStream),
    Client(u64, ClientMsg),
    Gone(u64),
    FileSaved(String),
    Probe(ProbeResult),
    Static(Vec<CheckStatus>),
    Gate { slice_id: String, jit: bool, result: Result<GateReport, String> },
    Llm { epoch: u64, kind: LlmKind, result: Result<String, LlmError> },
    Health(Result<String, LlmError>),
    Tick,
    Shutdown,
}

pub struct ProbeResult {
    pub cmd: String,
    pub exit: i32,
    pub excerpt: String,
    pub duration_ms: u64,
    pub source: &'static str,
}

#[derive(Clone, Debug)]
pub enum LlmKind {
    Answer,
    Kickoff,
    Mentor,
    Hint,
    WaitWhat,
    Review,
    MockQuestion,
    MockGrade { question: String },
    Retro,
    JitTrack { brief: String, attempt: u32 },
    JitNext { attempt: u32 },
}

pub struct Daemon {
    root: PathBuf,
    store: Store,
    st: State,
    track: Option<Track>,
    tx: Sender<Msg>,
    clients: HashMap<u64, UnixStream>,
    ui: HashSet<u64>,
    ui_gone_at: Option<Instant>,
    cfg: LlmConfig,
    model: Option<String>,
    llm_errs: u32,
    epoch: u64,
    last_health: Instant,
    probe_running: bool,
    gate_running: Option<String>,
    last_probe_at: Instant,
    dirty: bool,
    stall_emitted: bool,
    focus: String,
    focus_busy: bool,
    quiz: Option<QuizView>,
    review_pause: bool,
    excerpt: Option<String>,
    mock_q: Option<String>,
    /// Stalled or cold-returned this slice: never ambush with an interview question.
    unprimed: bool,
    /// Latest Neovim LSP error count per root-relative path (Layer 2).
    lsp: BTreeMap<String, u32>,
    hint_on: bool,
    anchor: Option<String>,
    notice: Option<String>,
    card: Option<(Vec<String>, Swap)>,
    jit_brief: Option<String>,
    jit_retried: bool,
    /// The watcher echoes the daemon's own scaffold/slice writes; ignore saves until then.
    quiet_until: Instant,
}

// ── process entry ──────────────────────────────────────────────────────────

extern "C" fn on_signal(sig: libc::c_int) {
    SIGNAL.store(sig, Ordering::SeqCst);
    SHUTDOWN.store(true, Ordering::SeqCst);
}

pub fn run(root: PathBuf) -> io::Result<()> {
    let started = Instant::now();
    let store = Store::open(&root)?;
    crate::shell::install(&store.dir)?;
    let sock = sock_path(&root);
    ensure_private_parent(&sock)?;
    if UnixStream::connect(&sock).is_ok() {
        log("info", "daemon_already_running", json!({"sock": sock}));
        return Ok(());
    }
    let _ = fs::remove_file(&sock);
    let listener = UnixListener::bind(&sock)?;
    fs::set_permissions(&sock, fs::Permissions::from_mode(0o600))?;
    // SAFETY: the handler only stores into atomics.
    unsafe {
        libc::signal(libc::SIGTERM, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGINT, on_signal as *const () as libc::sighandler_t);
    }

    let state_bytes = fs::read(store.path("state.json")).unwrap_or_default();
    log("info", "config_loaded", json!({"path": store.path("state.json"), "content_hash": err_hash(&String::from_utf8_lossy(&state_bytes)), "root": root, "sock": sock}));

    let (tx, rx) = channel::<Msg>();
    spawn_accept(listener, tx.clone());
    let _watcher = spawn_watcher(&root, tx.clone());
    let ttx = tx.clone();
    std::thread::spawn(move || {
        let mut last = Instant::now();
        loop {
            std::thread::sleep(Duration::from_millis(250));
            if SHUTDOWN.load(Ordering::SeqCst) {
                let _ = ttx.send(Msg::Shutdown);
                return;
            }
            if last.elapsed() >= TICK {
                last = Instant::now();
                if ttx.send(Msg::Tick).is_err() {
                    return;
                }
            }
        }
    });

    let cfg = llm::config();
    log("info", "llm_config", cfg.describe());
    let health = llm::health(&cfg);
    let mut d = Daemon::new(root, store, tx, cfg);
    d.apply_health(health, "boot");
    d.boot();
    for msg in rx {
        if !d.handle(msg) {
            break;
        }
    }
    let _ = fs::remove_file(&sock);
    let sig = SIGNAL.load(Ordering::SeqCst);
    log("info", "process_exit", json!({"exit_code": 0, "signal": (sig != 0).then_some(sig), "wall_ms": started.elapsed().as_millis() as u64, "peak_rss_bytes": crate::util::peak_rss_bytes()}));
    Ok(())
}

fn spawn_accept(listener: UnixListener, tx: Sender<Msg>) {
    std::thread::spawn(move || {
        for (id, conn) in (1u64..).zip(listener.incoming().flatten()) {
            let Ok(writer) = conn.try_clone() else { continue };
            let _ = writer.set_write_timeout(Some(Duration::from_secs(1)));
            if tx.send(Msg::Connected(id, writer)).is_err() {
                return;
            }
            let tx = tx.clone();
            std::thread::spawn(move || {
                let mut reader = BufReader::new(conn);
                loop {
                    let mut line = String::new();
                    match reader.by_ref().take(MAX_LINE).read_line(&mut line) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            let start = Instant::now();
                            match serde_json::from_str::<ClientMsg>(line.trim()) {
                                Ok(m) => {
                                    log("debug", "ipc_received", json!({"direction": "in", "conn": id, "size_bytes": n, "latency_ms": start.elapsed().as_millis() as u64}));
                                    if tx.send(Msg::Client(id, m)).is_err() {
                                        return;
                                    }
                                }
                                Err(e) => {
                                    let raw: String = line.chars().take(256).collect();
                                    log("warn", "framing_error", json!({"conn": id, "raw": raw, "error": e.to_string()}));
                                }
                            }
                        }
                    }
                }
                let _ = tx.send(Msg::Gone(id));
            });
        }
    });
}

fn ignored(rel: &str) -> bool {
    rel.split('/').any(|c| matches!(c, ".git" | ".symbiote" | "target" | "node_modules")) || rel.ends_with('~') || rel.ends_with(".swp")
}

fn spawn_watcher(root: &Path, tx: Sender<Msg>) -> Option<notify::RecommendedWatcher> {
    use notify::{EventKind, RecursiveMode, Watcher};
    let base = fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let mut last: HashMap<PathBuf, Instant> = HashMap::new();
    let b2 = base.clone();
    let mut w = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        let Ok(ev) = res else { return };
        if !matches!(ev.kind, EventKind::Create(_) | EventKind::Modify(_)) {
            return;
        }
        for p in ev.paths {
            let Ok(rel) = p.strip_prefix(&b2) else { continue };
            let rel_s = rel.to_string_lossy().to_string();
            if rel_s.is_empty() || ignored(&rel_s) || !p.is_file() {
                continue;
            }
            // ponytail: per-path 250ms throttle stands in for a trailing debounce; editors'
            // write-then-rename bursts collapse to one event, which is all churn needs.
            let now = Instant::now();
            if last.get(&p).is_some_and(|t| now.duration_since(*t) < Duration::from_millis(250)) {
                continue;
            }
            last.insert(p.clone(), now);
            let _ = tx.send(Msg::FileSaved(rel_s));
        }
    })
    .map_err(|e| log("error", "watcher_failed", json!({"message": e.to_string()})))
    .ok()?;
    w.watch(&base, RecursiveMode::Recursive).map_err(|e| log("error", "watcher_failed", json!({"message": e.to_string()}))).ok()?;
    Some(w)
}

// ── the actor ──────────────────────────────────────────────────────────────

impl Daemon {
    pub fn new(root: PathBuf, store: Store, tx: Sender<Msg>, cfg: LlmConfig) -> Self {
        let st = store.load_state();
        let track = st.track_dir.as_deref().and_then(|d| track::load(Path::new(d)).ok());
        Self {
            root,
            store,
            st,
            track,
            tx,
            clients: HashMap::new(),
            ui: HashSet::new(),
            ui_gone_at: None,
            cfg,
            model: None,
            llm_errs: 0,
            epoch: 0,
            last_health: Instant::now(),
            probe_running: false,
            gate_running: None,
            last_probe_at: Instant::now(),
            dirty: false,
            stall_emitted: false,
            focus: String::new(),
            focus_busy: false,
            quiz: None,
            review_pause: false,
            excerpt: None,
            mock_q: None,
            unprimed: false,
            lsp: BTreeMap::new(),
            hint_on: false,
            anchor: None,
            notice: None,
            card: None,
            jit_brief: None,
            jit_retried: false,
            quiet_until: Instant::now(),
        }
    }

    fn tier1(&self) -> bool {
        self.st.llm_mode == "tier1_spark"
    }

    fn slice(&self) -> Option<&Slice> {
        self.track.as_ref()?.slice(self.st.slice_id.as_deref()?)
    }

    fn save(&self) {
        if let Err(e) = self.store.save_state(&self.st) {
            log("error", "state_write_failed", json!({"error_type": "io", "message": e.to_string()}));
        }
    }

    fn stream(&self, v: Value) {
        let _ = self.store.append_stream(v);
    }

    fn spawn(&self, f: impl FnOnce() -> Msg + Send + 'static) {
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(f());
        });
    }

    /// Returns false to stop the loop.
    pub fn handle(&mut self, msg: Msg) -> bool {
        match msg {
            Msg::Connected(id, w) => {
                self.clients.insert(id, w);
            }
            Msg::Gone(id) => {
                self.clients.remove(&id);
                if self.ui.remove(&id) && self.ui.is_empty() {
                    self.ui_gone_at = Some(Instant::now());
                }
            }
            Msg::Client(id, m) => self.on_client(id, m),
            Msg::FileSaved(_) if Instant::now() < self.quiet_until => {}
            Msg::FileSaved(path) => {
                self.dirty = true;
                let churn = self.last_probe_at.elapsed().as_secs();
                self.stream(json!({"event": "file_saved", "path": path, "churn_sec": churn}));
            }
            Msg::Probe(r) => self.on_probe(r),
            Msg::Static(r) => self.on_static(r),
            Msg::Gate { slice_id, jit, result } => self.on_gate(slice_id, jit, result),
            Msg::Llm { epoch, kind, result } => self.on_llm(epoch, kind, result),
            Msg::Health(r) => self.apply_health(r, "recovery"),
            Msg::Tick => return self.on_tick(),
            Msg::Shutdown => return false,
        }
        true
    }

    fn on_client(&mut self, id: u64, m: ClientMsg) {
        match m {
            ClientMsg::Hello { role } => {
                log("info", "client_hello", json!({"conn": id, "role": role}));
                if role == "ui" {
                    self.ui.insert(id);
                    self.ui_gone_at = None;
                    self.push_all();
                }
            }
            ClientMsg::CmdEnd { cmd, exit_code, duration_ms, pipestatus, output_head } => {
                self.stream(json!({"event": "cmd_end", "source": "zsh", "cmd": cmd, "exit_code": exit_code, "pipestatus": pipestatus, "duration_ms": duration_ms}));
                if self.is_probe_like(&cmd) {
                    match output_head {
                        // `symbiote run` already captured output through a pipe.
                        Some(out) if self.st.probe_cmd.as_deref() == Some(cmd.trim()) => self.on_probe(ProbeResult {
                            excerpt: verify::failure_excerpt(&out),
                            cmd,
                            exit: exit_code,
                            duration_ms,
                            source: "symbiote_run",
                        }),
                        _ => self.run_probe("zsh_hook"),
                    }
                }
            }
            ClientMsg::Input { text } => self.on_input(text.trim().to_string()),
            ClientMsg::QuizAnswer { quiz_id, choice } => self.on_quiz_answer(&quiz_id, choice),
            ClientMsg::RunProbe => self.run_probe("ui"),
            ClientMsg::StartTrack { track } => self.start_track(&track),
            ClientMsg::Advance => self.advance(),
            ClientMsg::LspDiag { path, errors } => {
                let path: String = path.chars().take(512).collect();
                self.stream(json!({"event": "lsp_diag", "path": path, "errors": errors}));
                let changed = if errors == 0 { self.lsp.remove(&path).is_some() } else { self.lsp.insert(path, errors) != Some(errors) };
                if changed {
                    self.push_tasks();
                }
            }
        }
    }

    fn is_probe_like(&self, cmd: &str) -> bool {
        let Some(probe) = self.st.probe_cmd.as_deref() else { return false };
        let words = |s: &str| s.split_whitespace().map(str::to_string).collect::<Vec<_>>();
        let (c, p) = (words(cmd), words(probe));
        c == p || (p.len() >= 2 && c.len() >= 2 && c[..2] == p[..2])
    }

    // ── broadcasting ──

    fn send_ui(&mut self, m: &UiMsg) {
        let line = match serde_json::to_string(m) {
            Ok(l) => l + "\n",
            Err(_) => return,
        };
        let mut dead = vec![];
        for id in &self.ui {
            if let Some(w) = self.clients.get_mut(id) {
                if w.write_all(line.as_bytes()).is_err() {
                    dead.push(*id);
                }
            }
        }
        log("debug", "ipc_sent", json!({"direction": "out", "size_bytes": line.len(), "message_type": serde_json::to_value(m).ok().and_then(|v| v["type"].as_str().map(str::to_string)), "clients": self.ui.len()}));
        for id in dead {
            self.ui.remove(&id);
            self.clients.remove(&id);
        }
    }

    fn push_mode(&mut self) {
        let offline = !self.tier1();
        let label = if offline { OFFLINE_LABEL.to_string() } else { format!("tier1 · {}", self.model.clone().unwrap_or_default()) };
        self.send_ui(&UiMsg::Mode { llm_mode: self.st.llm_mode.clone(), offline, label });
    }

    fn push_tasks(&mut self) {
        if self.track.is_none() {
            let message = self.notice.clone();
            self.send_ui(&UiMsg::TrackPicker { tracks: track::list_tracks(), allow_custom: self.tier1(), message });
            return;
        }
        let v = self.tasks_view();
        self.send_ui(&UiMsg::SetTasks(v));
    }

    fn push_glossary(&mut self) {
        let v = self.glossary_view();
        self.send_ui(&UiMsg::UpsertGlossary(v));
    }

    fn push_quiz(&mut self) {
        let quiz = self.quiz.clone();
        self.send_ui(&UiMsg::PushQuiz { quiz });
    }

    fn set_focus(&mut self, text: impl Into<String>, busy: bool) {
        self.focus = llm::clamp_focus(&text.into());
        self.focus_busy = busy;
        let m = UiMsg::SetFocus { text: self.focus.clone(), busy };
        self.send_ui(&m);
    }

    fn push_all(&mut self) {
        self.push_mode();
        self.push_tasks();
        self.push_glossary();
        self.push_quiz();
        let m = UiMsg::SetFocus { text: self.focus.clone(), busy: self.focus_busy };
        self.send_ui(&m);
    }

    pub fn tasks_view(&self) -> TasksView {
        let m = srs::metrics(&self.store.load_events(), now_ms());
        let Some(s) = self.slice() else {
            return TasksView { capstone: self.st.capstone.clone().unwrap_or_default(), notice: self.notice.clone(), probe_status: if self.gate_running.is_some() { "verifying".into() } else { "unknown".into() }, due_items: m.due_items, open_misconceptions: m.open_misconceptions, ..Default::default() };
        };
        let ran = self.st.slice_probe_runs > 0;
        let green = self.st.probe_status == "green";
        let checks_ok = !self.st.static_status.is_empty() && self.st.static_status.iter().all(|c| c.status == "pass") || (s.static_checks.is_empty() && green);
        let steps = vec![
            Step { text: format!("Run `{}` and read the red", s.probe_cmd), done: ran },
            Step { text: format!("Edit {} until the probe is green", s.target_files.first().cloned().unwrap_or_default()), done: green },
            Step { text: "Static checks clean".into(), done: checks_ok && green },
            Step { text: format!("Drill: {}", s.tier0_drill.prompt), done: self.st.slice_complete && self.quiz.is_none() },
        ];
        TasksView {
            capstone: self.track.as_ref().map(|t| t.def.capstone_goal.clone()).unwrap_or_default(),
            ring: self.st.ring,
            ring_name: self.track.as_ref().map(|t| t.ring_slug(self.st.ring)).unwrap_or_default(),
            slice_id: s.id.clone(),
            goal: s.goal.clone(),
            target_files: s.target_files.clone(),
            probe_cmd: s.probe_cmd.clone(),
            probe_status: if self.gate_running.is_some() { "verifying".into() } else if self.probe_running { "running".into() } else { self.st.probe_status.clone() },
            static_checks: self.st.static_status.clone(),
            steps,
            due_items: m.due_items,
            open_misconceptions: m.open_misconceptions,
            hint: self.hint_on.then(|| s.hint.clone()),
            stderr_pointer: self.hint_on.then(|| self.excerpt.as_deref().and_then(verify::stderr_pointer)).flatten(),
            // Scaffolding for reaching green; once complete, the drill gets the space.
            lesson: (!self.tier1() && !self.st.slice_complete).then(|| s.lesson.clone()).flatten(),
            anchor: self.anchor.clone(),
            notice: self.notice.clone(),
            lsp_errors: self.lsp.iter().map(|(p, n)| format!("{p}: {n} error{}", if *n == 1 { "" } else { "s" })).collect(),
        }
    }

    pub fn glossary_view(&self) -> GlossaryView {
        let earned = self.store.load_glossary();
        let slice = self.slice();
        let active: Vec<String> = slice.map(|s| s.glossary.iter().map(|g| g.term.to_lowercase()).collect()).unwrap_or_default();
        let (mut first, mut rest): (Vec<_>, Vec<_>) = earned.into_iter().partition(|e| active.contains(&e.term.to_lowercase()));
        rest.reverse(); // most recently earned next
        first.extend(rest);
        first.truncate(10);
        let (trigger_words, say_this_not_that) = match (&self.card, slice) {
            (Some((tw, swap)), _) => (tw.clone(), vec![swap.clone()]),
            (None, Some(s)) => (s.trigger_words.clone(), s.say_this_not_that.clone()),
            _ => (vec![], vec![]),
        };
        GlossaryView { terms: first, trigger_words, say_this_not_that, story: self.store.latest_story() }
    }

    // ── health / tier switching ──

    fn apply_health(&mut self, r: Result<String, LlmError>, phase: &str) {
        self.last_health = Instant::now();
        match r {
            Ok(model) => {
                let was = self.st.llm_mode.clone();
                self.st.llm_mode = "tier1_spark".into();
                self.model = Some(model);
                self.llm_errs = 0;
                if was != self.st.llm_mode {
                    log("info", "llm_mode_changed", json!({"llm_mode": "tier1_spark", "phase": phase}));
                }
                if phase != "boot" {
                    self.save();
                    self.push_all();
                }
            }
            Err(e) => {
                if phase == "boot" || self.tier1() {
                    self.go_offline(&e.message);
                }
            }
        }
    }

    /// Non-blocking degradation: chat off, drills on, in-flight LLM work abandoned.
    fn go_offline(&mut self, reason: &str) {
        let changed = self.st.llm_mode != "tier0_offline";
        self.st.llm_mode = "tier0_offline".into();
        self.epoch += 1;
        self.mock_q = None;
        self.focus.clear();
        self.focus_busy = false;
        log("warn", "llm_mode_changed", json!({"llm_mode": "tier0_offline", "reason": reason}));
        if changed || !self.ui.is_empty() {
            self.save();
            self.push_all();
        }
    }

    fn on_tick(&mut self) -> bool {
        if !self.tier1() && self.last_health.elapsed() >= RECOVERY_PROBE {
            self.last_health = Instant::now();
            let cfg = self.cfg.clone();
            self.spawn(move || Msg::Health(llm::health(&cfg)));
        }
        if self.slice().is_some() && self.dirty && !self.st.slice_complete {
            self.st.churn_sec = self.last_probe_at.elapsed().as_secs();
            if self.st.churn_sec >= STALL_CHURN_SEC && !self.stall_emitted {
                self.stall("churn_sec", self.st.churn_sec as u32);
            }
        }
        if self.ui_gone_at.is_some_and(|t| t.elapsed() >= IDLE_EXIT) {
            log("info", "idle_exit", json!({"reason": "no ui clients"}));
            return false;
        }
        true
    }

    // ── boot / cold return ──

    pub fn boot(&mut self) {
        log("info", "boot", json!({"track": self.st.track, "slice_id": self.st.slice_id, "llm_mode": self.st.llm_mode}));
        self.save();
        let Some(slice) = self.slice().cloned() else {
            if self.track.is_some() {
                self.next_slice();
            }
            return;
        };
        let away = self.st.last_probe_ts.as_deref().and_then(parse_iso).map(|t| now_ms() - t).unwrap_or(0);
        if away >= COLD_RETURN_MS {
            self.cold_return(&slice);
        } else {
            self.offer_review();
        }
    }

    fn cold_return(&mut self, slice: &Slice) {
        self.unprimed = true;
        log("info", "cold_return", json!({"slice_id": slice.id, "last_probe_ts": self.st.last_probe_ts}));
        let goal = self.track.as_ref().map(|t| t.def.capstone_goal.clone()).unwrap_or_default();
        let story = self.store.latest_story().unwrap_or_else(|| "(no stories yet)".into());
        self.anchor = Some(format!("{goal}\n{story}\n{}", slice.target_files.first().cloned().unwrap_or_default()));
        if let Some(cmd) = self.st.last_green_probe_cmd.clone() {
            let root = self.root.clone();
            self.spawn(move || {
                let r = verify::run_shell(&cmd, &root, verify::PROBE_TIMEOUT);
                Msg::Probe(ProbeResult { excerpt: verify::failure_excerpt(&r.output), cmd, exit: r.exit, duration_ms: r.duration_ms, source: "cold_return" })
            });
        }
        let events = self.store.load_events();
        for e in srs::stagger(&events, now_ms()) {
            let _ = self.store.append_event(&e);
        }
        // Warmup on code already written: the most recent passed slice's drill.
        let warm = self.st.passed_slices.last().and_then(|id| self.track.as_ref()?.slice(id)).cloned();
        if let Some(w) = warm {
            self.quiz = Some(quiz_view(&w, "warmup", &w.concepts[0]));
        }
    }

    // ── tracks & slices ──

    fn start_track(&mut self, id: &str) {
        if self.track.is_some() {
            return;
        }
        let Some(dir) = track::find_bundled(id) else {
            if self.tier1() {
                self.jit_brief = Some(id.to_string());
                self.ask(LlmKind::JitTrack { brief: id.to_string(), attempt: 0 }, jit_track_prompt(id, None));
            } else {
                self.notice = Some(format!("No pre-built track for '{id}' and LLM is offline. Start spark or set SYMBIOTE_LLM_URL."));
                self.push_tasks();
            }
            return;
        };
        self.begin_track(dir);
    }

    fn begin_track(&mut self, dir: PathBuf) {
        let t = match track::load(&dir) {
            Ok(t) => t,
            Err(e) => return self.fail_notice("track_load_failed", &e),
        };
        let scaffold = dir.join("scaffold");
        let clash = track::collisions(&scaffold, &self.root);
        if !clash.is_empty() {
            return self.fail_notice("scaffold_collision", &format!("These files already exist and differ from the track scaffold: {}", clash.join(", ")));
        }
        self.quiet_until = Instant::now() + SELF_WRITE_QUIET;
        if let Err(e) = track::copy_overlay(&scaffold, &self.root) {
            return self.fail_notice("scaffold_write_failed", &e);
        }
        self.st.track = Some(t.def.track.clone());
        self.st.capstone = Some(t.def.capstone.clone());
        self.st.track_dir = Some(dir.to_string_lossy().to_string());
        self.st.ring = t.def.rings.first().map(|r| r.ring).unwrap_or(0);
        self.notice = None;
        log("info", "track_started", json!({"track": t.def.track, "slices": t.slices.len()}));
        self.track = Some(t);
        self.save_blueprint();
        self.save();
        self.next_slice();
    }

    fn fail_notice(&mut self, event: &str, msg: &str) {
        log("error", event, json!({"error_type": event, "message": msg}));
        self.notice = Some(msg.to_string());
        self.push_tasks();
    }

    fn save_blueprint(&self) {
        let Some(t) = &self.track else { return };
        let rings = t
            .def
            .rings
            .iter()
            .map(|r| {
                let status = if r.ring < self.st.ring { "done" } else if r.ring == self.st.ring { "active" } else { "fog" };
                BlueprintRing { ring: r.ring, slug: r.slug.clone(), goal: r.goal.clone(), status: status.into(), slices: if status == "fog" { vec![] } else { r.slices.clone() } }
            })
            .collect();
        let bp = Blueprint { track: t.def.track.clone(), capstone: t.def.capstone.clone(), baseline_zpd: self.st.baseline_zpd.clone(), rings };
        let _ = self.store.save_blueprint(&bp);
    }

    /// Pick the next slice, honoring exit criteria at ring boundaries and parked prerequisites.
    fn next_slice(&mut self) {
        let Some(t) = &self.track else { return };
        let cur = self.st.slice_id.as_deref().and_then(|id| t.slices.iter().position(|s| s.id == id));
        let done = |id: &String| self.st.passed_slices.contains(id) || self.st.parked_slices.contains(id);
        let mut cand = t.slices.iter().skip(cur.map_or(0, |i| i + 1)).find(|s| !done(&s.id)).cloned();
        if cand.is_none() {
            cand = self.st.parked_slices.first().and_then(|p| t.slice(p)).cloned();
        }
        let Some(mut next) = cand else { return self.track_done() };
        if let Some(p) = next.requires.iter().find(|r| self.st.parked_slices.contains(r)) {
            next = t.slice(p).cloned().unwrap_or(next);
        }
        self.st.parked_slices.retain(|p| p != &next.id);
        if next.ring > self.st.ring {
            let m = srs::metrics(&self.store.load_events(), now_ms());
            if srs::must_review(&m) && self.push_review(&m.queue) {
                self.review_pause = true;
                self.notice = Some(format!("Review first: {} due, {} open. Ring {} unlocks after.", m.due_items, m.open_misconceptions, next.ring));
                self.push_tasks();
                return;
            }
            self.review_pause = false;
            self.st.ring = next.ring;
            self.save_blueprint();
        }
        self.materialize(next);
    }

    fn is_jit(&self) -> bool {
        self.st.track_dir.as_deref().is_some_and(|d| d.contains("/.symbiote/jit/"))
    }

    fn track_done(&mut self) {
        if self.is_jit() && self.tier1() {
            self.ask(LlmKind::JitNext { attempt: 0 }, self.jit_next_prompt(None));
            return;
        }
        self.notice = Some(if self.tier1() { "All authored slices are done. Type /mock to pressure-test what you built.".into() } else { "All authored slices are done. Reviews keep running from your drill history.".into() });
        self.st.persona = "interviewer".into();
        self.save();
        self.offer_review();
        self.push_tasks();
    }

    fn materialize(&mut self, slice: Slice) {
        let jit = self.is_jit();
        let Some(t) = &self.track else { return };
        self.gate_running = Some(slice.id.clone());
        self.notice = Some(format!("Verifying {} in an isolated worktree…", slice.id));
        let (root, dir, s) = (self.root.clone(), t.dir.clone(), slice.clone());
        self.spawn(move || {
            let result = track::load(&dir).and_then(|t| verify::gate(&root, &t, &s));
            Msg::Gate { slice_id: s.id.clone(), jit, result }
        });
        self.push_tasks();
    }

    fn on_gate(&mut self, id: String, jit: bool, result: Result<GateReport, String>) {
        self.gate_running = None;
        if jit && !self.tier1() {
            log("warn", "jit_aborted", json!({"slice_id": id, "reason": "went offline"}));
            self.notice = None;
            return self.push_tasks();
        }
        if jit {
            // The JIT track dir changed on disk; reload so the slice is known.
            if let Some(d) = self.st.track_dir.clone() {
                self.track = track::load(Path::new(&d)).ok();
            }
        }
        let Some(slice) = self.track.as_ref().and_then(|t| t.slice(&id)).cloned() else { return };
        if let Err(e) = result {
            log("error", "verify_gate_failed", json!({"slice_id": id, "message": e}));
            if jit && self.tier1() && !self.jit_retried {
                self.jit_retried = true;
                return self.ask(LlmKind::JitNext { attempt: 1 }, self.jit_next_prompt(Some(&e)));
            }
            return self.fail_notice("slice_rejected", &format!("{id} failed verification and was not presented. {}", e.lines().next().unwrap_or("")));
        }
        let files = self.track.as_ref().map(|t| t.slice_dir(&id).join("files")).unwrap_or_default();
        let clash = track::collisions(&files, &self.root);
        if !clash.is_empty() {
            return self.fail_notice("slice_collision", &format!("{id} would overwrite your files: {}", clash.join(", ")));
        }
        self.quiet_until = Instant::now() + SELF_WRITE_QUIET;
        if let Err(e) = track::copy_overlay(&files, &self.root).and_then(|_| track::apply_wire(&slice, &self.root)) {
            return self.fail_notice("slice_write_failed", &e);
        }
        self.jit_retried = false;
        let yaml = self.track.as_ref().map(|t| t.slice_yaml(&id)).unwrap_or_default();
        let _ = self.store.save_slice_yaml(&id, &yaml);
        self.st.slice_id = Some(id.clone());
        self.st.ring = slice.ring;
        self.st.probe_cmd = Some(slice.probe_cmd.clone());
        self.st.probe_status = "red".into();
        self.st.consecutive_same_err = 0;
        self.st.last_err_hash = None;
        self.st.churn_sec = 0;
        self.st.slice_started_ts = Some(now_iso());
        self.st.slice_probe_runs = 0;
        self.st.slice_complete = false;
        self.st.static_status = slice.static_checks.iter().map(|c| CheckStatus { cmd: c.clone(), status: "pending".into(), output_head: String::new() }).collect();
        self.st.persona = "mentor".into(); // new concept → mentor until the first probe run
        self.quiz = None;
        self.hint_on = false;
        self.excerpt = None;
        self.card = None;
        self.anchor = None;
        self.notice = None;
        self.mock_q = None;
        self.unprimed = false;
        self.dirty = false;
        self.stall_emitted = false;
        self.last_probe_at = Instant::now();
        self.save();
        self.save_blueprint();
        log("info", "slice_materialized", json!({"slice_id": id, "ring": slice.ring}));
        self.offer_review();
        self.push_tasks();
        self.push_glossary();
        self.push_quiz();
        if self.tier1() {
            self.ask(LlmKind::Kickoff, "A new slice was just materialized and verified red. Give ONE concrete next step and the ONE runnable probe command. ≤6 lines.".into());
        }
    }

    // ── probes ──

    fn run_probe(&mut self, source: &'static str) {
        if self.probe_running || self.gate_running.is_some() {
            return;
        }
        let Some(cmd) = self.slice().map(|s| s.probe_cmd.clone()) else { return };
        self.probe_running = true;
        self.push_tasks();
        let root = self.root.clone();
        self.spawn(move || {
            let r = verify::run_shell(&cmd, &root, verify::PROBE_TIMEOUT);
            Msg::Probe(ProbeResult { excerpt: if r.exit == 0 { String::new() } else { verify::failure_excerpt(&r.output) }, cmd, exit: r.exit, duration_ms: r.duration_ms, source })
        });
    }

    fn on_probe(&mut self, r: ProbeResult) {
        let hash = (r.exit != 0).then(|| err_hash(&r.excerpt));
        self.stream(json!({"event": "cmd_end", "source": r.source, "cmd": r.cmd, "exit_code": r.exit, "err_hash": hash, "stderr_head": r.excerpt, "duration_ms": r.duration_ms}));
        if r.source == "cold_return" {
            if r.exit != 0 {
                self.notice = Some(format!("Toolchain check: last green probe now fails.\n{}", r.excerpt.lines().next().unwrap_or("")));
                self.push_tasks();
            }
            return;
        }
        self.probe_running = false;
        if self.st.probe_cmd.as_deref() != Some(r.cmd.as_str()) {
            return self.push_tasks(); // finished after a slice change
        }
        self.st.last_probe_ts = Some(now_iso());
        self.st.slice_probe_runs += 1;
        self.st.churn_sec = 0;
        self.last_probe_at = Instant::now();
        self.dirty = false;
        self.stall_emitted = false;
        let prev = std::mem::take(&mut self.st.probe_status);
        if r.exit == 0 {
            self.st.probe_status = "green".into();
            self.st.consecutive_same_err = 0;
            self.st.last_err_hash = None;
            self.excerpt = None;
            self.hint_on = false;
            self.st.last_green_probe_cmd = Some(r.cmd.clone());
            if prev != "green" && !self.st.slice_complete {
                self.st.persona = "supervisor".into();
                self.run_checks();
            }
        } else {
            self.st.probe_status = "red".into();
            self.st.consecutive_same_err = if self.st.last_err_hash == hash { self.st.consecutive_same_err + 1 } else { 1 };
            self.st.last_err_hash = hash;
            self.excerpt = Some(r.excerpt);
            if self.st.consecutive_same_err >= STALL_SAME_ERR {
                self.stall("consecutive_same_err", self.st.consecutive_same_err);
            } else if self.st.ring > 0 {
                self.st.persona = "pair".into();
            }
        }
        self.save();
        self.push_tasks();
    }

    fn stall(&mut self, reason: &str, count: u32) {
        self.stall_emitted = true;
        self.unprimed = true;
        self.stream(json!({"event": "stall_detected", "reason": reason, "count": count, "err_hash": self.st.last_err_hash}));
        log("info", "stall_detected", json!({"reason": reason, "count": count}));
        self.st.persona = "mentor".into();
        self.hint_on = true;
        self.save();
        self.push_tasks();
        if self.tier1() {
            self.ask(LlmKind::Mentor, format!("Stall detected ({reason}). Show the exact shape in ≤10 lines of code, verify it is correct, then name ONE small edit. Do not solve the whole slice."));
        }
    }

    fn run_checks(&mut self) {
        let Some(checks) = self.slice().map(|s| s.static_checks.clone()) else { return };
        for c in &mut self.st.static_status {
            c.status = "running".into();
        }
        let root = self.root.clone();
        self.spawn(move || {
            Msg::Static(
                checks
                    .iter()
                    .map(|cmd| {
                        let r = verify::run_shell(cmd, &root, verify::PROBE_TIMEOUT);
                        CheckStatus { cmd: cmd.clone(), status: if r.exit == 0 { "pass" } else { "fail" }.into(), output_head: if r.exit == 0 { String::new() } else { verify::failure_excerpt(&r.output) } }
                    })
                    .collect(),
            )
        });
    }

    fn on_static(&mut self, results: Vec<CheckStatus>) {
        let ok = results.iter().all(|c| c.status == "pass");
        self.stream(json!({"event": "static_checks", "pass": ok, "results": results.iter().map(|c| json!({"cmd": c.cmd, "status": c.status})).collect::<Vec<_>>()}));
        self.st.static_status = results;
        if ok && self.st.probe_status == "green" && !self.st.slice_complete {
            self.complete_slice();
        } else if !ok && self.tier1() {
            self.ask(LlmKind::Review, "The probe is green but static checks fail (see context). Run the Two-Axis Review with the failing check as the Standards finding.".into());
        }
        self.save();
        self.push_tasks();
    }

    fn complete_slice(&mut self) {
        let Some(s) = self.slice().cloned() else { return };
        self.st.slice_complete = true;
        if !self.st.passed_slices.contains(&s.id) {
            self.st.passed_slices.push(s.id.clone());
        }
        let mut events = self.store.load_events();
        for c in &s.concepts {
            let e = srs::grade_event(&events, now_ms(), "slice_pass", Some(&s.id), c, "solid", None);
            let _ = self.store.append_event(&e);
            events.push(e);
        }
        if s.ring == 0 && self.st.baseline_zpd.is_none() {
            let mins = self.st.slice_started_ts.as_deref().and_then(parse_iso).map(|t| (now_ms() - t) / 60_000).unwrap_or(0);
            let zpd = match (self.st.slice_probe_runs, mins) {
                (0..=2, 0..=5) => "fluent",
                (0..=4, 0..=15) => "steady",
                _ => "needs-scaffolding",
            };
            self.st.baseline_zpd = Some(zpd.into());
            self.save_blueprint();
        }
        let _ = self.store.promote_terms(&s.glossary);
        let _ = self.store.append_story(&s.id, &s.story_template, &s.trigger_words);
        log("info", "slice_pass", json!({"slice_id": s.id, "probe_runs": self.st.slice_probe_runs}));
        self.quiz = Some(quiz_view(&s, "drill", &s.concepts[0]));
        self.save();
        self.push_glossary();
        self.push_quiz();
        if self.tier1() && self.fluent(&s) {
            // WORKFLOW.md persona 4: concept already green + fluent → one interview question, never an ambush.
            self.st.persona = "interviewer".into();
            log("info", "persona_auto_interviewer", json!({"slice_id": s.id, "probe_runs": self.st.slice_probe_runs}));
            self.ask(LlmKind::MockQuestion, "The learner just went red → green fluently. You are now the Hiring Interviewer. In ONE line, name the strongest thing in their diff; then ask exactly ONE realistic interview question about it (failure modes, invariants, or tradeoffs). End with: (optional — answer below, or keep going)".into());
        } else if self.tier1() {
            self.ask(LlmKind::Review, "The probe just went red → green and static checks pass. Run the Two-Axis Review on the target files: (1) Spec — behavior and edge cases; (2) Standards & Depth — module depth, deletion test, invariants. Keep them separate.".into());
        }
    }

    /// Green with few probe runs, no stall, not right after a cold return, past the anchor.
    fn fluent(&self, s: &Slice) -> bool {
        s.ring >= 1 && !self.unprimed && self.st.slice_probe_runs <= FLUENT_PROBE_RUNS
    }

    // ── quizzes & SRS ──

    /// Push a review quiz for the oldest due/open concept that has an authored quiz.
    fn push_review(&mut self, queue: &[String]) -> bool {
        let Some(t) = &self.track else { return false };
        for c in queue {
            if let Some(s) = t.slices.iter().find(|s| s.concepts.contains(c) && self.st.passed_slices.contains(&s.id)) {
                self.quiz = Some(quiz_view(s, "review", c));
                self.push_quiz();
                return true;
            }
        }
        false
    }

    fn offer_review(&mut self) {
        if self.quiz.is_none() {
            let m = srs::metrics(&self.store.load_events(), now_ms());
            self.push_review(&m.queue);
        }
    }

    fn on_quiz_answer(&mut self, quiz_id: &str, choice: usize) {
        let Some(q) = self.quiz.clone().filter(|q| q.id == quiz_id) else { return };
        let slice_id = quiz_id.split(':').next().unwrap_or("");
        let Some(s) = self.track.as_ref().and_then(|t| t.slice(slice_id)).cloned() else { return };
        let quiz = &s.tier0_drill.quiz;
        let correct = choice == quiz.answer_index;
        let (event, grade) = if correct { ("drill_pass", "solid") } else { ("drill_fail", "wrong") };
        let misconception = (!correct).then(|| quiz.options.get(choice).cloned()).flatten();
        let e = srs::grade_event(&self.store.load_events(), now_ms(), event, Some(&s.id), &q.concept, grade, misconception.clone());
        let _ = self.store.append_event(&e);
        self.st.active_misconception = misconception;
        if q.kind == "warmup" {
            self.st.persona = if correct { "pair" } else { "mentor" }.into();
        }
        self.save();
        self.send_ui(&UiMsg::QuizResult { quiz_id: q.id.clone(), correct, correct_index: quiz.answer_index, explanation: quiz.explanation.clone() });
        self.push_tasks();
    }

    fn advance(&mut self) {
        let kind = self.quiz.take().map(|q| q.kind);
        self.push_quiz();
        match kind.as_deref() {
            Some("review") if self.review_pause => self.next_slice(),
            Some("drill") => self.next_slice(),
            _ if self.st.slice_complete => self.next_slice(),
            _ => self.push_tasks(),
        }
    }

    // ── Tier 1 input surface ──

    fn on_input(&mut self, text: String) {
        if !self.tier1() || text.is_empty() {
            return; // input bar is disabled in Tier 0
        }
        self.stream(json!({"event": "focus_input", "chars": text.len(), "command": text.starts_with('/').then(|| text.split_whitespace().next().unwrap_or("").to_string())}));
        match text.split_whitespace().next().unwrap_or("") {
            "/wait-what" => {
                self.st.persona = "mentor".into();
                let terms: Vec<String> = self.store.load_glossary().into_iter().map(|g| g.term).collect();
                self.ask(LlmKind::WaitWhat, format!("/wait-what — drop all jargon. Re-explain the current obstacle in plain English, using only these glossary terms: {}.", terms.join(", ")));
            }
            "/hint" => {
                self.hint_on = true;
                self.push_tasks();
                self.ask(LlmKind::Hint, "/hint — give the smallest unblocking nudge, or a ≤3-line example. Do not solve the slice.".into());
            }
            "/mock" => {
                self.st.persona = "interviewer".into();
                self.ask(LlmKind::MockQuestion, "/mock — you are the Hiring Interviewer. Ask exactly ONE realistic interview question grounded in the learner's code below (failure modes, invariants, scaling, or tradeoffs). Output only the question.".into());
            }
            "/next" => self.park(),
            "/retro" => {
                let stats = self.stream_stats();
                self.ask(LlmKind::Retro, format!("/retro — propose ≤3 environment improvements (aliases, test flags, hooks) as copyable snippets. Never claim to apply them; dotfiles are the user's. Recent terminal stats: {stats}"));
            }
            _ if self.mock_q.is_some() => {
                let question = self.mock_q.take().unwrap_or_default();
                self.ask(LlmKind::MockGrade { question: question.clone() }, format!(
                    "Interview question: {question}\nCandidate answer: {text}\n\nGrade it. Reply with ONLY a JSON object: {{\"grade\":\"solid|shaky|wrong\",\"misconception_shape\":\"short phrase or empty\",\"feedback\":\"≤4 lines, direct\",\"trigger_words\":[\"2-4 terms\"],\"avoid\":\"their imprecise phrase\",\"say\":\"senior phrasing\",\"story\":\"2 sentences: problem/constraint → mechanism/invariant → proof\"}}"
                ));
            }
            _ if self.track.is_none() => self.start_track(&text),
            _ => self.ask(LlmKind::Answer, text),
        }
        self.save();
    }

    fn park(&mut self) {
        let Some(id) = self.st.slice_id.clone() else { return };
        if !self.st.slice_complete {
            let _ = self.store.append_event(&Evidence { ts: now_iso(), event: "slice_parked".into(), slice_id: Some(id.clone()), concept: None, grade: None, misconception_shape: None, interval_days: None, due_at: None });
            if !self.st.parked_slices.contains(&id) {
                self.st.parked_slices.push(id);
            }
        }
        self.quiz = None;
        self.st.slice_complete = true; // allow next_slice to move on
        self.set_focus("", false);
        self.next_slice();
    }

    fn stream_stats(&self) -> String {
        let text = fs::read_to_string(self.store.path("stream.jsonl")).unwrap_or_default();
        let mut cmds: HashMap<String, (u32, u32)> = HashMap::new();
        for v in text.lines().rev().take(500).filter_map(|l| serde_json::from_str::<Value>(l).ok()) {
            if v["event"] == "cmd_end" && v["source"] == "zsh" {
                let head = v["cmd"].as_str().unwrap_or("").split_whitespace().take(2).collect::<Vec<_>>().join(" ");
                let e = cmds.entry(head).or_default();
                e.0 += 1;
                e.1 += u32::from(v["exit_code"].as_i64() != Some(0));
            }
        }
        let mut v: Vec<_> = cmds.into_iter().collect();
        v.sort_by_key(|(_, (n, _))| std::cmp::Reverse(*n));
        v.iter().take(8).map(|(c, (n, f))| format!("`{c}` ×{n} ({f} failed)")).collect::<Vec<_>>().join(", ")
    }

    fn context(&self) -> Value {
        let s = self.slice();
        let files: serde_json::Map<String, Value> = s
            .map(|s| s.target_files.iter().filter_map(|f| fs::read_to_string(self.root.join(f)).ok().map(|c| (f.clone(), Value::String(c.chars().take(4000).collect())))).collect())
            .unwrap_or_default();
        let m = srs::metrics(&self.store.load_events(), now_ms());
        json!({
            "persona": self.st.persona, "ring": self.st.ring, "track": self.st.track, "capstone": self.track.as_ref().map(|t| &t.def.capstone_goal),
            "baseline_zpd": self.st.baseline_zpd,
            "slice": s.map(|s| json!({"id": s.id, "goal": s.goal, "hint": s.hint, "concepts": s.concepts, "trigger_words": s.trigger_words})),
            "probe_cmd": self.st.probe_cmd, "probe_status": self.st.probe_status, "consecutive_same_err": self.st.consecutive_same_err, "churn_sec": self.st.churn_sec,
            "last_failure": self.excerpt, "static_checks": self.st.static_status, "target_files": files,
            "glossary_terms": self.store.load_glossary().into_iter().map(|g| g.term).collect::<Vec<_>>(),
            "due_items": m.due_items, "open_misconceptions": m.open_misconceptions, "active_misconception": self.st.active_misconception,
            "lsp_errors": self.lsp,
        })
    }

    fn ask(&mut self, kind: LlmKind, prompt: String) {
        if !self.tier1() {
            return;
        }
        let Some(model) = self.model.clone() else { return };
        let epoch = self.epoch;
        let cfg = self.cfg.clone();
        let system = format!("{}\n\n## Live context (JSON)\n{}", llm::system_prompt(), self.context());
        let request_id = format!("{}-{}", crate::util::session_id(), now_ms());
        let budget = if matches!(kind, LlmKind::JitTrack { .. } | LlmKind::JitNext { .. }) { llm::COMPILE } else { llm::CHAT };
        self.set_focus(if self.focus.is_empty() { "…".to_string() } else { self.focus.clone() }, true);
        self.spawn(move || {
            let result = llm::chat(&cfg, &model, &request_id, budget, &[("system", system), ("user", prompt)]).map(|o| o.text);
            Msg::Llm { epoch, kind, result }
        });
    }

    fn on_llm(&mut self, epoch: u64, kind: LlmKind, result: Result<String, LlmError>) {
        if epoch != self.epoch {
            return; // abandoned when we went offline
        }
        let text = match result {
            Ok(t) => {
                self.llm_errs = 0;
                t
            }
            Err(e) => {
                self.llm_errs += u32::from(e.transient);
                if e.timeout || self.llm_errs >= 2 {
                    return self.go_offline(&e.message);
                }
                return self.set_focus(format!("LLM request failed ({}). Ask again.", e.message), false);
            }
        };
        match kind {
            LlmKind::MockQuestion => {
                self.mock_q = Some(text.clone());
                self.set_focus(text, false);
            }
            LlmKind::MockGrade { question } => self.on_mock_grade(&question, &text),
            LlmKind::JitTrack { brief, attempt } => self.on_jit_track(&brief, attempt, &text),
            LlmKind::JitNext { attempt } => self.on_jit_next(attempt, &text),
            _ => self.set_focus(text, false),
        }
        self.save();
    }

    fn on_mock_grade(&mut self, question: &str, text: &str) {
        let Some(v) = llm::extract_json(text) else { return self.set_focus(text, false) };
        let grade = v["grade"].as_str().filter(|g| matches!(*g, "solid" | "shaky" | "wrong")).unwrap_or("shaky");
        let concept = self.slice().map(|s| s.concepts[0].clone()).unwrap_or_else(|| "general".into());
        let shape = v["misconception_shape"].as_str().filter(|s| !s.is_empty()).map(str::to_string);
        let e = srs::grade_event(&self.store.load_events(), now_ms(), "mock_grade", self.st.slice_id.as_deref(), &concept, grade, shape.clone());
        let _ = self.store.append_event(&e);
        self.st.active_misconception = shape;
        let tw: Vec<String> = v["trigger_words"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default();
        let swap = Swap { avoid: v["avoid"].as_str().unwrap_or("").into(), say: v["say"].as_str().unwrap_or("").into() };
        if let Some(story) = v["story"].as_str().filter(|s| !s.is_empty()) {
            let _ = self.store.append_story(self.st.slice_id.as_deref().unwrap_or("mock"), story, &tw);
        }
        if !tw.is_empty() || !swap.say.is_empty() {
            self.card = Some((tw, swap));
        }
        log("info", "mock_grade", json!({"grade": grade, "question_chars": question.len()}));
        self.st.persona = "pair".into();
        self.set_focus(v["feedback"].as_str().unwrap_or(text).to_string(), false);
        self.push_glossary();
        self.push_tasks();
    }

    // ── JIT course compiler ──

    fn jit_next_prompt(&self, error: Option<&str>) -> String {
        let passed = self.st.passed_slices.join(", ");
        let goal = self.track.as_ref().map(|t| t.def.capstone_goal.clone()).unwrap_or_default();
        jit_slice_prompt(&goal, &passed, self.st.ring, error)
    }

    fn on_jit_track(&mut self, brief: &str, attempt: u32, text: &str) {
        match write_jit_track(&self.store.dir, text) {
            Ok(dir) => {
                self.set_focus("", false);
                self.begin_track(dir);
            }
            Err(e) if attempt == 0 => {
                log("warn", "jit_parse_failed", json!({"message": e}));
                self.ask(LlmKind::JitTrack { brief: brief.into(), attempt: 1 }, jit_track_prompt(brief, Some(&e)));
            }
            Err(e) => self.fail_notice("jit_failed", &format!("Could not compile a course for that brief: {e}")),
        }
    }

    fn on_jit_next(&mut self, attempt: u32, text: &str) {
        let Some(dir) = self.st.track_dir.clone().map(PathBuf::from) else { return };
        match append_jit_slice(&dir, self.st.ring, text) {
            Ok(slice) => {
                self.set_focus("", false);
                self.track = track::load(&dir).ok();
                self.materialize(slice);
            }
            Err(e) if attempt == 0 => self.ask(LlmKind::JitNext { attempt: 1 }, self.jit_next_prompt(Some(&e))),
            Err(e) => self.fail_notice("jit_failed", &format!("Next slice could not be compiled: {e}")),
        }
    }
}

fn quiz_view(s: &Slice, kind: &str, concept: &str) -> QuizView {
    QuizView {
        id: format!("{}:{kind}", s.id),
        kind: kind.into(),
        prompt: (kind == "drill").then(|| s.tier0_drill.prompt.clone()),
        question: s.tier0_drill.quiz.question.clone(),
        options: s.tier0_drill.quiz.options.clone(),
        concept: concept.into(),
    }
}

const JIT_SLICE_SCHEMA: &str = r#""slice": {"id":"NN-slug","ring":1,"goal":"…","target_files":["…"],"probe_cmd":"…","static_checks":["…"],"hint":"…","lesson":"≤10-line shape example","trigger_words":["…"],"say_this_not_that":[{"avoid":"…","say":"…"}],"story_template":"2 sentences","concepts":["kebab-concept"],"glossary":[{"term":"…","definition":"…","avoid":["…"]}],"tier0_drill":{"prompt":"…","quiz":{"question":"…","options":["correct first","…","…"],"answer_index":0,"explanation":"…"}},"files":{"relative/path":"red stub + failing test, compiles"},"solution":{"relative/path":"reference implementation of the stub file(s) only"}}"#;

fn jit_track_prompt(brief: &str, error: Option<&str>) -> String {
    format!(
        "JIT Course Compiler. The learner wants: {brief:?}.\nPick a capstone and toolchain already likely installed (go, cargo, bun). Reply with ONLY one JSON object:\n{{\"track\":\"kebab-id\",\"title\":\"…\",\"capstone\":\"kebab\",\"capstone_goal\":\"one line\",\"rings\":[{{\"ring\":0,\"slug\":\"anchor\",\"goal\":\"…\"}},{{\"ring\":1,\"slug\":\"tracer-bullets\",\"goal\":\"…\"}},{{\"ring\":2,\"slug\":\"read-defend\",\"goal\":\"…\"}},{{\"ring\":3,\"slug\":\"break-harden\",\"goal\":\"…\"}},{{\"ring\":4,\"slug\":\"architect\",\"goal\":\"…\"}}],\"scaffold\":{{\"relative/path\":\"content (e.g. go.mod)\"}},{JIT_SLICE_SCHEMA}}}\nRules: probe_cmd must FAIL on the stub via a failing test (not a compile error) and PASS with the solution. No network in tests.{}",
        error.map(|e| format!("\nYour previous attempt was rejected: {e}")).unwrap_or_default()
    )
}

fn jit_slice_prompt(goal: &str, passed: &str, ring: u32, error: Option<&str>) -> String {
    format!(
        "JIT Course Compiler, next slice. Capstone: {goal}. Passed slices: {passed}. Current ring: {ring}. Design the next narrow vertical slice building on the existing workspace. Reply with ONLY {{{JIT_SLICE_SCHEMA}}}\nRules: new files only; probe_cmd fails on the stub via a failing test and passes with the solution.{}",
        error.map(|e| format!("\nPrevious attempt rejected: {e}")).unwrap_or_default()
    )
}

/// Validate an LLM-authored track and lay it out like a bundled one under `.symbiote/jit/`.
pub fn write_jit_track(symbiote_dir: &Path, text: &str) -> Result<PathBuf, String> {
    let v = llm::extract_json(text).ok_or("reply contained no JSON object")?;
    let id = v["track"].as_str().filter(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')).ok_or("track id must be kebab-case")?;
    let mut def: TrackDef = serde_json::from_value(json!({
        "track": id, "title": v["title"], "capstone": v["capstone"], "capstone_goal": v["capstone_goal"], "rings": v["rings"],
    }))
    .map_err(|e| format!("track fields: {e}"))?;
    let dir = symbiote_dir.join("jit").join(id);
    let _ = fs::remove_dir_all(&dir);
    write_files(&dir.join("scaffold"), &v["scaffold"])?;
    let slice = write_jit_slice_files(&dir, &v)?;
    match def.rings.iter_mut().find(|r| r.ring == slice.ring) {
        Some(r) => r.slices.push(slice.id.clone()),
        None => return Err(format!("slice ring {} not in rings", slice.ring)),
    }
    fs::write(dir.join("track.yaml"), serde_yaml::to_string(&def).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    track::load(&dir)?;
    Ok(dir)
}

fn append_jit_slice(dir: &Path, ring: u32, text: &str) -> Result<Slice, String> {
    let v = llm::extract_json(text).ok_or("reply contained no JSON object")?;
    let slice = write_jit_slice_files(dir, &v)?;
    let path = dir.join("track.yaml");
    let mut def: TrackDef = serde_yaml::from_str(&fs::read_to_string(&path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let r = def.rings.iter_mut().find(|r| r.ring == slice.ring.max(ring)).ok_or("slice ring not in track")?;
    if !r.slices.contains(&slice.id) {
        r.slices.push(slice.id.clone());
    }
    fs::write(&path, serde_yaml::to_string(&def).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    Ok(slice)
}

fn write_jit_slice_files(dir: &Path, v: &Value) -> Result<Slice, String> {
    let sv = &v["slice"];
    let slice: Slice = serde_json::from_value(sv.clone()).map_err(|e| format!("slice fields: {e}"))?;
    if !slice.id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err("slice id must be kebab-case".into());
    }
    track::validate_slice(&slice)?;
    track::jit_commands_ok(&slice)?;
    let sdir = dir.join("slices").join(&slice.id);
    write_files(&sdir.join("files"), &sv["files"])?;
    write_files(&sdir.join("solution"), &sv["solution"])?;
    fs::write(sdir.join("slice.yaml"), serde_yaml::to_string(&slice).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    Ok(slice)
}

fn write_files(base: &Path, files: &Value) -> Result<(), String> {
    let map = files.as_object().ok_or("files must be an object of path → content")?;
    fs::create_dir_all(base).map_err(|e| e.to_string())?;
    for (p, content) in map {
        let rel = track::safe_rel(p)?;
        let to = base.join(rel);
        fs::create_dir_all(to.parent().unwrap_or(base)).map_err(|e| e.to_string())?;
        fs::write(&to, content.as_str().ok_or("file content must be a string")?).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Client helper for the hook and `symbiote run`: fire one message, never block the prompt.
pub fn send_once(root: &Path, msg: &ClientMsg) -> io::Result<()> {
    let sock = std::env::var_os("SYMBIOTE_SOCK").map(PathBuf::from).unwrap_or_else(|| sock_path(root));
    let mut s = UnixStream::connect(sock)?;
    s.set_write_timeout(Some(Duration::from_millis(300)))?;
    let hello = serde_json::to_string(&ClientMsg::Hello { role: "hook".into() })?;
    let body = serde_json::to_string(msg)?;
    s.write_all(format!("{hello}\n{body}\n").as_bytes())
}
