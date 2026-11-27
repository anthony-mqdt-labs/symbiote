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
