# 11: MCP server（12 工具）

**What to build:** cc-mcp：rmcp（官方 Rust SDK）stdio transport，暴露 spec §7.2 的 12 个工具（project_create / project_status / slide_render / script_get / script_set / script_import_notes / voice_configure / voice_synthesize / person_matte / layout_set / render / artifact_clean），全部薄包装 cc-core，返回结构化 JSON（ok / stage / 产物路径 / `next` 建议）。权限边界：路径 canonicalize 后校验在项目根内、项目 ID 限定 `[a-z0-9]{8,32}`、artifact_clean 需显式 `confirm: true`。

**Blocked by:** 10

**Status:** done

依据：spec「Implementation Decisions」MCP 工具面与权限边界；PRD §7.4 配置示例（--data-dir 参数）。

- [x] AI 通过 MCP 协议能独立跑完全流程（脚本化 stdio 测试：initialize → tools/list → 逐工具调用）
- [x] 每个工具返回值与对应 CLI 子命令的 JSON 同构（同源 cc-core 的证明）
- [x] 路径逃逸 / 非法项目 ID / 无 confirm 的清理操作被拒绝
- [x] API Key 相关工具不回显密钥
