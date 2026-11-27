//! Tier 1 client: any OpenAI-compatible endpoint (default: local spark on :8095).
//! Contract: 500ms health on boot, 15s per interactive request. Classifies failures so the
//! daemon can flip to Tier 0 (timeout, or ≥2 consecutive 5xx/429/transport errors).

use crate::util::log;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

pub const HEALTH_TIMEOUT: Duration = Duration::from_millis(500);

/// Output cap and deadline for one request.
#[derive(Clone, Copy, Debug)]
pub struct Budget {
    pub max_tokens: u32,
    pub timeout: Duration,
}

/// Focus-region replies: ≤15 lines, so 700 tokens and the spec's 15s are plenty.
pub const CHAT: Budget = Budget { max_tokens: 700, timeout: Duration::from_secs(15) };
/// JIT track/slice compilation returns JSON with stub, test and solution files; 700 tokens
/// truncated it. Deviation from ARCHITECTURE.md's flat 15s: no model writes that much in 15s.
pub const COMPILE: Budget = Budget { max_tokens: 8000, timeout: Duration::from_secs(180) };

#[derive(Clone, Debug)]
pub struct LlmConfig {
    pub url: String,
    pub model: Option<String>,
    key: Option<String>,
}

pub fn config() -> LlmConfig {
    let url = std::env::var("SYMBIOTE_LLM_URL").unwrap_or_else(|_| "http://127.0.0.1:8095/v1".into());
    LlmConfig {
        url: url.trim_end_matches('/').to_string(),
        model: std::env::var("SYMBIOTE_LLM_MODEL").ok().filter(|s| !s.is_empty()),
        key: std::env::var("SYMBIOTE_LLM_KEY").ok().filter(|s| !s.is_empty()),
    }
}

impl LlmConfig {
    /// Loggable view: presence of the key, never its value.
    pub fn describe(&self) -> Value {
        json!({"url": self.url, "model": self.model, "api_key": if self.key.is_some() { "set" } else { "unset" }})
    }
}

#[derive(Debug)]
pub struct LlmError {
    pub timeout: bool,
    /// 5xx, 429 (e.g. a ChatGPT-plan usage window ran out) or transport — counts toward the
    /// 2-strike flip to Tier 0. Other 4xx do not.
    pub transient: bool,
    pub message: String,
}

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder().timeout_global(Some(timeout)).build().into()
}

fn classify(e: ureq::Error) -> LlmError {
    match e {
        ureq::Error::Timeout(_) => LlmError { timeout: true, transient: true, message: "timeout".into() },
        ureq::Error::StatusCode(c) => LlmError { timeout: false, transient: c >= 500 || c == 429, message: format!("http {c}") },
        other => LlmError { timeout: false, transient: true, message: other.to_string() },
    }
}

/// `GET /models`. Returns the model to use (configured, else the first served).
pub fn health(cfg: &LlmConfig) -> Result<String, LlmError> {
    let start = Instant::now();
    let mut req = agent(HEALTH_TIMEOUT).get(format!("{}/models", cfg.url));
    if let Some(k) = &cfg.key {
        req = req.header("Authorization", format!("Bearer {k}"));
    }
    let res = req.call().map_err(classify).and_then(|mut r| r.body_mut().read_json::<Value>().map_err(classify));
    let ok = res.is_ok();
    log("info", "health_check", json!({"component_checked": "llm", "pass": ok, "latency_ms": start.elapsed().as_millis() as u64}));
    let body = res?;
    if let Some(m) = &cfg.model {
        return Ok(m.clone());
    }
    body["data"][0]["id"]
        .as_str()
        .map(str::to_string)
        .ok_or(LlmError { timeout: false, transient: false, message: "no models served".into() })
}

pub struct ChatOut {
    pub text: String,
    pub completion_tokens: u64,
    pub total_ms: u64,
}

pub fn chat(cfg: &LlmConfig, model: &str, request_id: &str, budget: Budget, messages: &[(&str, String)]) -> Result<ChatOut, LlmError> {
    let msgs: Vec<Value> = messages.iter().map(|(r, c)| json!({"role": r, "content": c})).collect();
    let prompt_chars: usize = messages.iter().map(|(_, c)| c.len()).sum();
    log("info", "inference_dispatched", json!({"request_id": request_id, "model": model, "prompt_tokens_est": prompt_chars / 4, "max_tokens": budget.max_tokens, "timeout_ms": budget.timeout.as_millis() as u64}));
    let start = Instant::now();
    let mut req = agent(budget.timeout).post(format!("{}/chat/completions", cfg.url));
    if let Some(k) = &cfg.key {
        req = req.header("Authorization", format!("Bearer {k}"));
    }
    let res = req
        .send_json(json!({"model": model, "messages": msgs, "temperature": 0.3, "max_tokens": budget.max_tokens}))
        .map_err(classify)
        .and_then(|mut r| r.body_mut().read_json::<Value>().map_err(classify));
    let total_ms = start.elapsed().as_millis() as u64;
    match res {
        Ok(body) => {
            let text = body["choices"][0]["message"]["content"].as_str().unwrap_or("").trim().to_string();
            let completion_tokens = body["usage"]["completion_tokens"].as_u64().unwrap_or(0);
            log("info", "inference_received", json!({"request_id": request_id, "completion_tokens": completion_tokens, "total_ms": total_ms}));
            Ok(ChatOut { text, completion_tokens, total_ms })
        }
        Err(e) => {
            log("error", "backend_error", json!({"request_id": request_id, "error_class": if e.timeout { "timeout" } else if e.transient { "transient" } else { "client" }, "message": e.message}));
            Err(e)
        }
    }
}

/// The SYMBIOTE_PROMPT block from PROMPT.md, compiled in so the binary is self-contained.
pub fn system_prompt() -> &'static str {
    const DOC: &str = include_str!("../../../PROMPT.md");
    let start = DOC.find("```markdown\n").map(|i| i + 12).unwrap_or(0);
    let end = DOC.rfind("```").filter(|&e| e > start).unwrap_or(DOC.len());
    &DOC[start..end]
}

/// Hard Interaction Rule 1, enforced rather than trusted: ≤15 prose lines (code fences exempt).
pub fn clamp_focus(text: &str) -> String {
    let (mut out, mut prose, mut in_code, mut cut) = (vec![], 0, false, false);
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            in_code = !in_code;
            out.push(line);
            continue;
        }
        if !in_code && !line.trim().is_empty() {
            prose += 1;
            if prose > 15 {
                cut = true;
                continue;
            }
        }
        out.push(line);
    }
    if in_code {
        out.push("```");
    }
    let mut s = out.join("\n");
    if cut {
        s.push_str("\n…");
    }
    s
}

/// First balanced `{...}` object in a model reply.
pub fn extract_json(text: &str) -> Option<Value> {
    let start = text.find('{')?;
    let mut depth = 0;
    let (mut in_str, mut esc) = (false, false);
    for (i, ch) in text[start..].char_indices() {
        match ch {
            _ if esc => esc = false,
            '\\' if in_str => esc = true,
            '"' => in_str = !in_str,
            '{' if !in_str => depth += 1,
            '}' if !in_str => {
                depth -= 1;
                if depth == 0 {
                    return serde_json::from_str(&text[start..start + i + 1]).ok();
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_limit_429_counts_toward_offline_flip() {
        assert!(classify(ureq::Error::StatusCode(429)).transient, "plan usage exhausted → Tier 0, not an error loop");
        assert!(classify(ureq::Error::StatusCode(503)).transient);
        assert!(!classify(ureq::Error::StatusCode(400)).transient);
        assert!(COMPILE.max_tokens > CHAT.max_tokens && COMPILE.timeout > CHAT.timeout);
    }

    #[test]
    fn prompt_block_extracted() {
        let p = system_prompt();
        assert!(p.starts_with("# SYMBIOTE"));
        assert!(!p.contains("```markdown"));
    }

    #[test]
    fn clamp_counts_prose_only() {
        let long: String = (0..20).map(|i| format!("line {i}\n")).collect();
        let c = clamp_focus(&long);
        assert!(c.contains("line 14") && !c.contains("line 15") && c.ends_with('…'));
        let code = format!("```go\n{}```\nok", (0..30).map(|i| format!("x{i}\n")).collect::<String>());
        assert_eq!(clamp_focus(&code), code);
    }

    #[test]
    fn json_extraction() {
        let v = extract_json("Sure! {\"grade\":\"solid\",\"note\":\"a } in {str}\"} trailing").unwrap();
        assert_eq!(v["grade"], "solid");
        assert!(extract_json("no json").is_none());
    }
}
