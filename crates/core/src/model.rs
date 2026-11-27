//! `.symbiote/` schemas (ARCHITECTURE.md §2) and the symbiote.sock wire protocol.

use serde::{Deserialize, Serialize};

// ── state.json ─────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct State {
    pub track: Option<String>,
    pub capstone: Option<String>,
    pub ring: u32,
    pub slice_id: Option<String>,
    pub persona: String,
    pub baseline_zpd: Option<String>,
    pub probe_cmd: Option<String>,
    pub probe_status: String,
    pub last_err_hash: Option<String>,
    pub last_probe_ts: Option<String>,
    pub consecutive_same_err: u32,
    pub churn_sec: u64,
    pub active_misconception: Option<String>,
    pub parked_slices: Vec<String>,
    pub llm_mode: String,
    // Engine bookkeeping beyond the minimal schema.
    pub track_dir: Option<String>,
    pub passed_slices: Vec<String>,
    pub last_green_probe_cmd: Option<String>,
    pub slice_started_ts: Option<String>,
    pub slice_probe_runs: u32,
    pub slice_complete: bool,
    pub static_status: Vec<CheckStatus>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            track: None,
            capstone: None,
            ring: 0,
            slice_id: None,
            persona: "mentor".into(),
            baseline_zpd: None,
            probe_cmd: None,
            probe_status: "unknown".into(),
            last_err_hash: None,
            last_probe_ts: None,
            consecutive_same_err: 0,
            churn_sec: 0,
            active_misconception: None,
            parked_slices: vec![],
            llm_mode: "tier0_offline".into(),
            track_dir: None,
            passed_slices: vec![],
            last_green_probe_cmd: None,
            slice_started_ts: None,
            slice_probe_runs: 0,
            slice_complete: false,
            static_status: vec![],
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct CheckStatus {
    pub cmd: String,
    /// `pending | running | pass | fail`
    pub status: String,
    #[serde(default)]
    pub output_head: String,
}

// ── blueprint.json ─────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Blueprint {
    pub track: String,
    pub capstone: String,
    pub baseline_zpd: Option<String>,
    pub rings: Vec<BlueprintRing>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BlueprintRing {
    pub ring: u32,
    pub slug: String,
    pub goal: String,
    /// `done | active | fog`
    pub status: String,
    /// Empty for fog-of-war rings.
    pub slices: Vec<String>,
}

// ── tracks/<track>/track.yaml + slices/NN-slug/slice.yaml ──────────────────

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TrackDef {
    pub track: String,
    pub title: String,
    pub capstone: String,
    pub capstone_goal: String,
    pub rings: Vec<RingDef>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RingDef {
    pub ring: u32,
    pub slug: String,
    pub goal: String,
    #[serde(default)]
    pub slices: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Slice {
    pub id: String,
    pub ring: u32,
    pub goal: String,
    pub target_files: Vec<String>,
    pub probe_cmd: String,
    #[serde(default)]
    pub static_checks: Vec<String>,
    pub hint: String,
    /// Tier 0 structured lesson: the shape, shown before any quiz.
    #[serde(default)]
    pub lesson: Option<String>,
    pub trigger_words: Vec<String>,
    pub say_this_not_that: Vec<Swap>,
    pub story_template: String,
    pub concepts: Vec<String>,
    #[serde(default)]
    pub glossary: Vec<GlossaryEntry>,
    /// Slices whose code this slice builds on; a parked prerequisite is resurfaced first.
    #[serde(default)]
    pub requires: Vec<String>,
    /// Idempotent line appends into existing files (e.g. `pub mod x;` in lib.rs).
    #[serde(default)]
    pub wire: Vec<Wire>,
    pub tier0_drill: Drill,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Swap {
    pub avoid: String,
    pub say: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct GlossaryEntry {
    pub term: String,
    pub definition: String,
    #[serde(default)]
    pub avoid: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Wire {
    pub file: String,
    pub line: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Drill {
    pub prompt: String,
    pub quiz: Quiz,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Quiz {
    pub question: String,
    pub options: Vec<String>,
    pub answer_index: usize,
    #[serde(default)]
    pub explanation: Option<String>,
}

// ── events.jsonl ───────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Evidence {
    pub ts: String,
    pub event: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slice_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub concept: Option<String>,
    /// `solid | shaky | wrong`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grade: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub misconception_shape: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interval_days: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due_at: Option<String>,
}

// ── symbiote.sock protocol (newline-delimited JSON) ────────────────────────

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMsg {
    Hello { role: String },
    /// From the zsh precmd hook or `symbiote run`. `cmd` is observed, never executed.
    CmdEnd {
        cmd: String,
        exit_code: i32,
        duration_ms: u64,
        #[serde(default)]
        pipestatus: Vec<i32>,
        #[serde(default)]
        output_head: Option<String>,
    },
    Input { text: String },
    QuizAnswer { quiz_id: String, choice: usize },
    RunProbe,
    StartTrack { track: String },
    Advance,
    /// Layer 2: pushed by the Neovim plugin shim when a buffer's LSP error count changes.
    LspDiag { path: String, errors: u32 },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)] // short-lived wire values; boxing buys nothing
pub enum UiMsg {
    Mode { llm_mode: String, offline: bool, label: String },
    SetFocus { text: String, busy: bool },
    SetTasks(TasksView),
    UpsertGlossary(GlossaryView),
    PushQuiz { quiz: Option<QuizView> },
    QuizResult { quiz_id: String, correct: bool, correct_index: usize, explanation: Option<String> },
    TrackPicker { tracks: Vec<TrackInfo>, allow_custom: bool, message: Option<String> },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct TasksView {
    pub capstone: String,
    pub ring: u32,
    pub ring_name: String,
    pub slice_id: String,
    pub goal: String,
    pub target_files: Vec<String>,
    pub probe_cmd: String,
    /// `red | green | running | verifying | unknown`
    pub probe_status: String,
    pub static_checks: Vec<CheckStatus>,
    pub steps: Vec<Step>,
    pub due_items: usize,
    pub open_misconceptions: usize,
    pub hint: Option<String>,
    pub stderr_pointer: Option<String>,
    pub lesson: Option<String>,
    pub anchor: Option<String>,
    pub notice: Option<String>,
    /// Neovim LSP errors per file, before save/test (Layer 2). Empty without Neovim.
    #[serde(default)]
    pub lsp_errors: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Step {
    pub text: String,
    pub done: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct GlossaryView {
    pub terms: Vec<GlossaryEntry>,
    pub trigger_words: Vec<String>,
    pub say_this_not_that: Vec<Swap>,
    pub story: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct QuizView {
    pub id: String,
    /// `drill | review | warmup`
    pub kind: String,
    pub prompt: Option<String>,
    pub question: String,
    /// Authored order. The widget shuffles at render time and answers with the authored index.
    pub options: Vec<String>,
    pub concept: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TrackInfo {
    pub id: String,
    pub title: String,
    pub capstone_goal: String,
    pub recommended: bool,
}
