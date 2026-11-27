//! Small shared primitives: ISO-8601 time, err_hash normalization, JSONL logging.
//! Hand-rolled to avoid pulling a date/time or logging framework into the tree.

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

// ── time ───────────────────────────────────────────────────────────────────

pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

pub fn now_iso() -> String {
    iso(now_ms())
}

/// Epoch milliseconds → `YYYY-MM-DDTHH:MM:SS.mmmZ`.
pub fn iso(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let (days, sod) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z",
        sod / 3600,
        (sod % 3600) / 60,
        sod % 60,
        ms.rem_euclid(1000)
    )
}

/// Parses `YYYY-MM-DDTHH:MM:SS[.fff]Z` → epoch milliseconds.
pub fn parse_iso(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 20 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' || !s.ends_with('Z') {
        return None;
    }
    let n = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, mo, d, h, mi, se) = (n(0..4)?, n(5..7)?, n(8..10)?, n(11..13)?, n(14..16)?, n(17..19)?);
    let frac = match s.get(19..s.len() - 1) {
        Some("") => 0,
        Some(f) if f.starts_with('.') => {
            let digits = &f[1..];
            let v: i64 = digits.get(..digits.len().min(3))?.parse().ok()?;
            v * 10_i64.pow(3 - digits.len().min(3) as u32)
        }
        _ => return None,
    };
    let days = days_from_civil(y, mo as u32, d as u32);
    Some((days * 86_400 + h * 3600 + mi * 60 + se) * 1000 + frac)
}

// Howard Hinnant's civil-calendar algorithms.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468
}

// ── err_hash ───────────────────────────────────────────────────────────────

fn re(p: &str) -> regex::Regex {
    regex::Regex::new(p).expect("static regex")
}

/// Strip run-to-run noise so the same failure hashes the same way twice.
pub fn normalize_err(s: &str) -> String {
    static RES: OnceLock<Vec<(regex::Regex, &'static str)>> = OnceLock::new();
    let res = RES.get_or_init(|| {
        vec![
            (re(r"\x1b\[[0-9;?]*[ -/]*[@-~]"), ""),
            (re(r"\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}(\.\d+)?Z?"), "<ts>"),
            (re(r"(/private)?/(tmp|var/folders)/\S+"), "<tmp>"),
            (re(r"0x[0-9a-fA-F]+"), "<hex>"),
            (re(r"-[0-9a-f]{16}\b"), "-<h>"),
            (re(r"\b\d+(\.\d+)?(ns|µs|us|ms|s|m)\b"), "<dur>"),
            (re(r"[ \t]+"), " "),
        ]
    });
    let mut out = s.to_string();
    for (r, with) in res {
        out = r.replace_all(&out, *with).into_owned();
    }
    out.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("\n")
}

/// First 8 hex chars of SHA-256 over the first 10 normalized lines.
pub fn err_hash(s: &str) -> String {
    let norm = normalize_err(s);
    let head: Vec<&str> = norm.lines().take(10).collect();
    let digest = Sha256::digest(head.join("\n").as_bytes());
    digest.iter().take(4).map(|b| format!("{b:02x}")).collect()
}

// ── JSONL logging (vibes runtime logging contract) ─────────────────────────

struct Logger {
    component: &'static str,
    session_id: String,
    dir: PathBuf,
    recent: Mutex<VecDeque<String>>,
}

static LOGGER: OnceLock<Logger> = OnceLock::new();
const CONTEXT_LINES: usize = 20;
const RETAIN_DAYS: i64 = 7;

pub fn log_dir() -> PathBuf {
    std::env::var_os("SYMBIOTE_LOG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../logs")))
}

/// Initialize once per process. Prunes files older than 7 days, installs a panic hook.
pub fn init_log(component: &'static str) {
    let dir = log_dir().join(component);
    let _ = fs::create_dir_all(&dir);
    if let Ok(rd) = fs::read_dir(&dir) {
        let cutoff = now_ms() - RETAIN_DAYS * 86_400_000;
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            let day = name.trim_end_matches(".jsonl");
            if parse_iso(&format!("{day}T00:00:00Z")).is_some_and(|t| t < cutoff) {
                let _ = fs::remove_file(e.path());
            }
        }
    }
    let session_id = format!("{}-{}", std::process::id(), now_ms());
    let _ = LOGGER.set(Logger { component, session_id, dir, recent: Mutex::new(VecDeque::new()) });
    std::panic::set_hook(Box::new(|info| {
        let loc = info.location();
        log(
            "critical",
            "panic",
            json!({
                "message": info.payload().downcast_ref::<&str>().map(|s| s.to_string())
                    .or_else(|| info.payload().downcast_ref::<String>().cloned()).unwrap_or_default(),
                "file": loc.map(|l| l.file()), "line": loc.map(|l| l.line()),
            }),
        );
        eprintln!("{info}");
    }));
}

pub fn session_id() -> String {
    LOGGER.get().map(|l| l.session_id.clone()).unwrap_or_default()
}

/// Emit one structured log line. `error`/`critical` carry the preceding lines as `context_window`.
pub fn log(level: &str, event: &str, fields: Value) {
    let Some(lg) = LOGGER.get() else { return };
    let mut obj = json!({
        "ts": now_iso(), "level": level, "component": lg.component, "event": event,
        "session_id": lg.session_id, "pid": std::process::id(),
    });
    if let (Some(o), Value::Object(f)) = (obj.as_object_mut(), fields) {
        o.extend(f);
    }
    let Ok(mut recent) = lg.recent.lock() else { return };
    if matches!(level, "error" | "critical") {
        obj["context_window"] = json!(recent.iter().collect::<Vec<_>>());
    }
    let line = obj.to_string();
    recent.push_back(line.clone());
    if recent.len() > CONTEXT_LINES {
        recent.pop_front();
    }
    let path = lg.dir.join(format!("{}.jsonl", &now_iso()[..10]));
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{line}");
    }
}

/// Peak resident set size in bytes (macOS reports bytes, Linux KiB).
pub fn peak_rss_bytes() -> i64 {
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    // SAFETY: getrusage writes into a caller-owned, correctly sized struct.
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut ru) } != 0 {
        return -1;
    }
    let v = ru.ru_maxrss as i64;
    if cfg!(target_os = "macos") { v } else { v * 1024 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_roundtrip() {
        for ms in [0, 1_795_729_455_123, 951_782_400_000 /* 2000-02-29 */, -86_400_000] {
            assert_eq!(parse_iso(&iso(ms)), Some(ms), "{}", iso(ms));
        }
        assert_eq!(iso(1_795_725_855_000), "2026-11-26T20:44:15.000Z");
        assert_eq!(parse_iso("2026-11-26T20:44:15Z"), Some(1_795_725_855_000));
        assert_eq!(parse_iso("garbage"), None);
    }

    #[test]
    fn err_hash_ignores_noise() {
        let a = "\x1b[31mFAIL\x1b[0m queue 0.31s\nworker.go:34: undefined: ctx at 0xdeadbeef\n/tmp/go-build123/x";
        let b = "FAIL   queue 1.9s\nworker.go:34: undefined: ctx at 0xcafe\n/private/var/folders/zz/abc";
        assert_eq!(err_hash(a), err_hash(b));
        assert_ne!(err_hash(a), err_hash("worker.go:35: undefined: ctx"));
        assert_eq!(err_hash(a).len(), 8);
    }
}
