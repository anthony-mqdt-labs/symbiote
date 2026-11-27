//! `symbiote` CLI: daemon, zsh hook emitter, `run` wrapper, and track self-verification.

use serde_json::json;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, ExitCode, Stdio};
use symbiote_core::daemon::{self, resolve_root, send_once};
use symbiote_core::model::ClientMsg;
use symbiote_core::util::{init_log, log, now_ms};
use symbiote_core::{track, verify};

const USAGE: &str = "usage:
  symbiote daemon [--root DIR]
  symbiote emit cmd_end --exit N --duration MS [--pipestatus a,b] -- CMD...
  symbiote run -- CMD...            run through a pipe, report output to the daemon
  symbiote tracks                   list bundled tracks
  symbiote verify-track <id|all> [--git] [--keep]";

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn after_dashes(args: &[String]) -> String {
    args.iter().position(|a| a == "--").map(|i| args[i + 1..].join(" ")).unwrap_or_default()
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let sub = args.get(1).map(String::as_str).unwrap_or("");
    let component = match sub {
        "daemon" => "symbiote-daemon",
        "emit" | "run" => "symbiote-hook",
        _ => "symbiote-cli",
    };
    init_log(component);
    let started = std::time::Instant::now();
    if sub != "emit" {
        log("info", "process_start", json!({"exe": args[0], "args": args[1..].iter().take(4).collect::<Vec<_>>(), "ppid": unsafe { libc::getppid() }, "cwd": std::env::current_dir().ok()}));
    }
    let code = match sub {
        "daemon" => match daemon::run(resolve_root(flag(&args, "--root").map(PathBuf::from))) {
            Ok(()) => 0,
            Err(e) => {
                log("critical", "daemon_failed", json!({"error_type": "io", "message": e.to_string()}));
                eprintln!("symbiote daemon: {e}");
                1
            }
        },
        "emit" => emit(&args),
        "run" => run(&args),
        "tracks" => {
            for t in track::list_tracks() {
                println!("{}{:<6} {} — {}", if t.recommended { "➡️ " } else { "   " }, t.id, t.title, t.capstone_goal);
            }
            0
        }
        "verify-track" => verify_tracks(&args),
        _ => {
            eprintln!("{USAGE}");
            2
        }
    };
    if sub != "emit" {
        log("info", "process_exit", json!({"exit_code": code, "wall_ms": started.elapsed().as_millis() as u64, "peak_rss_bytes": symbiote_core::util::peak_rss_bytes()}));
    }
    ExitCode::from(code)
}

/// Hot path from zsh precmd: one connect, one write, silent on failure.
fn emit(args: &[String]) -> u8 {
    let msg = ClientMsg::CmdEnd {
        cmd: after_dashes(args),
        exit_code: flag(args, "--exit").and_then(|s| s.parse().ok()).unwrap_or(-1),
        duration_ms: flag(args, "--duration").and_then(|s| s.parse().ok()).unwrap_or(0),
        pipestatus: flag(args, "--pipestatus").map(|s| s.split(',').filter_map(|x| x.parse().ok()).collect()).unwrap_or_default(),
        output_head: None,
    };
    if let Err(e) = send_once(&resolve_root(None), &msg) {
        log("warn", "emit_failed", json!({"message": e.to_string()}));
    }
    0
}

/// `symbiote run <cmd>`: tee output to the terminal and hand the daemon the captured head.
fn run(args: &[String]) -> u8 {
    let cmd = after_dashes(args);
    if cmd.is_empty() {
        eprintln!("{USAGE}");
        return 2;
    }
    let start = std::time::Instant::now();
    let mut child = match Command::new("/bin/sh").arg("-c").arg(format!("exec 2>&1; {cmd}")).stdout(Stdio::piped()).spawn() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("symbiote run: {e}");
            return 127;
        }
    };
    let mut captured = Vec::new();
    if let Some(mut out) = child.stdout.take() {
        let mut buf = [0u8; 8192];
        while let Ok(n) = out.read(&mut buf) {
            if n == 0 {
                break;
            }
            let _ = std::io::stdout().write_all(&buf[..n]);
            let _ = std::io::stdout().flush();
            captured.extend_from_slice(&buf[..n]);
        }
    }
    let exit = child.wait().ok().and_then(|s| s.code()).unwrap_or(-1);
    let output = String::from_utf8_lossy(&captured).into_owned();
    let msg = ClientMsg::CmdEnd { cmd, exit_code: exit, duration_ms: start.elapsed().as_millis() as u64, pipestatus: vec![exit], output_head: Some(output) };
    let _ = send_once(&resolve_root(None), &msg);
    exit.clamp(0, 255) as u8
}

/// Walk every slice of a track through the real gate, then play the learner:
/// apply the stub, then the reference, and require a green probe in the workspace itself.
fn verify_tracks(args: &[String]) -> u8 {
    let which = args.get(2).cloned().unwrap_or_default();
    let ids: Vec<String> = if which == "all" { track::list_tracks().into_iter().map(|t| t.id).collect() } else { vec![which] };
    let (use_git, keep) = (args.iter().any(|a| a == "--git"), args.iter().any(|a| a == "--keep"));
    let mut failed = 0;
    for id in ids {
        let Some(dir) = track::find_bundled(&id) else {
            eprintln!("unknown track {id:?}");
            return 2;
        };
        let t = match track::load(&dir) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("{id}: {e}");
                failed += 1;
                continue;
            }
        };
        let ws = std::env::temp_dir().join(format!("symbiote-trackcheck-{id}-{}", now_ms()));
        std::fs::create_dir_all(&ws).expect("temp workspace");
        let git = |a: &[&str]| Command::new("git").arg("-C").arg(&ws).args(["-c", "user.name=symbiote", "-c", "user.email=placeholder@example.com"]).args(a).output();
        if let Err(e) = track::copy_overlay(&dir.join("scaffold"), &ws) {
            eprintln!("{id}: scaffold: {e}");
            failed += 1;
            continue;
        }
        if use_git {
            let _ = git(&["init", "-q"]);
            let _ = git(&["add", "-A"]);
            let _ = git(&["commit", "-qm", "scaffold"]);
        }
        println!("── {} ({} slices) in {}", t.def.title, t.slices.len(), ws.display());
        for s in &t.slices {
            let t0 = std::time::Instant::now();
            let gate = verify::gate(&ws, &t, s);
            let learner = gate.as_ref().map_err(String::clone).and_then(|_| {
                let sd = t.slice_dir(&s.id);
                track::copy_overlay(&sd.join("files"), &ws)?;
                track::apply_wire(s, &ws)?;
                track::copy_overlay(&sd.join("solution"), &ws)?;
                let r = verify::run_shell(&s.probe_cmd, &ws, verify::PROBE_TIMEOUT);
                if r.exit != 0 {
                    return Err(format!("workspace probe red after reference:\n{}", verify::failure_excerpt(&r.output)));
                }
                for c in &s.static_checks {
                    let r = verify::run_shell(c, &ws, verify::PROBE_TIMEOUT);
                    if r.exit != 0 {
                        return Err(format!("workspace check `{c}` failed:\n{}", verify::failure_excerpt(&r.output)));
                    }
                }
                Ok(())
            });
            match learner {
                Ok(()) => println!("  ✓ {:<24} red→green verified ({:.1}s)", s.id, t0.elapsed().as_secs_f32()),
                Err(e) => {
                    failed += 1;
                    println!("  ✗ {:<24} {e}", s.id);
                }
            }
            if use_git {
                let _ = git(&["add", "-A"]);
                let _ = git(&["commit", "-qm", &s.id]);
            }
        }
        if !keep {
            let _ = std::fs::remove_dir_all(&ws);
        }
    }
    u8::from(failed > 0)
}
