# Symbiote Implementation Kickoff Prompt

Use this prompt to initiate an engineering agent to implement Symbiote from the specification documents.

```markdown
Read the Symbiote specification in `~/Repos/vibes/crashcourse/symbiote/` in full:
1. `README.md`
2. `ARCHITECTURE.md`
3. `WORKFLOW.md`
4. `PROMPT.md`

Reference codebases in `~/Repos/vibes/` (read only what you need for integration contracts):
- `atlas` — for its split-pane desktop shell and embedded PTY pattern (`Bun.Terminal` + `ghostty-web` WASM).
- `spark` — for the local OpenAI-compatible LLM proxy contract (`SYMBIOTE_LLM_URL`).

## Scope & Build Order
Implement **Phase 1 only** (defer Phase 2+ diagrams and diff cards):
1. **Daemon + `.symbiote/` Control Plane**: `symbiote.sock` IPC, `state.json` / `stream.jsonl` / `events.jsonl` / `blueprint.json` / `slices/NN-slug.yaml` / `glossary.md` / `artifacts/stories.md`, `zsh` `preexec`/`precmd` + `fsnotify` observer, `err_hash` stall detector, isolated `git worktree` `_verify/` gate, and `spark` 500ms/15s health + Tier 0 fallback.
2. **Split-Pane Desktop App**: Embedded PTY terminal pane (`zsh` + any editor) + Collaborative Workspace pane (Tier 1 Focus Region + Pinned UI Object Dock).
3. **Phase 1 UI Objects (complete each to production quality before moving on)**:
   - **Task / Todo List**
   - **Glossary & Speech Card** (earned terms, Trigger Words, Say This / Not That, 2-sentence Engineering Story)
   - **Quiz / Drill Widget** (render-time option shuffling + Tier 0 offline drills)

## How to Start
1. Before writing code, ask me only the unresolved architectural decisions you need settled (e.g., target repo path, daemon/UI stack choice, and any `spark` endpoint assumptions).
2. Propose a tracer-bullet vertical slice plan so we can verify the end-to-end loop (terminal event → daemon → UI object update) first.
3. Enforce every invariant in `WORKFLOW.md §6` (zero dotfile mutation, non-blocking Tier 0 degradation, isolated `_verify/`, zero user bookkeeping).
```
