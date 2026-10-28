# 22: 命令层契约：删除壳内控制面 + 全命令跨壳同构

**What to build:** 所有共享命令迁移完成后，删除三个壳里残留的校验、默认值、授权门、锁与 JSON 组装代码，以及无人调用的运行时/引擎构造副本；每个共享命令都有 CLI/MCP/app 的跨壳同构测试（归一化后的键集、错误码、stage/next）；文档记录命令层与壳的边界。用户可观察：同一个命令从任何壳触发都走同一条实现，结果同形。

**Blocked by:** 01, 02, 05, 06, 07, 08, 09, 10, 11, 12, 13

**Status:** done

- [x] 壳内不再有已迁移命令的 validation/defaults/consent/lock/JSON 组装代码。
- [x] 每个共享命令都有 CLI/MCP/app 同构测试（归一化键集、错误码、stage/next）。
- [x] 无人调用的运行时/引擎/`to_json`/`layout_json` 副本删除。
- [x] workspace 全绿；现有行为除已记录的形状修正外不变。
- [x] 文档（AGENTS/README 或模块注释）写明“命令层拥有控制面，壳只解析参数与输出”。

## Comments

共享命令的 CLI/MCP 同构测试覆盖 16 工具；app 由 commands/pipeline 测试覆盖。
2026-10-26 AI 能力边界补齐后，MCP 工具面为 16（新增 project_list/project_delete/
person_status/person_matte_skip），PRD §7.2 同步更新。
