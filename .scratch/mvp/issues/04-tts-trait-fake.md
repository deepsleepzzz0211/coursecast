# 04: TTS 引擎 trait + 假引擎 + 按页合成

**What to build:** `voice_synthesize` 阶段端到端（假引擎）：`voice_synthesize --project ID [--pages 1-5] --json` 按页合成 wav 落盘到 work/audio/，逐页指纹，幂等（重跑前查已落盘产物，已完成页跳过并进 `skipped` 列表），请求层带超时 + 重试框架。引擎 trait 是后续 MiMo/SAPI/edge 三张票的接缝，测试用确定性假引擎（canned wav），不打网络。

**Blocked by:** 01

**Status:** ready-for-agent

依据：spec「Testing Decisions」第 2 道缝；tts 指纹 = 该页文本 + 引擎 + 模型名 + 参考音频哈希 + 风格指令 + 语速参数。

- [ ] 假引擎确定性产出 24kHz mono wav，逐页落盘命名 slide_NNN.wav
- [ ] --pages 子集合成；JSON 返回 generated / skipped / artifacts / total_duration_sec / next
- [ ] 同参数重复调用 → 全部 skipped，不重复调用引擎
- [ ] 改某页文本 → 仅该页重新生成
- [ ] 引擎超时（fake 可注入慢响应）触发重试；重试前查询已落盘产物
- [ ] 失败页不污染其他页产物（失败不落盘、错误独立）
