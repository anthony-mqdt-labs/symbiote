# Symbiote

**Agentic human enhancement in a split-pane desktop app—pairing a full embedded terminal with a structured Collaborative Workspace UI, with deterministic drill-based graceful degradation when offline.**

Symbiote pairs a complete embedded terminal (running `zsh`, Neovim/any editor, and your real toolchain, modeled on `atlas`) with a **Collaborative Workspace** pane—not a scrolling chat log. Important context (glossaries, task/todo lists, quizzes/drills, trigger words) renders as persistent, interactive UI objects alongside a focused 1-turn response while Symbiote silently handles curriculum, observation, role-shifting, and spaced repetition.

---

## Core Objectives

1. **Speed to Real Skill**: Acquire applicable engineering capability via red→green vertical slices in your actual toolchain.
2. **Competence & Speech Translation**: Convey senior-level employability regardless of starting polish—via **Trigger Words**, **"Say This / Not That"** phrasing swaps, and auto-captured **2-Sentence Engineering Stories** pinned in the workspace.
3. **Engineering Wisdom**: Internalize invariants, module depth, failure modes, and architectural tradeoffs that compound across roles.
4. **Zero Friction / Zero Bookkeeping**:
   - **Tier 1**: No homework (only optional primary-source links), no passive lessons, no rote multiple-choice exercises.
   - Zero user-facing trackers (`STATE.md`/`TRACKER.md`/`LOG.md` are eliminated).
   - Zero choice paralysis (every session opens with **one** concrete command and **one** next step).

---

## Two-Tier Execution Model

| Tier | Role | Capabilities |
| :--- | :--- | :--- |
| **Tier 1: Agentic Enhancement** *(Primary)* | Connected to an LLM via `~/Repos/vibes/spark/` (or any OpenAI-compatible / CLI agent). | Live Collaborative Workspace (focused Q&A + pinned UI objects), JIT Course & Slice Compilation, adaptive 4-persona role-shifting, live speech/phrasing translation, dynamic `/mock` grilling grounded in your diff, environment `/retro`. |
| **Tier 0: Graceful Degradation** *(Offline / Fallback)* | Pure deterministic local runtime (**no chat interface**). | Full embedded terminal, `zsh` + `fsnotify` observation, deterministic probe execution, stall detection, static smell checks, implicit SRS, and **pre-compiled learn-by-doing drills, shuffled quizzes, and structured offline lessons** rendered in the Workspace UI. |

---

## Phased UI Object Rollout

Workspace UI objects are implemented sequentially—each brought to production quality before starting the next:
- **Phase 1 (Current Target)**: **Glossaries** (earned terms + Trigger Words + Say This/Not That), **Task/Todo Lists** (active slice steps, probe status, `due_items`), and **Quizzes / Drills** (render-time shuffled retrieval checks + Tier 0 learn-by-doing drills).
- **Phase 2+ (Deferred)**: Interactive **Architectural**, **Flow**, and **Execution** Diagrams; inline diff review cards.

---

## Lineage (Consolidated Experiments)

| Source | Retained | Eliminated |
| :--- | :--- | :--- |
| `~/Repos/skills` | Frontier `grilling`, earned `GLOSSARY.md`, tracer-bullet TDD, tight red→green debugging, deep modules, two-axis review, `/retro`, `/wait-what`, primary-source grounding. | User-facing multi-file ceremony; `/teach` gaps (no baseline calibration `#725`, disconnected glossary `#559`, quiz slot-A bias `#335`, missing exit criteria). |
| `teach--*` | ZPD pacing, storage vs. fluency strength, `_verify/` compile-checked snippets. | Passive HTML decks and unverified prose. |
| `cellar`, `goforge`, `workbench` | Terminal-first single-project focus, red acceptance gates, read-and-defend drills, `FEEDBACK.md` guardrails (see [WORKFLOW.md](file:///Users/main/Repos/vibes/crashcourse/symbiote/WORKFLOW.md)). | 7+ redundant tracking files; cold-interviewer ambush (`workbench` Session 1). |
| `sonny`, `bastion`, `artificer`, `keystone`, `bedrock` | Real-world domain rigor, Senior rubrics, structured JSONL observability. | Static lab progression without live observation. |
| `meta-engine`, `atlas` | Sandboxed deterministic verification, embedded PTY (`Bun.Terminal` + `ghostty-web`), non-blocking concept graph, misconception-shape logging. | Heavy graph ceremony ahead of verifier coverage (2.5% gradeability cliff). |

---

## Document Map (For Implementors)

- [ARCHITECTURE.md](file:///Users/main/Repos/vibes/crashcourse/symbiote/ARCHITECTURE.md) — Desktop split-pane topology, embedded PTY + `zsh`/`fsnotify` observer, Collaborative Workspace UI objects & mutations, `.symbiote/` schemas, Tier 1 (`spark`) vs. Tier 0 fallback, JIT Slice Compiler + `_verify/` contract.
- [WORKFLOW.md](file:///Users/main/Repos/vibes/crashcourse/symbiote/WORKFLOW.md) — Adaptive 4-persona state machine, Speech & Engineering Story Coach, 5 Capability Rings, Cold Return Protocol, and user escape hatches.
- [PROMPT.md](file:///Users/main/Repos/vibes/crashcourse/symbiote/PROMPT.md) — Drop-in `SYMBIOTE_PROMPT` for agentic collaborators (`AGENTS.md` / `CLAUDE.md`).
- [IMPLEMENT.md](file:///Users/main/Repos/vibes/crashcourse/symbiote/IMPLEMENT.md) — Kickoff prompt to initiate an implementor agent for Phase 1.

---

## Build & Run (Phase 1 implementation)

Stack: Rust workspace. `crates/core` = daemon + `.symbiote/` control plane + CLI (`symbiote`); `crates/app` = GPUI desktop app (`symbiote-app`) with an `alacritty_terminal` PTY pane.

```bash
xcodebuild -downloadComponent MetalToolchain   # once: GPUI compiles Metal shaders at build time
cargo build --workspace
./target/debug/symbiote-app ~/code/my-go-queue  # opens the split pane; spawns the daemon
```

| Command | Purpose |
| :--- | :--- |
| `symbiote daemon [--root DIR]` | Control plane (the app spawns it; exits 30s after the last UI disconnects) |
| `symbiote verify-track <go\|rust\|ts\|all> [--git]` | Runs every bundled slice through the red/green `_verify/` gate, then replays it as a learner |
| `symbiote run -- <cmd>` | Runs a command through a pipe and reports its output (stderr_head/err_hash) to the daemon |
| `symbiote tracks` | Lists bundled tracks |

Environment: `SYMBIOTE_LLM_URL` (default `http://127.0.0.1:8095/v1`, spark), `SYMBIOTE_LLM_MODEL` (default: first `/v1/models` entry), `SYMBIOTE_LLM_KEY` (optional; logged as set/unset only), `SYMBIOTE_FONT`, `SYMBIOTE_TRACKS`, `SYMBIOTE_LOG_DIR` (default `logs/<component>/<date>.jsonl`, 7-day retention).

Tests: `cargo test --workspace` (unit + `crates/core/tests/daemon_e2e.rs`, which drives the real binary: hook event → daemon → UI objects, plus Tier 1 → Tier 0 fallback against a fake OpenAI server). Content check: `symbiote verify-track all --git`.

Invariants as implemented: the zsh hooks load through a `ZDOTDIR` redirect (`.symbiote/shell/`) that sources your dotfiles without writing to them. Slice gates run in a detached `git worktree`, or in a temp copy when the repo has no commits. `.symbiote/` is mode 0700 and the socket is 0600. Messages on the socket never carry a command to execute; only `probe_cmd`/`static_checks` from the slice YAML run. Dependencies are pinned with `=` and no build script in the tree downloads anything.

Layer 2 (Neovim, optional): inside the Symbiote terminal, `nvim` is a shell function that adds `--cmd` to load `.symbiote/shell/symbiote.lua`. Your `init.lua` is never touched, and the function is skipped if you already define `nvim()`. The shim sends `{"type":"lsp_diag","path","errors"}` to `symbiote.sock` whenever a project buffer's LSP error count changes. The Task List shows the counts before you save or test. This is a push from inside Neovim rather than a client on the `$NVIM` RPC socket, which avoids writing a msgpack-RPC client.

Terminal: xterm mouse reporting (SGR 1006, plus legacy X10 up to column/row 223) for apps that request it. Hold Shift for local selection. IME composition (preedit drawn at the cursor; candidate window placed on the cursor cell).

Persona: after a fluent green (ring ≥ 1, ≤ 3 probe runs, no stall or cold return in the slice), Tier 1 asks one optional interview question instead of the Two-Axis Review. Your next free-form input is graded as `mock_grade`.
