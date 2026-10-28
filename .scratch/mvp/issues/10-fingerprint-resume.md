# 10: 指纹失效传播 + 断点续跑

**What to build:** 编排层行为：输入变更后自动算出**最小重跑集**并执行——改第 7 页讲稿 → 仅第 7 页 tts → 整轨 → 成片失效，matte 不动；换参考音频 → 全部页失效；改版式 → 仅成片失效。进程被 kill -9 后重进项目，已完成阶段（指纹匹配）不重跑。这是"单步可重跑"承诺的验收票。

**Blocked by:** 04, 08, 09

**Status:** done

依据：spec「Testing Decisions」必测场景；验证报告 §5 失效传播图。

- [x] 改第 7 页文本 → voice_synthesize 返回 regenerated=[7]、其余 skipped；render 需重跑；matte 产物指纹未变
- [x] 换参考音频 → 全部页音频失效 → 整轨 → 成片
- [x] 改 layout.toml → 仅成片失效
- [x] tts 中途 kill -9 → 重进项目 → 已完成页 skipped，未完成页续跑
- [x] 失败阶段重试：单阶段重试命令（不从全片重跑）
