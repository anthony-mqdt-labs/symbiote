//! Isolated probe execution and the `_verify/` red/green gate (ARCHITECTURE.md §4).
//!
//! Invariant: the user's working tree is never mutated here. Every gate runs in a
//! detached git worktree (or a plain copy when the workspace has no commits).

use crate::model::Slice;
use crate::track::{apply_wire, copy_overlay, tree, Track};
use crate::util::{log, now_ms};
use serde_json::json;
use std::fs;
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub const PROBE_TIMEOUT: Duration = Duration::from_secs(180);
const SKIP_DIRS: [&str; 4] = [".git", ".symbiote", "target", "node_modules"];

pub struct RunOut {
    pub exit: i32,
    pub output: String,
    pub timed_out: bool,
    pub duration_ms: u64,
}

/// GUI-launched processes inherit a minimal PATH; add the usual toolchain homes.
pub fn augmented_path() -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    let mut parts: Vec<String> = std::env::var("PATH").unwrap_or_default().split(':').map(str::to_string).collect();
    for extra in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/local/go/bin", &format!("{home}/.cargo/bin"), &format!("{home}/.bun/bin"), &format!("{home}/go/bin"), "/usr/bin", "/bin"] {
        if !parts.iter().any(|p| p == extra) {
            parts.push(extra.to_string());
        }
    }
    parts.join(":")
}

/// Run a slice-defined command (`probe_cmd` / `static_checks`) through `/bin/sh -c`.
/// Own process group so a timeout kills the whole tree (go test → test binary).
pub fn run_shell(cmd: &str, cwd: &Path, timeout: Duration) -> RunOut {
    let start = Instant::now();
    let child = Command::new("/bin/sh")
        .arg("-c")
        .arg(cmd)
        .current_dir(cwd)
        .env("PATH", augmented_path())
        .env("NO_COLOR", "1")
        .env("CARGO_TERM_COLOR", "never")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => return RunOut { exit: -1, output: format!("spawn failed: {e}"), timed_out: false, duration_ms: 0 },
    };
    let pipe = |r: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut s = String::new();
            if let Some(mut r) = r {
                let mut buf = vec![];
                let _ = r.read_to_end(&mut buf);
                s = String::from_utf8_lossy(&buf).into_owned();
            }
            s
        })
    };
    let out_t = pipe(child.stdout.take().map(|s| Box::new(s) as Box<dyn Read + Send>));
    let err_t = pipe(child.stderr.take().map(|s| Box::new(s) as Box<dyn Read + Send>));
    let mut timed_out = false;
    let exit = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st.code().unwrap_or(-1),
            Ok(None) if start.elapsed() >= timeout => {
                timed_out = true;
                // SAFETY: negative pid targets the process group created by process_group(0).
                unsafe { libc::kill(-(child.id() as i32), libc::SIGKILL) };
                let _ = child.wait();
                break -1;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(_) => break -1,
        }
    };
    let output = format!("{}{}", out_t.join().unwrap_or_default(), err_t.join().unwrap_or_default());
    RunOut { exit, output, timed_out, duration_ms: start.elapsed().as_millis() as u64 }
}

fn failure_re() -> &'static regex::Regex {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"(?i)(fail|panic|error|expected|assert|undefined|cannot|mismatch|✗|\(fail\)|left:|right:|got)").unwrap())
}

/// The lines that matter in a failing run (≤2 KiB): failure-ish lines, else the tail.
pub fn failure_excerpt(output: &str) -> String {
    let hits: Vec<&str> = output.lines().filter(|l| failure_re().is_match(l)).collect();
    let lines: Vec<&str> = if hits.is_empty() {
        let all: Vec<&str> = output.lines().collect();
        all[all.len().saturating_sub(10)..].to_vec()
    } else {
        hits
    };
    let mut s = lines.join("\n");
    if s.len() > 2048 {
        let mut cut = 2048;
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        s.truncate(cut);
    }
    s
}

/// First `file.ext:line` reference, for the Tier 0 stall pointer.
pub fn stderr_pointer(excerpt: &str) -> Option<String> {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| regex::Regex::new(r"[\w./-]+\.(go|rs|ts|tsx|js|py):\d+(:\d+)?").unwrap());
    excerpt.lines().find(|l| re.is_match(l)).map(|l| l.trim().to_string())
}

/// True when `cmd` invokes `<tool> test` as adjacent words (so "cargo test" ≠ "go test").
fn runs(cmd: &str, tool: &str) -> bool {
    let w: Vec<&str> = cmd.split_whitespace().collect();
    w.windows(2).any(|p| p[0] == tool && p[1] == "test")
}

/// Red must come from test execution or an assertion, never a broken harness.
pub fn classify_red(probe_cmd: &str, out: &RunOut) -> Result<(), String> {
    if out.timed_out {
        return Err("probe timed out (a hang is not a valid red)".into());
    }
    if out.exit == 0 {
        return Err("probe passed against the stub — slice would start green".into());
    }
    let o = &out.output;
    let harness_broken = if runs(probe_cmd, "go") {
        o.contains("[build failed]") || o.contains("[setup failed]") || !o.contains("--- FAIL")
    } else if runs(probe_cmd, "cargo") {
        o.contains("error[E") || o.contains("could not compile") || !o.contains("test result: FAILED")
    } else if runs(probe_cmd, "bun") {
        // A failure bun attributes to a test prints `(fail) <name>`; a module that won't
        // load surfaces as "Unhandled error between tests" instead.
        !o.contains("(fail) ") || o.contains("Unhandled error between tests") || o.contains("Cannot find module")
    } else {
        false
    };
    if harness_broken {
        return Err(format!("red came from a broken harness, not a failing test:\n{}", failure_excerpt(o)));
    }
    Ok(())
}

/// A throwaway copy of the workspace. Removed on drop.
pub struct Isolated {
    pub path: PathBuf,
    worktree_of: Option<PathBuf>,
}

impl Drop for Isolated {
    fn drop(&mut self) {
        if let Some(root) = &self.worktree_of {
            let _ = Command::new("git").arg("-C").arg(root).args(["worktree", "remove", "--force"]).arg(&self.path).output();
            let _ = Command::new("git").arg("-C").arg(root).args(["worktree", "prune"]).output();
        }
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn git_ok(root: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let out = Command::new("git").arg("-C").arg(root).args(args).output().ok()?;
    out.status.success().then_some(out.stdout)
}

pub fn isolate(root: &Path, id: &str) -> Result<Isolated, String> {
    let path = std::env::temp_dir().join(format!("symbiote-verify-{id}-{}-{}", std::process::id(), now_ms()));
    let has_head = git_ok(root, &["rev-parse", "--verify", "-q", "HEAD"]).is_some();
    if has_head {
        let p = path.to_string_lossy().to_string();
        git_ok(root, &["worktree", "add", "--detach", "-q", &p, "HEAD"]).ok_or("git worktree add failed")?;
        let iso = Isolated { path: path.clone(), worktree_of: Some(root.to_path_buf()) };
        // Overlay dirty + untracked files so verification sees what the user sees.
        let listed = git_ok(root, &["ls-files", "-z", "-m", "-o", "--exclude-standard"]).unwrap_or_default();
        for rel in listed.split(|b| *b == 0).filter(|r| !r.is_empty()) {
            let rel = String::from_utf8_lossy(rel).to_string();
            if rel.starts_with(".symbiote/") {
                continue;
            }
            let (from, to) = (root.join(&rel), path.join(&rel));
            if from.is_file() {
                fs::create_dir_all(to.parent().unwrap_or(&path)).map_err(|e| e.to_string())?;
                fs::copy(&from, &to).map_err(|e| e.to_string())?;
            } else {
                let _ = fs::remove_file(&to); // deleted in the working tree
            }
        }
        return Ok(iso);
    }
    fs::create_dir_all(&path).map_err(|e| e.to_string())?;
    let iso = Isolated { path: path.clone(), worktree_of: None };
    copy_tree(root, &path)?;
    Ok(iso)
}

fn copy_tree(src: &Path, dest: &Path) -> Result<(), String> {
    for e in fs::read_dir(src).map_err(|e| e.to_string())?.flatten() {
        let name = e.file_name();
        let Ok(ft) = e.file_type() else { continue };
        if SKIP_DIRS.iter().any(|s| name == *s) || ft.is_symlink() {
            continue;
        }
        let to = dest.join(&name);
        if ft.is_dir() {
            fs::create_dir_all(&to).map_err(|e| e.to_string())?;
            copy_tree(&e.path(), &to)?;
        } else {
            fs::copy(e.path(), &to).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[derive(Debug)]
pub struct GateReport {
    pub red_excerpt: String,
    pub red_ms: u64,
    pub green_ms: u64,
}

/// Red gate on the stub, green gate on the reference overlay + static checks.
pub fn gate(root: &Path, track: &Track, slice: &Slice) -> Result<GateReport, String> {
    let sdir = track.slice_dir(&slice.id);
    let iso = isolate(root, &slice.id)?;
    copy_overlay(&sdir.join("files"), &iso.path)?;
    apply_wire(slice, &iso.path)?;
    let red = run_shell(&slice.probe_cmd, &iso.path, PROBE_TIMEOUT);
    classify_red(&slice.probe_cmd, &red).map_err(|e| format!("red gate: {e}"))?;
    if tree(&sdir.join("solution")).is_empty() {
        return Err("green gate: slice has no reference solution".into());
    }
    copy_overlay(&sdir.join("solution"), &iso.path)?;
    let green = run_shell(&slice.probe_cmd, &iso.path, PROBE_TIMEOUT);
    if green.exit != 0 {
        return Err(format!("green gate: reference solution fails probe:\n{}", failure_excerpt(&green.output)));
    }
    for check in &slice.static_checks {
        let r = run_shell(check, &iso.path, PROBE_TIMEOUT);
        if r.exit != 0 {
            return Err(format!("green gate: static check `{check}` fails on reference:\n{}", failure_excerpt(&r.output)));
        }
    }
    log("info", "verify_gate_pass", json!({"slice_id": slice.id, "red_ms": red.duration_ms, "green_ms": green.duration_ms}));
    Ok(GateReport { red_excerpt: failure_excerpt(&red.output), red_ms: red.duration_ms, green_ms: green.duration_ms })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn excerpt_and_pointer() {
        let out = "=== RUN TestX\n    queue_test.go:12: got 0, want 3\n--- FAIL: TestX (0.00s)\nFAIL\nok other";
        let ex = failure_excerpt(out);
        assert!(ex.contains("queue_test.go:12"));
        assert!(!ex.contains("ok other"));
        assert_eq!(stderr_pointer(&ex).as_deref(), Some("queue_test.go:12: got 0, want 3"));
    }

    #[test]
    fn red_classifier() {
        let mk = |exit, output: &str| RunOut { exit, output: output.into(), timed_out: false, duration_ms: 1 };
        assert!(classify_red("go test ./...", &mk(1, "--- FAIL: TestA\nFAIL")).is_ok());
        assert!(classify_red("go test ./...", &mk(1, "x.go:1: syntax error\nFAIL q [build failed]")).is_err());
        assert!(classify_red("cargo test", &mk(101, "test result: FAILED. 0 passed; 1 failed")).is_ok());
        assert!(classify_red("cargo test", &mk(101, "error[E0425]: cannot find value")).is_err());
        assert!(classify_red("bun test", &mk(1, "(fail) parses [1ms]\n 0 pass\n 2 fail\n")).is_ok());
        assert!(classify_red("bun test", &mk(1, "(fail) t\nError name: \"SyntaxError\"\n 1 fail")).is_ok(), "SyntaxError thrown inside a test is a valid red");
        assert!(classify_red("bun test", &mk(1, "# Unhandled error between tests\nSyntaxError: Unexpected token\n 1 fail")).is_err());
        assert!(classify_red("make check", &mk(0, "")).is_err());
    }

    #[test]
    fn run_shell_timeout_kills_group() {
        let t = Instant::now();
        let r = run_shell("sleep 5; echo never", Path::new("/"), Duration::from_millis(200));
        assert!(r.timed_out && t.elapsed() < Duration::from_secs(2));
        assert!(!r.output.contains("never"));
    }
}
