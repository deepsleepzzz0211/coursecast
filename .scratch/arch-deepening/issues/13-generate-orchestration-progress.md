# 13: 生成编排 + 进度 observer：四段跑在同一条命令路径上

**What to build:** 四段生成编排（合成语音 → 拼接音轨 → 处理人像 → 渲染成片）收进命令层，进度通过一个中性的 observer 暴露；app 只把 observer 事件转发成 Tauri event。任一段失败仍隔离、重试只重跑该段及之后，已完成段不重做。用户可观察：第 6 步的四段进度与失败重试行为不变，但编排不再由 Tauri 壳独占。

**Blocked by:** 06, 07, 08, 12

**Status:** done

- [x] 生成按四段顺序调用命令层 operation；app 从 observer 收到 start/progress/done/skipped/failed。
- [x] 某段失败时前序段的产物与记录完好；重试复用已完成段，只重跑失败段及其后继。
- [x] `use_matte=false` 走卡片兜底；未授权仍被授权门拦截。
- [x] 现有 generate 测试通过；段 id、结果信封与前端消费的形状不变。
- [x] observer 有 CLI/MCP 的 no-op adapter；cc-core 不出现 Tauri 类型。
