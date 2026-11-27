# Symbiote Architecture

## 1. Runtime Topology (Split-Pane Desktop App)

Modeled on `atlas`'s embedded terminal architecture, Symbiote is a two-pane desktop application:

```
┌─────────────────────────────────┬─────────────────────────────────────────┐
│ TERMINAL PANE (60%)             │ COLLABORATIVE WORKSPACE PANE (40%)      │
│                                 │                                         │
│ • Embedded PTY (Bun.Terminal    │ ┌─ FOCUS REGION (Tier 1 Only) ────────┐ │
│   + ghostty-web WASM)           │ │ Current 1-turn response / nudge     │ │
│ • zsh (preexec/precmd) + Neovim │ │ Input: free-form Q&A or /commands   │ │
│   (or any terminal editor)      │ └─────────────────────────────────────┘ │
│ • Real toolchain                │ ┌─ PINNED UI OBJECTS (Phase 1) ───────┐ │
│   (go, cargo, tsc, pytest)      │ │ • Task/Todo List (slice + probe)    │ │
│                                 │ │ • Glossary & Trigger Words Card     │ │
│                                 │ │ • Interactive Quiz / Drill Widget   │ │
│                                 │ └─────────────────────────────────────┘ │
└──────────────┬──────────────────┴────────────────────▲────────────────────┘
               │                                       │ Local RPC /
               │ emits telemetry                       │ .symbiote/symbiote.sock
               ▼                                       ▼
┌───────────────────────────────────────────────────────────────────────────┐
│ SYMBIOTE DAEMON (Single Control Plane)                                    │
│ • fsnotify watcher (file saves + git diff)                                │
│ • zsh hook consumer (cmd, exit_code, duration_ms) + isolated probe runner │
│ • Stall detector (consecutive_same_err >= 2 OR churn_sec >= 180)          │
│ • Deterministic Tier 0 drill/quiz/static-check engine                     │
│ • Optional LLM Client via spark / OpenAI-compat HTTP (Tier 1)             │
└───────────────────────────────────────────────────────────────────────────┘
```

### Desktop App & Control Plane Contract
- **Embedded Terminal Pane**: Hosts a real PTY (`Bun.Terminal` + `ghostty-web` WASM, or equivalent) running `zsh` in `$SYMBIOTE_ROOT` with `SYMBIOTE_ACTIVE=1`. **Grading and `_verify/` never run through the PTY**—they run isolated in the daemon.
- **Collaborative Workspace Pane (Not a Chat Scroll)**:
  - **Focus Region** *(Tier 1 only)*: Shows only the active 1-turn AI response and a single input bar (accepting free-form questions, `/mock` answers, or `/wait-what`, `/hint`, `/mock`, `/next`, `/retro`). In **Tier 0**, the chat input is hidden/disabled (`[offline: drill mode]`).
  - **Pinned UI Object Dock** *(Phase 1 — complete before Phase 2)*:
    1. **Task / Todo List**: Active slice goal, `RED`/`GREEN` probe status, `static_checks`, and `due_items`.
    2. **Glossary & Speech Card**: Live-filtered `.symbiote/glossary.md` terms for the active slice, **Trigger Words**, **Say This / Not That** swaps, and latest **Engineering Story**.
    3. **Quiz / Drill Widget**: Interactive learn-by-doing drills and retrieval checks. **Must shuffle answer options at render time** (eliminating LLM slot-A bias from `teach` `#335`).
    - *(Phase 2+ deferred: interactive **Architectural**, **Flow**, and **Execution** diagrams; inline diff cards.)*
- **Structured UI Mutations**: The `symbiote` daemon owns all writes to `.symbiote/*` and pushes typed UI state updates (`set_focus`, `set_tasks`, `upsert_glossary`, `push_quiz`) over `symbiote.sock` / desktop RPC so important items stay pinned in context rather than scrolling away.

### Observation Layers
1. **Layer 1 (Default, Zero-Plugin)**: `zsh` `preexec`/`precmd` hook + `fsnotify` watcher. Zero Neovim config required.
   - `zsh` hook activates only when `SYMBIOTE_ACTIVE=1` (resolving `.symbiote/` via `$SYMBIOTE_ROOT` or parent walk); records `cmd`, `$?` (`$pipestatus`), and `duration_ms` via `$EPOCHREALTIME`. **Never** globally redirect `fd 2` (`stderr`) in `preexec`, as that breaks interactive TUIs (`nvim`, `fzf`, `less`).
   - `stderr_head` (`≤2 KiB`) is captured by having the daemon execute `probe_cmd` (or `symbiote run <cmd>`) via a pipe/PTY.
   - `fsnotify` watcher ignores `.git/` and `.symbiote/`, debounces saves (`250ms`), emits `file_saved`, and updates `churn_sec` (seconds since last `probe_cmd` run).
   - **`err_hash` Normalization & Stall Detection**: Strip ANSI escapes, timestamps/durations, hex addresses, and temp paths; collapse whitespace; take the first 8 hex chars of SHA-256 over the first 10 lines. Emit `stall_detected` when `consecutive_same_err >= 2` OR `churn_sec >= 180`.
2. **Layer 2 (Optional Enhancement)**: Neovim `--listen` RPC via `$NVIM` socket streaming `lsp_diag` (`{"ts","event":"lsp_diag","path","errors":N}`) prior to save/test.

---

## 2. Silent Engine Directory (`.symbiote/`)

User never edits or reads `.symbiote/` directly during normal sessions.

| File | Format | Purpose |
| :--- | :--- | :--- |
| `state.json` | JSON | Active ring, slice ID, persona, probe status, stall counters, parked slices, LLM mode. |
| `stream.jsonl` | JSONL | Ephemeral raw telemetry (`file_saved`, `cmd_end`, `lsp_diag`, `stall_detected`). Rotates via rename at 5 MiB (daemon-serialized appends). |
| `events.jsonl` | JSONL | Append-only learning evidence (`slice_pass`, `slice_parked`, `mock_grade`, `drill_pass`). Drives SRS & exit criteria. |
| `blueprint.json` | JSON | 5-Ring Capstone Blueprint (generated JIT on track start; future rings remain fog-of-war). |
| `slices/NN-slug.yaml` | YAML | Materialized slice contracts (including Tier 0 drills/quizzes). |
| `glossary.md` | Markdown | Earned ubiquitous language (`**Term**: Definition` + `_Avoid_: aliases`). |
| `artifacts/stories.md` | Markdown | Auto-captured 2-sentence Engineering Stories + Trigger Words per completed slice. |

### Minimal Schemas

**`state.json`**
```json
{
  "track": "go",
  "capstone": "durable-job-queue",
  "ring": 1,
  "slice_id": "02-worker-shutdown",
  "persona": "pair",
  "baseline_zpd": "familiar-syntax-new-concurrency",
  "probe_cmd": "go test -race -run TestShutdown ./...",
  "probe_status": "red",
  "last_err_hash": "a9f3c102",
  "last_probe_ts": "2026-11-26T20:44:15Z",
  "consecutive_same_err": 0,
  "churn_sec": 42,
  "active_misconception": null,
  "parked_slices": [],
  "llm_mode": "tier1_spark"
}
```
*(Valid `persona`: `mentor | pair | supervisor | interviewer`. Valid `llm_mode`: `tier1_spark | tier0_offline`.)*

**`blueprint.json`**
```json
{
  "track": "go",
  "capstone": "durable-job-queue",
  "baseline_zpd": "familiar-syntax-new-concurrency",
  "rings": [
    {"ring": 0, "slug": "anchor", "goal": "Verify toolchain & first red→green edit", "status": "done", "slices": ["00-anchor"]},
    {"ring": 1, "slug": "tracer-bullets", "goal": "Vertical queue + HTTP worker slices", "status": "active", "slices": ["01-enqueue", "02-worker-shutdown"]}
  ]
}
```

**`slices/NN-slug.yaml`** (Every slice MUST define a deterministic `probe_cmd` that fails before the slice starts)
```yaml
id: 02-worker-shutdown
ring: 1
goal: "Wire ctx.Done() into worker select loop for race-free shutdown"
target_files: [internal/queue/worker.go, internal/queue/worker_test.go]
probe_cmd: "go test -race -run TestShutdown ./..."
static_checks: ["go vet ./..."]
hint: "Select on <-ctx.Done() alongside your job channel and return after draining."
trigger_words: ["context cancellation", "goroutine leak", "graceful drain"]
say_this_not_that:
  - avoid: "kill the background thread"
    say: "propagate ctx.Done() to drain in-flight jobs"
story_template: "In my Go job queue, I prevented goroutine leaks on shutdown by wiring ctx.Done() into the worker select loop. Verified zero races under go test -race."
concepts: [context-cancellation, select-statement]
tier0_drill:
  prompt: "Break TestShutdown by removing ctx.Done(), observe the race/timeout, then restore it."
  quiz:
    question: "Why does closing a channel or cancelling ctx unblock select immediately?"
    options: ["Closed channels and ctx.Done() always return zero-value immediately", "The runtime kills the goroutine", "Mutex unlocks automatically"]
    answer_index: 0   # Shuffled at render time by Quiz Widget
```

**`stream.jsonl` & `events.jsonl` lines**
```json
{"ts":"2026-11-26T20:44:10Z","event":"file_saved","path":"internal/queue/worker.go","churn_sec":42}
{"ts":"2026-11-26T20:44:15Z","event":"cmd_end","cmd":"go test -race ./...","exit_code":1,"err_hash":"a9f3c102","stderr_head":"worker.go:34: undefined: ctx","duration_ms":310}
{"ts":"2026-11-26T20:44:28Z","event":"stall_detected","reason":"consecutive_same_err","count":2,"err_hash":"a9f3c102"}
```
```json
{"ts":"2026-11-26T20:45:00Z","event":"mock_grade","slice_id":"02-worker-shutdown","concept":"context-cancellation","grade":"shaky","misconception_shape":"confused buffered channel close with ctx cancellation","interval_days":1,"due_at":"2026-11-27T20:45:00Z"}
{"ts":"2026-11-26T20:52:00Z","event":"slice_pass","slice_id":"02-worker-shutdown","concept":"context-cancellation","grade":"solid","interval_days":3,"due_at":"2026-11-29T20:52:00Z"}
```

### Implicit SRS, Exit Criteria & Cold Return
- Group `events.jsonl` by `concept` using the **latest** event per concept:
  - **Intervals**: `1 → 3 → 7 → 14 → 30` days on `slice_pass`, `drill_pass`, or `mock_grade: "solid"`. Reset to `1` day on `shaky` or `wrong`.
  - **`due_items`**: Count of distinct concepts where `latest.due_at <= now`.
  - **`open_misconceptions`**: Count of distinct concepts where `latest.grade` is `"shaky"` or `"wrong"`. **Cleared** when a subsequent `mock_grade: "solid"`, `drill_pass`, or `slice_pass` is recorded for that `concept`.
- **Exit Criteria (Stop New Material)**: When `due_items >= 3` OR `open_misconceptions >= 2`, Symbiote pauses advancing to a new Ring and surfaces the oldest due/open concept as the next single review, drill, or `/mock` action.
- **Cold Return Protocol (`now - last_probe_ts >= 7 days`)**:
  1. Silently runs the last green `probe_cmd` to verify the toolchain still passes.
  2. Pins a 3-line Context Anchor (capstone goal + latest `artifacts/stories.md` entry + target file/line) in the Workspace Pane.
  3. Runs 1 low-stakes warmup on previously completed code to recalibrate `persona` (`mentor` if rusty, `pair` if fluent).
  4. Caps re-entry review to **1–2** high-leverage due concepts and staggers remaining `due_at` timestamps over the next 3–7 days.

---

## 3. Tier 1 (Agentic) vs. Tier 0 (Graceful Degradation)

| Subsystem | Tier 1: Agentic (`spark` / OpenAI-compat / Agent CLI) | Tier 0: Graceful Degradation (No Chat, Deterministic Drills) |
| :--- | :--- | :--- |
| **Workspace Pane** | Focus Region (1-turn Q&A + `/commands`) + live UI Objects (Tasks, Glossary, Quiz/Drill). | **No chat input** (`[offline: drill mode]`). Renders pre-authored **learn-by-doing drills, shuffled quizzes, and structured lessons** in the UI Object Dock. |
| **Track Start (No Course)** | **JIT Course Compiler**: 1-question frontier grill → generates `blueprint.json` → materializes & `_verify/`-runs Slice 01. | Loads pre-bundled track (`./tracks/<track>/` or `crashcourse`). If none exists, fail fast: `"No pre-built track for '<track>' and LLM is offline. Start spark or set SYMBIOTE_LLM_URL."` |
| **Stall Response** | **Mentor**: Inspects exact diff + stderr, shows ≤10-line working pattern, runs it, gives 1 next edit. | Displays slice `hint`, exact stderr line pointer, and matching `glossary.md` card. |
| **Green Test Review** | **Supervisor / Senior**: Two-axis review (Spec + Standards/Depth) on actual `git diff`, promotes terms to `glossary.md`. | Runs `static_checks` (`go vet`, race detector, regex/AST smell rules) and unlocks the slice's `tier0_drill` / shuffled quiz. |
| **Speech & Story Coach** | Translates user's rough spoken/typed explanation into senior phrasing; pins updated 2-sentence story to Glossary Card & `stories.md`. | Pins slice's `trigger_words`, `say_this_not_that`, and `story_template` to Glossary Card & `stories.md`. |

### `spark` Integration & Mid-Session Fallback Contract
- Configured via `SYMBIOTE_LLM_URL` (defaulting to local `spark` OpenAI-compatible endpoint) and `SYMBIOTE_LLM_MODEL`.
- **Boot & Mid-Session Resilience**: `GET /v1/models` (500ms timeout) on boot; per-request timeout `15s`. On timeout or ≥2 consecutive 5xx/transport errors mid-session, the daemon flips `state.json` `llm_mode` to `tier0_offline`, disables the chat input bar, aborts any in-flight JIT compilation without touching the workspace, displays `[offline: drill mode]`, and probes `/v1/models` every `60s` for automatic recovery.

---

## 4. JIT Slice Compiler & `_verify/` Gate

When generating a slice on the fly (Tier 1):
1. Author `slices/NN-slug.yaml` (including `tier0_drill` fallback so the slice works offline later), target stub, and failing test (`_test.go` / `tests/`).
2. **Collision & Isolated Verification**: Check for file collisions against the user's workspace; spin up an isolated `git worktree add --detach /tmp/symbiote-verify-<id>` (copying any dirty/untracked workspace files) so the user's live working tree is **never** mutated during verification.
3. **Red Gate**: Execute `probe_cmd` in the isolated worktree and assert **non-zero exit** from test execution/assertion or expected stub boundary (never from broken test-harness syntax).
4. **Green Gate**: Apply the reference solution in the isolated worktree and assert `probe_cmd` and `static_checks` exit **zero**.
5. **Cleanup & Commit to Workspace**: Remove `/tmp/symbiote-verify-<id>` (`git worktree remove --force`) and write only the verified **red** stub + test + `slices/NN-slug.yaml` to the user's workspace.
