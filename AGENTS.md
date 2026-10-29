# 课影 CourseCast — AGENTS.md

内部产品/验证文档保存在本地 `docs/`（不随仓库分发）。

## Command layer

Shared commands live in `cc-core::command` (project, deck/script, voice, person, render,
generate). A command owns validation/defaults, the consent gate, the project lock,
ordering, and the canonical `{ok,...}` / `{ok:false,error{code,message}}` envelope.
`cc-cli`, `cc-mcp`, and `cc-app` only parse arguments and print/serialize; they must not
add a second implementation of a shared command. Stage runners assume the caller holds
the project lock and carry the `*_locked` suffix; where a convenience wrapper also exists
(`slide_render`, `voice_synthesize`), the wrapper acquires the lock and delegates to the
suffixed runner. `generate` holds one lock across all four segments and uses the segment
commands' `*_locked` forms. Project list/delete are shared project commands;
`project_delete` requires explicit confirmation and must never be called speculatively.
The MCP face exposes all 16 tools in PRD §7.2, including project list/delete and person
status/skip.

`script_get` is a read, not an error: all shells return
`{ok:true, exists, pages:[string]}` (missing script = `exists:false`, empty pages).
The CLI/MCP default TTS preference is the offline `fake` engine for deterministic AI
tooling; the app persists its choice in `project.json` `tts` (`mimo` clone or `edge`
preset + `edge_voice`) and builds generate/preview chains from that preference via
`command::voice::chain_for_project`. Shells parse engine names into
`cc_core::TtsEngineKind` instead of comparing raw strings. Layout is persisted as free
`x_pct`/`y_pct` (0–100 of remaining canvas space) + `width_pct` 5–40; legacy `position`
presets remain readable and render unchanged. The browser mock
(`web/src/mockAdapter.ts`) is a dev-only, stateless fixture replay, not authoritative.

## Agent skills

### Issue tracker

Issues 以本地 markdown 文件形式存放在 `.scratch/<feature>/`（内部工作区，不随仓库分发）。见本地 `docs/agents/issue-tracker.md`（内部文档，不随仓库分发）。

### Triage labels

默认五角色词汇，标签串与角色同名（needs-triage / needs-info / ready-for-agent / ready-for-human / wontfix）。见本地 `docs/agents/triage-labels.md`（内部文档，不随仓库分发）。

### Domain docs

单上下文（single-context）：仓库根一份 `GLOSSARY.md` + 本地 `docs/adr/`（内部文档，不随仓库分发）。见本地 `docs/agents/domain.md`。
