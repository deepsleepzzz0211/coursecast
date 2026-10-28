# 07: 人像抠像命令：person_matte/status/skip 三壳同形

**What to build:** person_matte（运行抠像）、person_status、person_matte_skip，以及 app 的提前抠像预览，都走同一条命令路径；CLI/MCP/app 返回相同的键与错误码；01 提供的 fake 引擎 seam 在三个壳都可用。用户可观察：第 5 步预览过的抠像在生成时被跳过，不会重复推理。

**Blocked by:** 01, 05, 06

**Status:** done

- [x] CLI/MCP/app 调用同一个抠像命令 operation；JSON 键与错误码一致。
- [x] 提前预览与生成时抠像共用同一份指纹记录；生成时已预览则跳过。
- [x] 跳过抠像会清掉 alpha/记录，渲染走卡片兜底；换视频仍由 06 触发失效。
- [x] fake 引擎路径在三个壳可测；现有抠像 stage/inference 测试通过。
- [x] 壳内不再各自组装 person 域的 JSON 或解释抠像状态。

## Comments

2026-10-26 验收：person_matte run 三壳同源；status/skip 已进命令层并在 CLI/app 暴露。
2026-10-26 AI 能力边界补齐：MCP 新增 person_status / person_matte_skip 工具，三壳同形。
