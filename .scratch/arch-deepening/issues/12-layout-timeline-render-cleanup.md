# 12: 版式 + 时间轴 + 渲染 + 清理 + 成片批次

**What to build:** layout_get/set、timeline、render、artifact_clean、final_video、download 收进命令层；render 的“先确保时间轴是最新的，再渲染”成为一条策略，不再由每个壳各自决定。用户可观察：从 CLI、MCP 或 app 触发渲染，得到相同的翻页点与成片；改版式只重渲成片。

**Blocked by:** 05

**Status:** done

- [x] CLI/MCP/app 的 layout/render/final_video 形状一致；render 在任何入口都先确保时间轴就绪。
- [x] 时间轴公式不变（页时长 = 本页音频 + 页间静音，末页不加；总时长同公式）；fixture 翻页点误差 ≤33 ms。
- [x] artifact_clean 仍需显式 confirm，且只清中间产物与阶段记录，不动课件与逐字稿记录。
- [x] 下载命名/防覆盖只有一份实现；app 行为不变。
- [x] 现有 timeline/render/layout/clean 测试通过；断点续跑与失效传播不变。
