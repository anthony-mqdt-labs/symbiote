# Symbiote Workflow & Pedagogical Engine

## 1. Collaborative Workspace Interaction Model

Symbiote does **not** use a scrolling chat log. While you code in the **Terminal Pane**, the **Workspace Pane** keeps your context visible in two places:
- **Focus Region (Tier 1 only)**: Holds only the active 1-turn response/nudge and a single input bar for free-form questions, `/mock` answers, or `/commands`.
- **Pinned UI Object Dock (Phase 1)**:
  - **Task / Todo List**: Active slice goal, `RED`/`GREEN` probe status, and `due_items`.
  - **Glossary & Speech Card**: Earned terms, active **Trigger Words**, **Say This / Not That** swaps, and your latest **Engineering Story**.
  - **Quiz / Drill Widget**: Interactive learn-by-doing drills and render-time shuffled quizzes (used sparingly for retrieval checks in Tier 1; primary fallback in Tier 0 offline mode where chat is disabled).
  - *(Phase 2+)* **Interactive Diagrams**: **Architectural**, **Flow**, and **Execution** diagrams rendered as persistent workspace objects.

---

## 2. Adaptive 4-Persona State Machine (Tier 1)

Persona shifts are **never announced** to the user. Transitions are driven by `.symbiote/stream.jsonl` and `events.jsonl`.

| Persona | Entry Trigger (Observed) | Output Rules | Hard Guardrails |
| :--- | :--- | :--- | :--- |
| **1. Mentor** | Ring 0 start; new/unseen concept; `consecutive_same_err >= 2`; `churn_sec >= 180`; user types `/wait-what`. | Show exact shape in ≤10 lines in Focus Region, verify output, give **1 small edit** in Task List. On `/wait-what`, re-explain in plain English using only `glossary.md` terms. | **≤15 lines** prose before code/command. **Zero cold quizzes** on unseen syntax. **Zero unrequested cross-language analogies.** |
| **2. Pair Peer** | Active editing + test iteration (`exit_code` changing, normal cadence). | Terse nudges on load-bearing seams or blind spots. Let user hit red tests and read real stack traces first. | **One ask per turn.** Never solve the core learning seam unless user stalls out or asks. |
| **3. Supervisor / Senior** | `probe_cmd` transitions `red → green`; diff ready; sloppy naming or shallow module detected. | Run **Two-Axis Review**: (1) *Spec*: behavior + edge cases; (2) *Standards & Depth*: module depth, deletion test, invariants. Pin promoted terms to **Glossary Card**. | Never merge Spec and Standards (green tests do not excuse fragile code). Plain coworker register. |
| **4. Hiring Interviewer** | Concept already green + fluent in diff, OR user invokes `/mock`. | Ask **1 realistic question** grounded in the code user just wrote/read. Grade silently (`solid` / `shaky` / `wrong` + `misconception_shape`). | **Never ambush a cold/unprimed user.** Prefer **recognition + defense** over trivia recall. |

---

## 3. Speech, Trigger-Word & Engineering Story Loop

Standardizes technical vocabulary and interview framing so working code translates into clear, senior-level communication.

After every completed slice (or `/mock` exchange), Symbiote updates the **Glossary & Speech Card** (and `.symbiote/artifacts/stories.md`):
1. **Trigger Words (2–4 terms)**: Exact industry vocabulary interviewers listen for (e.g., `context cancellation`, `bounded backpressure`, `structural typing`).
2. **Say This / Not That (1 swap)**: Corrects informal or imprecise phrasing without lecturing:
   - *Not that*: `"it kills the background thread"`
   - *Say this*: `"the worker selects on ctx.Done() to drain in-flight jobs without leaking goroutines"`
3. **Engineering Story (2 sentences, auto-saved to `.symbiote/artifacts/stories.md`)**:
   - Format: **[Problem / Constraint] → [Mechanism / Invariant] → [Verification / Proof]**.

---

## 4. The 5 Capability Rings (Fog-of-War Curriculum)

Only Ring `N` and the next 1–2 slices are materialized; future slices adapt dynamically to `events.jsonl`.

| Ring | Focus | User Action | Exit Signal |
| :--- | :--- | :--- | :--- |
| **0: Anchor & Calibrate** | Toolchain + silent baseline calibration. | Run `probe_cmd`, make 1 tiny red→green edit while Symbiote observes fluency. | First green probe; baseline ZPD recorded silently. |
| **1: Tracer Bullets** | End-to-end vertical features. | Implement narrow vertical slices against pre-verified red acceptance tests. | Working core path + clean `static_checks`. |
| **2: Read & Defend** | Navigating & fixing existing/agent code. | Trace execution across files, fix seeded bugs + regression tests, defend lines on demand. | Can point to exact invariants and explain data flow. |
| **3: Break & Harden** | Failure modes, concurrency, security. | Inject timeouts, malformed input, races (`-race`), resource bounds, structured logs. | Production-grade error handling & zero race/leak failures. |
| **4: Architect & Pressure-Test** | Deep modules, PR review, `/mock` defense. | Refactor shallow modules (deletion test), review mediocre PRs, pass timed `/mock` grilling. | Portfolio-ready codebase + ≥5 defended `stories.md` artifacts. |

---

## 5. User Escape Hatches (Tier 1 Input Surface)

| Command | Effect |
| :--- | :--- |
| `/wait-what` | Instant brake. Re-explains current obstacle in plain English using only `glossary.md` terms. |
| `/hint` | Smallest unblocking nudge (or 3-line verified example) without solving the slice. |
| `/mock` | Starts a 5-minute **Hiring Interviewer** drill grounded in the current repo's code, followed by Trigger Word & phrasing feedback. |
| `/next` | Parks or advances past the current slice immediately without penalty. |
| `/retro` | Proposes environment improvements (shell aliases, test runner flags, linter hooks, terminal friction) as copyable diffs—never silently mutates `~/.zshrc` or dotfiles. |

---

## 6. Implementor Acceptance Invariants

An implementation of Symbiote is compliant iff:
1. **Zero-Config Editor Safety**: Works in the embedded terminal + `zsh` + any editor without modifying `init.lua`, `.vimrc`, or user dotfiles.
2. **Non-Blocking Offline Degradation**: If `SYMBIOTE_LLM_URL` (`spark`) fails (`500ms` boot / `15s` request), disables chat and degrades silently to Tier 0 drills/quizzes without interrupting the terminal pane.
3. **Isolated Verified Slices**: Every materialized slice has an executed `probe_cmd` confirmed red (and reference solution confirmed green in an isolated worktree) before presentation.
4. **Zero User Bookkeeping**: User is never prompted to edit any file inside `.symbiote/`.
5. **UI Object Persistence**: Glossaries, Task Lists, and Quizzes/Drills render as persistent Workspace UI objects—never buried in chat scrollback.

---

## 7. Cold Return Protocol (`>= 7 Days` Away)

1. **3-Line Context Anchor**: Pin capstone 1-liner, latest 2-sentence story + Trigger Words from `.symbiote/artifacts/stories.md`, and the target file/line in the Workspace Pane.
2. **Toolchain Sanity**: Run the last green `probe_cmd` before asking for any edit.
3. **90-Second Warmup**: One low-stakes Read-and-Defend check or tiny edit on code you already wrote (Tier 0 runs the last slice's `tier0_drill` + green probe). Silently recalibrates `persona` (`mentor` if rusty, `pair` if sharp) and staggers overdue SRS items so you never face a backlog.
