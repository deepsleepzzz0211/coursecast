# 05: 命令层骨架 + 项目命令：project create/status 三壳同形

**What to build:** 在 cc-core 增加命令层骨架和统一结果信封；project create/status 先走命令层，CLI/MCP/app 返回相同的键与形状（包括修正 app 的 status.inputs：从 array 改为与 CLI/MCP 一致的 object）。三个壳只负责参数解析与输出。用户可观察：同一个项目在三个壳里查到的状态 JSON 同形，AI 与 UI 拿到同一个结果。

**Blocked by:** None (can start immediately)

**Status:** done

- [x] project create/status 在 CLI/MCP/app 返回相同键集与形状；app 的 status.inputs 与 CLI/MCP 一致。
- [x] 命令层拥有参数校验、默认值与结果信封；壳内不再各自组装这两条命令的 JSON。
- [x] 新增跨壳同构测试：同一 fixture 经 CLI/MCP/app 归一化后比较。
- [x] 现有 project 测试通过；除 inputs 形状修正外无行为变化。
- [x] 结果信封能表达 ok=false 的结构化错误，且三个壳的错误码一致。
